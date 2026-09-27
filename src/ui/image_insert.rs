//! Puts images and videos into the presentation from the editor: the
//! Image button of the tool palette, files dropped on the canvas, a paste,
//! and the image and video fills of the inspector.
//!
//! An image or a video is a rectangle filled with it. Each way reads the
//! file off the UI thread (and converts a video that is not an MP4 with
//! H.264 and AAC), then makes one undo step that embeds it (unless the same
//! bytes are embedded already) and uses it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{ClipboardEntry, Context, PathPromptOptions, Window};

use crate::document::{
    Element, ElementId, ElementKind, Fill, Frame, ImageData, ImageFill, ImageFit, ImageId,
    Operation, RectangleElement, ShapeStylePatch, VideoData, VideoFill, VideoId,
};
use crate::editor::{EditorView, Tool};
use crate::images::ImageError;
use crate::ui::show_error;

/// Where a new image or video goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageTarget {
    /// A new rectangle centered on this slide point, or on the slide.
    Insert(Option<(f32, f32)>),
    /// The fill of the selected rectangles and ellipses.
    Fill,
    /// The `iChannel0` of the selected shader fills; images only.
    Channel,
}

/// Share of the slide a new image may cover at most.
const FIT_SHARE: f32 = 0.8;

/// Extensions of the files read as videos; other files are read as images.
const VIDEO_EXTENSIONS: [&str; 10] = [
    "mp4", "m4v", "mov", "webm", "mkv", "avi", "ogv", "mpg", "mpeg", "wmv",
];

/// Whether the file is read as a video, from its extension.
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// What a file gives: the bytes of an image, or a video ready to embed.
pub enum Media {
    Image(Vec<u8>),
    Video(VideoData),
}

impl EditorView {
    /// The id of the embedded image with these bytes, and the operation that
    /// embeds it when none has them yet.
    fn embed(&mut self, data: ImageData) -> (ImageId, Option<Operation>) {
        match self.presentation.images.find(&data.bytes) {
            Some(id) => (id, None),
            None => {
                let id = self.presentation.new_image_id();
                (id, Some(Operation::AddImage { id, data }))
            }
        }
    }

    /// Like [`Self::embed`] for a video.
    fn embed_video(&mut self, data: VideoData) -> (VideoId, Option<Operation>) {
        match self.presentation.videos.find(&data.bytes) {
            Some(id) => (id, None),
            None => {
                let id = self.presentation.new_video_id();
                (id, Some(Operation::AddVideo { id, data }))
            }
        }
    }

    /// A frame of `width` × `height` pixels, no larger than most of the
    /// slide, centered on `at` or on the slide.
    fn media_frame(&self, width: f32, height: f32, at: Option<(f32, f32)>) -> Frame {
        let slide = self.presentation.size;
        let (slide_width, slide_height) = (slide.width as f32, slide.height as f32);
        let scale = (slide_width * FIT_SHARE / width)
            .min(slide_height * FIT_SHARE / height)
            .min(1.);
        let (width, height) = (width * scale, height * scale);
        let (cx, cy) = at.unwrap_or((slide_width / 2., slide_height / 2.));
        Frame {
            x: cx - width / 2.,
            y: cy - height / 2.,
            width,
            height,
            rotation: 0.,
        }
    }

    /// Adds a rectangle with the fill in the frame on top of the current
    /// slide, after the operations that embed what it shows, as one undo
    /// step. Selects it.
    fn insert_filled(
        &mut self,
        label: &str,
        fill: Fill,
        embed: Option<Operation>,
        frame: Frame,
    ) -> ElementId {
        let id = self.presentation.new_element_id();
        let element = Element::new(
            id,
            frame,
            ElementKind::Rectangle(RectangleElement {
                fill,
                stroke: None,
                corner_radius: 0.,
            }),
        );
        let mut operations: Vec<Operation> = embed.into_iter().collect();
        operations.push(Operation::AddElement {
            slide: self.current_slide,
            parent: None,
            index: usize::MAX,
            element,
        });
        self.end_text_edit();
        self.commit(label, Operation::Batch(operations), vec![id]);
        self.active_tool = Tool::Move;
        id
    }

    /// Adds a rectangle filled with the video, at its size in pixels but no
    /// larger than most of the slide, as one undo step. Selects it.
    pub fn insert_video(&mut self, data: VideoData, at: Option<(f32, f32)>) -> ElementId {
        let frame = self.media_frame(data.width as f32, data.height as f32, at);
        let (video, add) = self.embed_video(data);
        self.insert_filled(
            "Insert video",
            Fill::Video(VideoFill::new(video)),
            add,
            frame,
        )
    }

    /// Fills the selected rectangles and ellipses with the video, keeping
    /// the settings of a video they already show, as one undo step.
    pub fn fill_with_video(&mut self, data: VideoData) {
        let Some(ids) = self.selected_shapes() else {
            return;
        };
        let (video, add) = self.embed_video(data);
        let mut operations: Vec<Operation> = add.into_iter().collect();
        for id in ids {
            let Some(fill) = self.presentation.element(id).and_then(|e| e.kind.fill()) else {
                continue;
            };
            let fill = match fill {
                Fill::Video(current) => VideoFill {
                    id: video,
                    ..*current
                },
                Fill::Image(image) => VideoFill {
                    fit: image.fit,
                    opacity: image.opacity,
                    ..VideoFill::new(video)
                },
                _ => VideoFill::new(video),
            };
            operations.push(Operation::SetShapeStyle {
                id,
                patch: ShapeStylePatch {
                    fill: Some(Fill::Video(fill)),
                    ..ShapeStylePatch::default()
                },
            });
        }
        let selection = self.selection.clone();
        self.commit(
            "Video fill",
            Operation::Batch(operations),
            selection.clone(),
        );
        self.selection = selection;
    }

    /// Adds a rectangle filled with the image, at its size in pixels but
    /// no larger than most of the slide, as one undo step. Selects it.
    pub fn insert_image_bytes(
        &mut self,
        bytes: Vec<u8>,
        at: Option<(f32, f32)>,
    ) -> Result<ElementId, ImageError> {
        let data = ImageData::read(Arc::from(bytes))?;
        let frame = self.media_frame(data.width as f32, data.height as f32, at);
        let (image, add) = self.embed(data);
        let fill = Fill::Image(ImageFill {
            id: image,
            fit: ImageFit::Cover,
            opacity: 1.,
        });
        Ok(self.insert_filled("Insert image", fill, add, frame))
    }

    /// Fills the selected rectangles and ellipses with the image, keeping
    /// the fit and opacity of an image they already show, as one undo step.
    pub fn fill_with_image_bytes(&mut self, bytes: Vec<u8>) -> Result<(), ImageError> {
        let data = ImageData::read(Arc::from(bytes))?;
        let Some(ids) = self.selected_shapes() else {
            return Ok(());
        };
        let (image, add) = self.embed(data);
        let mut operations: Vec<Operation> = add.into_iter().collect();
        for id in ids {
            let Some(fill) = self.presentation.element(id).and_then(|e| e.kind.fill()) else {
                continue;
            };
            let (fit, opacity) = match fill {
                Fill::Image(current) => (current.fit, current.opacity),
                _ => (ImageFit::Cover, 1.),
            };
            operations.push(Operation::SetShapeStyle {
                id,
                patch: ShapeStylePatch {
                    fill: Some(Fill::Image(ImageFill {
                        id: image,
                        fit,
                        opacity,
                    })),
                    ..ShapeStylePatch::default()
                },
            });
        }
        let selection = self.selection.clone();
        self.commit(
            "Image fill",
            Operation::Batch(operations),
            selection.clone(),
        );
        self.selection = selection;
        Ok(())
    }

    /// Changes how the image and video fills of the selected shapes fit
    /// their box.
    pub fn set_image_fit(&mut self, fit: ImageFit) {
        let label = match self.selected_fill_type() {
            Some(crate::ui::shape_inspector::FillType::Video) => "Video fit",
            _ => "Image fit",
        };
        self.edit_selected_shapes(label, |kind| {
            let fill = match kind.fill()? {
                Fill::Image(image) if image.fit != fit => Fill::Image(ImageFill { fit, ..*image }),
                Fill::Video(video) if video.fit != fit => Fill::Video(VideoFill { fit, ..*video }),
                _ => return None,
            };
            Some(ShapeStylePatch {
                fill: Some(fill),
                ..ShapeStylePatch::default()
            })
        });
    }

    fn use_media(&mut self, media: Media, target: ImageTarget) -> Result<(), String> {
        let image = |error: ImageError| error.to_string();
        match (media, target) {
            (Media::Image(bytes), ImageTarget::Insert(at)) => self
                .insert_image_bytes(bytes, at)
                .map(|_| ())
                .map_err(image),
            (Media::Image(bytes), ImageTarget::Fill) => {
                self.fill_with_image_bytes(bytes).map_err(image)
            }
            (Media::Image(bytes), ImageTarget::Channel) => {
                let data = ImageData::read(Arc::from(bytes)).map_err(image)?;
                let (id, add) = self.embed(data);
                self.set_shader_channel(Some(id), add);
                Ok(())
            }
            (Media::Video(data), ImageTarget::Insert(at)) => {
                self.insert_video(data, at);
                Ok(())
            }
            (Media::Video(data), ImageTarget::Fill) => {
                self.fill_with_video(data);
                Ok(())
            }
            (Media::Video(_), ImageTarget::Channel) => {
                Err("A shader channel takes an image, not a video".into())
            }
        }
    }

    /// Asks for an image or a video file, then uses it.
    pub fn choose_image(
        &mut self,
        target: ImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Insert".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.load_image_files(paths, target, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Reads image and video files off the UI thread, converts the videos
    /// that need it, then uses each file. Several inserted files step down
    /// and right, so that none hides another.
    pub fn load_image_files(
        &mut self,
        paths: Vec<PathBuf>,
        target: ImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (progress, done) = async_channel::unbounded::<f32>();
        let converting = paths.iter().any(|path| is_video(path));
        if converting {
            self.converting = Some(0.);
            cx.spawn(async move |this, cx| {
                while let Ok(share) = done.recv().await {
                    let update = this.update(cx, |this, cx| {
                        this.converting = Some(share);
                        cx.notify();
                    });
                    if update.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.spawn_in(window, async move |this, cx| {
            let files = gpui_kit::AppContext::background_spawn(cx, async move {
                paths
                    .into_iter()
                    .map(|path| {
                        let bytes = std::fs::read(&path)
                            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
                        if !is_video(&path) {
                            return Ok(Media::Image(bytes));
                        }
                        let report = |share: f32| {
                            progress.try_send(share).ok();
                        };
                        crate::videos::prepare(Arc::from(bytes), &report)
                            .map(|(data, _)| Media::Video(data))
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                if converting {
                    this.converting = None;
                }
                for (index, file) in files.into_iter().enumerate() {
                    let target = match target {
                        ImageTarget::Insert(Some((x, y))) => {
                            let step = 24. * index as f32;
                            ImageTarget::Insert(Some((x + step, y + step)))
                        }
                        target => target,
                    };
                    let result = file.and_then(|media| this.use_media(media, target));
                    if let Err(message) = result {
                        show_error(message, window, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Inserts the image or the image files on the clipboard. Returns false
    /// when it holds neither, so that the key can do something else.
    pub fn paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    if let Err(error) = self.insert_image_bytes(image.bytes.clone(), None) {
                        show_error(error.to_string(), window, cx);
                    }
                    return true;
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    let paths = paths.paths().to_vec();
                    self.load_image_files(paths, ImageTarget::Insert(None), window, cx);
                    return true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        false
    }
}
