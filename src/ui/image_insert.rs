//! Puts images into the presentation from the editor: the Image button of
//! the tool palette, files dropped on the canvas, a paste, and the image
//! fill of the inspector.
//!
//! An image is a rectangle filled with the image. Each way reads the file
//! off the UI thread, then makes one undo step that embeds the image (unless
//! the same bytes are embedded already) and uses it.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{ClipboardEntry, Context, PathPromptOptions, Window};

use crate::document::{
    Element, ElementId, ElementKind, Fill, ImageData, ImageFill, ImageFit, ImageId, Operation,
    RectangleElement, ShapeStylePatch,
};
use crate::editor::{EditorView, Tool};
use crate::images::ImageError;
use crate::ui::show_error;

/// Where a new image goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageTarget {
    /// A new rectangle centered on this slide point, or on the slide.
    Insert(Option<(f32, f32)>),
    /// The fill of the selected rectangles and ellipses.
    Fill,
}

/// Share of the slide a new image may cover at most.
const FIT_SHARE: f32 = 0.8;

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

    /// Adds a rectangle filled with the image, at its size in pixels but
    /// no larger than most of the slide, as one undo step. Selects it.
    pub fn insert_image_bytes(
        &mut self,
        bytes: Vec<u8>,
        at: Option<(f32, f32)>,
    ) -> Result<ElementId, ImageError> {
        let data = ImageData::read(Arc::from(bytes))?;
        let (width, height) = (data.width as f32, data.height as f32);
        let (image, add) = self.embed(data);
        let slide = self.presentation.size;
        let (slide_width, slide_height) = (slide.width as f32, slide.height as f32);
        let scale = (slide_width * FIT_SHARE / width)
            .min(slide_height * FIT_SHARE / height)
            .min(1.);
        let (width, height) = (width * scale, height * scale);
        let (cx, cy) = at.unwrap_or((slide_width / 2., slide_height / 2.));
        let frame = crate::document::Frame {
            x: cx - width / 2.,
            y: cy - height / 2.,
            width,
            height,
            rotation: 0.,
        };
        let id = self.presentation.new_element_id();
        let element = Element::new(
            id,
            frame,
            ElementKind::Rectangle(RectangleElement {
                fill: Fill::Image(ImageFill {
                    id: image,
                    fit: ImageFit::Cover,
                    opacity: 1.,
                }),
                stroke: None,
                corner_radius: 0.,
            }),
        );
        let mut operations: Vec<Operation> = add.into_iter().collect();
        operations.push(Operation::AddElement {
            slide: self.current_slide,
            parent: None,
            index: usize::MAX,
            element,
        });
        self.end_text_edit();
        self.commit("Insert image", Operation::Batch(operations), vec![id]);
        self.active_tool = Tool::Move;
        Ok(id)
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

    /// Changes how the image fills of the selected shapes fit their box.
    pub fn set_image_fit(&mut self, fit: ImageFit) {
        self.edit_selected_shapes("Image fit", |kind| match kind.fill()? {
            Fill::Image(image) if image.fit != fit => Some(ShapeStylePatch {
                fill: Some(Fill::Image(ImageFill { fit, ..*image })),
                ..ShapeStylePatch::default()
            }),
            _ => None,
        });
    }

    fn use_image_bytes(&mut self, bytes: Vec<u8>, target: ImageTarget) -> Result<(), ImageError> {
        match target {
            ImageTarget::Insert(at) => self.insert_image_bytes(bytes, at).map(|_| ()),
            ImageTarget::Fill => self.fill_with_image_bytes(bytes),
        }
    }

    /// Asks for an image file, then uses it.
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

    /// Reads image files off the UI thread, then uses each one. Several
    /// inserted images step down and right, so that none hides another.
    pub fn load_image_files(
        &mut self,
        paths: Vec<PathBuf>,
        target: ImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let files = gpui_kit::AppContext::background_spawn(cx, async move {
                paths
                    .into_iter()
                    .map(|path| std::fs::read(&path).map_err(|error| (path, error)))
                    .collect::<Vec<_>>()
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                for (index, file) in files.into_iter().enumerate() {
                    let target = match target {
                        ImageTarget::Insert(Some((x, y))) => {
                            let step = 24. * index as f32;
                            ImageTarget::Insert(Some((x + step, y + step)))
                        }
                        target => target,
                    };
                    let result = match file {
                        Ok(bytes) => this
                            .use_image_bytes(bytes, target)
                            .map_err(|error| error.to_string()),
                        Err((path, error)) => {
                            Err(format!("Cannot read {}: {error}", path.display()))
                        }
                    };
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
