//! The presentation: slides full screen, one after the other, with their
//! videos and shaders playing.
//!
//! A slide shows with its `auto` fills playing. Each click (or →, space or
//! Page Down) starts the next `on_click` fill of the slide, in layer order
//! from the bottom; when none is left, it goes to the next slide. A click
//! on a started fill pauses or plays it. ← and Page Up go back a slide;
//! Escape ends the presentation.
//!
//! The presentation shows a copy of the document made when it starts. It
//! draws each slide as the export renderer does: the elements that do not
//! play are rendered once per slide in the background, in layers; the
//! playing fills are rendered between those layers on each frame.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui_kit::{
    App, AppContext as _, Bounds, Context, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Render,
    RenderImage, Styled as _, Window, WindowBounds, WindowOptions, canvas, div, point, px, size,
};

use crate::document::{Element, ElementId, Fill, Presentation, SlideId, Start};
use crate::render::{self, Layer};
use crate::ui::playback::Playback;
use crate::ui::shape_paint::render_image;

/// What the presentation shows and plays, without the window.
pub struct Show {
    pub presentation: Presentation,
    /// Index of the slide on screen.
    pub index: usize,
    /// The `on_click` fills of the slide that did not start, in layer
    /// order.
    pub queue: VecDeque<ElementId>,
    pub playback: Playback,
}

/// What a click or a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Started the next `on_click` fill.
    Started(ElementId),
    /// Paused or played a started fill.
    Toggled(ElementId),
    /// Went to another slide.
    Slide(usize),
    /// Nothing: the last slide, or the first one going back.
    None,
}

impl Show {
    pub fn new(presentation: Presentation, index: usize, playback: Playback) -> Self {
        let mut show = Show {
            presentation,
            index: 0,
            queue: VecDeque::new(),
            playback,
        };
        show.enter(index);
        show
    }

    pub fn slide_id(&self) -> Option<SlideId> {
        self.presentation
            .slides
            .get(self.index)
            .map(|slide| slide.id)
    }

    /// The visible elements of the slide with a video or shader fill, in
    /// layer order.
    fn playing_elements(&self) -> Vec<(ElementId, Fill)> {
        let Some(slide) = self.presentation.slides.get(self.index) else {
            return Vec::new();
        };
        slide
            .visible_leaves()
            .filter_map(|node| {
                let fill = node.element.kind.fill()?;
                fill.is_animated().then(|| (node.element.id, fill.clone()))
            })
            .collect()
    }

    /// Shows the slide at `index`: stops the fills of the slide before and
    /// starts the `auto` ones.
    pub fn enter(&mut self, index: usize) {
        self.index = index.min(self.presentation.slides.len().saturating_sub(1));
        self.playback.clear();
        self.queue.clear();
        for (id, fill) in self.playing_elements() {
            match fill.playback() {
                Some((Start::Auto, _)) => {
                    self.playback.start(id, &fill, &self.presentation);
                }
                Some((Start::OnClick, _)) => self.queue.push_back(id),
                None => {}
            }
        }
    }

    /// Starts the next `on_click` fill, or goes to the next slide.
    pub fn advance(&mut self) -> Step {
        while let Some(id) = self.queue.pop_front() {
            let fill = self
                .presentation
                .element(id)
                .and_then(|element| element.kind.fill())
                .cloned();
            if let Some(fill) = fill
                && self.playback.start(id, &fill, &self.presentation)
            {
                return Step::Started(id);
            }
        }
        if self.index + 1 < self.presentation.slides.len() {
            self.enter(self.index + 1);
            Step::Slide(self.index)
        } else {
            Step::None
        }
    }

    pub fn back(&mut self) -> Step {
        if self.index == 0 {
            return Step::None;
        }
        self.enter(self.index - 1);
        Step::Slide(self.index)
    }

    /// A click at a point of the slide: pauses or plays the started fill
    /// under it, starts the `on_click` fill under it, or else advances.
    pub fn click(&mut self, x: f32, y: f32) -> Step {
        let under = self.playing_elements().into_iter().rev().find(|(id, _)| {
            self.presentation
                .element(*id)
                .is_some_and(|element| element.frame.contains(x, y))
        });
        match under {
            Some((id, _)) if self.playback.is_live(id) => {
                self.playback.toggle(id);
                Step::Toggled(id)
            }
            Some((id, fill)) if self.queue.contains(&id) => {
                self.queue.retain(|queued| *queued != id);
                self.playback.start(id, &fill, &self.presentation);
                Step::Started(id)
            }
            _ => self.advance(),
        }
    }
}

/// The still layers of a slide, rendered for one scale.
struct Layers {
    slide: SlideId,
    scale: f32,
    layers: Vec<ShownLayer>,
}

enum ShownLayer {
    Still(Arc<RenderImage>),
    Apart(ElementId),
}

/// A fill as painted: its image, the area of the slide it covers, and the
/// picture and scale it was made from.
#[derive(Clone)]
struct Frame {
    image: Arc<RenderImage>,
    area: crate::document::Frame,
    /// For a video drawn as it is: the box of the shape, which cuts the
    /// image, and its corner radius, in slide units.
    clip: Option<(crate::document::Frame, f32)>,
    /// The address of the picture, or the serial of a video frame drawn as
    /// it is; 0 for none.
    key: usize,
    scale: f32,
}

/// What a fill the GPU can draw directly, without the CPU renderer, is made
/// of: a video frame fitted with [`crate::document::ImageFit`], or a shader
/// frame, which always fills its box exactly.
enum DirectContent {
    Video(crate::document::ImageFit),
    Shader,
}

/// The content and the corner radius of a video or shader fill that the GPU
/// can draw as it is: in a rectangle without rotation, stroke or
/// transparency, whose corners cut the picture as they cut the shape. A
/// shader fills its box exactly, so its corners always cut this way; a
/// video does only when its fit leaves no box uncovered, or its corner
/// radius is 0. Other fills go through the CPU renderer, which draws any
/// shape.
fn direct_fill(element: &Element, opacity: f32) -> Option<(DirectContent, f32)> {
    let crate::document::ElementKind::Rectangle(rectangle) = &element.kind else {
        return None;
    };
    let radius = rectangle.corner_radius;
    let plain = element.frame.rotation == 0. && rectangle.stroke.is_none();
    match &rectangle.fill {
        Fill::Video(video) => {
            let corners_cut = radius == 0. || video.fit != crate::document::ImageFit::Contain;
            (plain && opacity * video.opacity >= 1. && corners_cut)
                .then_some((DirectContent::Video(video.fit), radius))
        }
        Fill::Shader(shader) => {
            (plain && opacity * shader.opacity >= 1.).then_some((DirectContent::Shader, radius))
        }
        _ => None,
    }
}

/// Flags the key of a video frame drawn as it is, apart from addresses.
const SERIAL_KEY: usize = 1 << (usize::BITS - 1);

/// Flags the key of a shader frame drawn as it is, apart from addresses and
/// from a video's [`SERIAL_KEY`]: the two serials do not share a number
/// space, so a fill that changes kind cannot show a stale frame with a key
/// that happens to match.
const SHADER_SERIAL_KEY: usize = 1 << (usize::BITS - 2);

/// Performance figures of the presentation, shown with D.
#[derive(Default)]
struct Stats {
    shown: bool,
    /// When the last renders ran, for the frames per second.
    renders: VecDeque<std::time::Instant>,
    /// When a fill got a new picture, for the pictures per second.
    pictures: VecDeque<std::time::Instant>,
    /// The frames the video players decoded, and when, for the decoded
    /// frames per second.
    decoded: VecDeque<(std::time::Instant, u64)>,
    /// Shader submits skipped for a full ring of readback buffers, and
    /// when, for the skipped submits per second.
    skipped: VecDeque<(std::time::Instant, u64)>,
    /// Milliseconds of each step of a new picture, averaged.
    picture_ms: f32,
    raster_ms: f32,
    convert_ms: f32,
    /// Milliseconds of the work of a render before its paint, and of the
    /// paint, averaged.
    render_ms: f32,
    paint_ms: std::rc::Rc<std::cell::Cell<f32>>,
    /// Pixels of the last rasterized fill.
    raster_size: (u32, u32),
}

/// The weight of a new measure in an average.
const STATS_WEIGHT: f32 = 0.1;

fn average(value: &mut f32, measure: std::time::Duration) {
    let ms = measure.as_secs_f32() * 1000.;
    *value = if *value == 0. {
        ms
    } else {
        *value + (ms - *value) * STATS_WEIGHT
    };
}

impl Stats {
    /// Drops what is older than a second, and returns how many are left.
    fn per_second<T>(times: &mut VecDeque<T>, at: impl Fn(&T) -> std::time::Instant) -> usize {
        let now = std::time::Instant::now();
        while times
            .front()
            .is_some_and(|first| now.duration_since(at(first)).as_secs_f32() > 1.)
        {
            times.pop_front();
        }
        times.len()
    }

    /// The rate of a growing count kept as (when, cumulative total) pairs
    /// over the last second: `decoded` and `skipped` count this way.
    fn per_second_rate(counts: &mut VecDeque<(std::time::Instant, u64)>) -> f32 {
        Self::per_second(counts, |(at, _)| *at);
        match (counts.front(), counts.back()) {
            (Some((first, from)), Some((last, to))) if last > first => {
                to.saturating_sub(*from) as f32 / last.duration_since(*first).as_secs_f32()
            }
            _ => 0.,
        }
    }

    fn lines(&mut self) -> Vec<String> {
        let fps = Self::per_second(&mut self.renders, |at| *at);
        let pictures = Self::per_second(&mut self.pictures, |at| *at);
        let decoded = Self::per_second_rate(&mut self.decoded);
        let skipped = Self::per_second_rate(&mut self.skipped);
        vec![
            format!(
                "{fps} fps · {pictures} new pictures/s · {decoded:.0} decoded frames/s · \
                 {skipped:.0} skipped shader/s"
            ),
            format!(
                "picture {:.1} ms · raster {:.1} ms ({}×{}) · convert {:.1} ms",
                self.picture_ms,
                self.raster_ms,
                self.raster_size.0,
                self.raster_size.1,
                self.convert_ms
            ),
            format!(
                "render {:.1} ms · paint {:.1} ms",
                self.render_ms,
                self.paint_ms.get()
            ),
        ]
    }
}

pub struct PresenterView {
    show: Show,
    focus: FocusHandle,
    layers: Option<Layers>,
    /// The slide and scale whose layers are being rendered.
    rendering: Option<(SlideId, f32)>,
    /// The frame of each video and shader fill painted last; dropped from
    /// the GPU when replaced.
    frames: HashMap<ElementId, Frame>,
    /// Images to drop from the GPU on the next paint.
    stale: Vec<Arc<RenderImage>>,
    stats: Stats,
}

impl PresenterView {
    pub fn new(presentation: Presentation, index: usize, cx: &mut Context<Self>) -> Self {
        PresenterView {
            show: Show::new(presentation, index, Playback::default()),
            focus: cx.focus_handle(),
            layers: None,
            rendering: None,
            frames: HashMap::new(),
            stale: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// The scale and the offset that fit the slide in the window, centered.
    fn placement(&self, viewport: gpui_kit::Size<Pixels>) -> (f32, gpui_kit::Point<Pixels>) {
        let slide = self.show.presentation.size;
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height));
        let scale = (width / slide.width as f32).min(height / slide.height as f32);
        let offset = point(
            px((width - slide.width as f32 * scale) / 2.),
            px((height - slide.height as f32 * scale) / 2.),
        );
        (scale, offset)
    }

    /// Renders the still layers of the slide in the background when the
    /// shown ones are for another slide or scale.
    fn want_layers(&mut self, scale: f32, cx: &mut Context<Self>) {
        let Some(slide) = self.show.slide_id() else {
            return;
        };
        let current = self
            .layers
            .as_ref()
            .is_some_and(|layers| layers.slide == slide && layers.scale == scale);
        if current || self.rendering == Some((slide, scale)) {
            return;
        }
        self.rendering = Some((slide, scale));
        let presentation = self.show.presentation.clone();
        cx.spawn(async move |this, cx| {
            let rendered = gpui_kit::AppContext::background_spawn(cx, async move {
                let apart = |element: &Element| element.kind.fill().is_some_and(Fill::is_animated);
                let layers = render::render_layers(&presentation, slide, scale, &apart).ok()?;
                Some(
                    layers
                        .into_iter()
                        .filter_map(|layer| match layer {
                            Layer::Still(pixmap) => {
                                Some(ShownLayer::Still(Arc::new(render_image(&pixmap)?)))
                            }
                            Layer::Apart(id) => Some(ShownLayer::Apart(id)),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .await;
            this.update(cx, |this, cx| {
                if this.rendering != Some((slide, scale)) {
                    return;
                }
                this.rendering = None;
                if let Some(layers) = rendered {
                    if let Some(old) = this.layers.take() {
                        this.stale
                            .extend(old.layers.into_iter().filter_map(|layer| match layer {
                                ShownLayer::Still(image) => Some(image),
                                ShownLayer::Apart(_) => None,
                            }));
                    }
                    this.layers = Some(Layers {
                        slide,
                        scale,
                        layers,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renders the current frame of each video and shader fill of the
    /// slide at `scale`: the live frame of a started one, the still picture
    /// of one that did not start. A frame that did not change keeps its
    /// image.
    fn render_frames(&mut self, scale: f32) {
        let Some(slide) = self.show.presentation.slides.get(self.show.index) else {
            return;
        };
        let nodes: Vec<(Element, f32)> = slide
            .visible_leaves()
            .filter(|node| node.element.kind.fill().is_some_and(Fill::is_animated))
            .map(|node| (node.element.clone(), node.opacity))
            .collect();
        let mut shown = HashMap::new();
        for (element, opacity) in nodes {
            let Some(fill) = element.kind.fill() else {
                continue;
            };
            let id = element.id;
            let presentation = &self.show.presentation;
            let device = (
                (element.frame.width * scale).round().max(1.) as u32,
                (element.frame.height * scale).round().max(1.) as u32,
            );
            // A shader's direct path has its own `ShaderPlayer`, at a
            // different format than the CPU path's: falling through to
            // `picture` below when it has no frame yet would recreate the
            // player back and forth between the two formats. A video has a
            // single player for both paths, so it can fall through safely.
            let mut direct_shader_pending = false;
            if let Some((content, radius)) = direct_fill(&element, opacity) {
                direct_shader_pending = matches!(content, DirectContent::Shader);
                let started = std::time::Instant::now();
                let box_ = element.frame;
                let direct = match content {
                    DirectContent::Video(fit) => self.show.playback.video_frame(id).map(
                        |video| -> (u32, u32, Vec<u8>, usize, crate::document::Frame) {
                            let key = SERIAL_KEY | video.serial as usize;
                            let (x, y, width, height) = crate::shape::fit_rect(
                                fit,
                                box_.width,
                                box_.height,
                                (video.width, video.height),
                            );
                            let area = crate::document::Frame {
                                x: box_.x + x,
                                y: box_.y + y,
                                width,
                                height,
                                rotation: 0.,
                            };
                            (video.width, video.height, video.bgra.to_vec(), key, area)
                        },
                    ),
                    DirectContent::Shader => self
                        .show
                        .playback
                        .shader_frame(id, fill, presentation, device)
                        .map(|shader| {
                            let key = SHADER_SERIAL_KEY | shader.serial as usize;
                            (
                                shader.width,
                                shader.height,
                                shader.bytes.to_vec(),
                                key,
                                box_,
                            )
                        }),
                };
                if let Some((width, height, bgra, key, area)) = direct {
                    let picture_time = started.elapsed();
                    if let Some(frame) = self.frames.remove(&id) {
                        if frame.key == key {
                            shown.insert(id, frame);
                            continue;
                        }
                        self.stale.push(frame.image);
                    }
                    let stats = &mut self.stats;
                    average(&mut stats.picture_ms, picture_time);
                    stats.pictures.push_back(std::time::Instant::now());
                    stats.raster_ms = 0.;
                    stats.raster_size = (width, height);
                    let started = std::time::Instant::now();
                    let image = image::RgbaImage::from_raw(width, height, bgra)
                        .map(|buffer| RenderImage::new(vec![image::Frame::new(buffer)]));
                    average(&mut stats.convert_ms, started.elapsed());
                    if let Some(image) = image {
                        shown.insert(
                            id,
                            Frame {
                                image: Arc::new(image),
                                area,
                                clip: Some((box_, radius)),
                                key,
                                scale,
                            },
                        );
                    }
                    continue;
                }
            }
            let started = std::time::Instant::now();
            let live = if direct_shader_pending {
                None
            } else {
                self.show.playback.picture(id, fill, presentation, device)
            };
            let picture_time = started.elapsed();
            let picture = live.or_else(|| {
                // Not started, or no frame yet: the still picture.
                let still = crate::pictures::Picture::of(fill, &element.frame)?;
                still.cached(presentation).or_else(|| {
                    still.load(presentation)?.run();
                    still.cached(presentation)
                })
            });
            let key = picture
                .as_ref()
                .map_or(0, |picture| Arc::as_ptr(picture) as usize);
            if let Some(frame) = self.frames.remove(&id) {
                if frame.key == key && frame.scale == scale {
                    shown.insert(id, frame);
                    continue;
                }
                self.stale.push(frame.image);
            }
            let stats = &mut self.stats;
            average(&mut stats.picture_ms, picture_time);
            stats.pictures.push_back(std::time::Instant::now());
            let pixels = |_: &Fill, _| picture.clone();
            let started = std::time::Instant::now();
            let Some((pixmap, area)) = render::render_shape_box(&element, opacity, scale, &pixels)
            else {
                continue;
            };
            average(&mut stats.raster_ms, started.elapsed());
            stats.raster_size = (pixmap.width(), pixmap.height());
            let started = std::time::Instant::now();
            let image = render_image(&pixmap);
            average(&mut stats.convert_ms, started.elapsed());
            if let Some(image) = image {
                shown.insert(
                    id,
                    Frame {
                        image: Arc::new(image),
                        area,
                        clip: None,
                        key,
                        scale,
                    },
                );
            }
        }
        // The frames of elements no longer shown.
        self.stale
            .extend(self.frames.drain().map(|(_, frame)| frame.image));
        self.frames = shown;
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "right" | "down" | "space" | "pagedown" | "enter" => {
                self.show.advance();
            }
            "left" | "up" | "pageup" | "backspace" => {
                self.show.back();
            }
            "d" => self.stats.shown = !self.stats.shown,
            "escape" => {
                self.show.playback.clear();
                window.remove_window();
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (scale, offset) = self.placement(window.viewport_size());
        let x = f32::from(event.position.x - offset.x) / scale;
        let y = f32::from(event.position.y - offset.y) / scale;
        self.show.click(x, y);
        cx.notify();
    }
}

impl Focusable for PresenterView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PresenterView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (scale, offset) = self.placement(window.viewport_size());
        let scale_factor = window.scale_factor();
        // Rendered at device pixels, so that the slide is sharp.
        let device_scale = scale * scale_factor;
        let started = std::time::Instant::now();
        self.want_layers(device_scale, cx);
        self.render_frames(device_scale);
        average(&mut self.stats.render_ms, started.elapsed());
        let now = std::time::Instant::now();
        self.stats.renders.push_back(now);
        self.stats
            .decoded
            .push_back((now, self.show.playback.delivered()));
        self.stats
            .skipped
            .push_back((now, self.show.playback.skipped()));
        let hud = self.stats.shown.then(|| {
            div()
                .absolute()
                .top(px(12.))
                .left(px(12.))
                .p(px(8.))
                .rounded(px(6.))
                .bg(gpui_kit::hsla(0., 0., 0., 0.7))
                .text_color(gpui_kit::white())
                .text_size(px(13.))
                .font_family("monospace")
                .flex()
                .flex_col()
                .children(self.stats.lines())
        });
        let paint_ms = self.stats.paint_ms.clone();
        if self.show.playback.animating() {
            window.request_animation_frame();
        }
        let slide = self.show.presentation.size;
        let slide_size = size(
            px(slide.width as f32 * scale),
            px(slide.height as f32 * scale),
        );
        let layers: Vec<ShownLayer> = self
            .layers
            .as_ref()
            .filter(|layers| Some(layers.slide) == self.show.slide_id())
            .map(|layers| {
                layers
                    .layers
                    .iter()
                    .map(|layer| match layer {
                        ShownLayer::Still(image) => ShownLayer::Still(image.clone()),
                        ShownLayer::Apart(id) => ShownLayer::Apart(*id),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let frames = self.frames.clone();
        let stale = std::mem::take(&mut self.stale);
        div()
            .id("presenter")
            .track_focus(&self.focus)
            .size_full()
            .bg(gpui_kit::black())
            .cursor_pointer()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let started = std::time::Instant::now();
                        for image in stale {
                            window.drop_image(image).ok();
                        }
                        let origin = bounds.origin + offset;
                        let slide_bounds = Bounds::new(origin, slide_size);
                        if layers.is_empty() {
                            window.paint_quad(gpui_kit::fill(slide_bounds, gpui_kit::white()));
                        }
                        for layer in &layers {
                            match layer {
                                ShownLayer::Still(image) => {
                                    window
                                        .paint_image(
                                            slide_bounds,
                                            slide_bounds,
                                            Default::default(),
                                            image.clone(),
                                            0,
                                            false,
                                        )
                                        .ok();
                                }
                                ShownLayer::Apart(id) => {
                                    let Some(Frame {
                                        image, area, clip, ..
                                    }) = frames.get(id)
                                    else {
                                        continue;
                                    };
                                    let place = |frame: &crate::document::Frame| {
                                        Bounds::new(
                                            origin
                                                + point(px(frame.x * scale), px(frame.y * scale)),
                                            size(px(frame.width * scale), px(frame.height * scale)),
                                        )
                                    };
                                    let area_bounds = place(area);
                                    let (bounds, radius) = match clip {
                                        Some((shape, radius)) => (place(shape), *radius * scale),
                                        None => (area_bounds, 0.),
                                    };
                                    window
                                        .paint_image(
                                            bounds,
                                            area_bounds,
                                            gpui_kit::Corners::all(px(radius)),
                                            image.clone(),
                                            0,
                                            false,
                                        )
                                        .ok();
                                }
                            }
                        }
                        let mut ms = paint_ms.get();
                        average(&mut ms, started.elapsed());
                        paint_ms.set(ms);
                    },
                )
                .size_full(),
            )
            .children(hud)
    }
}

impl crate::editor::EditorView {
    /// Starts the presentation full screen from the current slide, on the
    /// display of the editor.
    pub fn present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_text_edit();
        self.preview.borrow_mut().clear();
        let presentation = self.presentation.clone();
        let index = self
            .presentation
            .index_of(self.current_slide)
            .unwrap_or_default();
        let display = window.display(cx);
        let bounds = display
            .as_ref()
            .map(|display| display.bounds())
            .unwrap_or_else(|| window.bounds());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Fullscreen(bounds)),
            display_id: display.as_ref().map(|display| display.id()),
            titlebar: None,
            focus: true,
            app_id: Some("sliderino".into()),
            ..WindowOptions::default()
        };
        cx.spawn(async move |_, cx| {
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| PresenterView::new(presentation, index, cx));
                window.focus(&view.read(cx).focus.clone(), cx);
                // Some window managers ignore full screen bounds when a
                // window opens.
                if !window.is_fullscreen() {
                    window.toggle_fullscreen();
                }
                view
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ElementKind, RectangleElement, ShaderFill, Stroke};
    use crate::script;

    /// A rectangle with a shader fill, at (0, 0, 100, 100).
    fn shader_element() -> Element {
        Element::new(
            ElementId(1),
            crate::document::Frame {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
                rotation: 0.,
            },
            ElementKind::Rectangle(RectangleElement {
                fill: Fill::Shader(ShaderFill::default()),
                stroke: None,
                corner_radius: 0.,
            }),
        )
    }

    #[test]
    fn a_shader_in_a_plain_rectangle_is_direct() {
        let element = shader_element();
        assert!(direct_fill(&element, 1.).is_some());
    }

    #[test]
    fn a_rotated_shader_is_not_direct() {
        let mut element = shader_element();
        element.frame.rotation = 45.;
        assert!(direct_fill(&element, 1.).is_none());
    }

    #[test]
    fn a_stroked_shader_is_not_direct() {
        let mut element = shader_element();
        let ElementKind::Rectangle(rectangle) = &mut element.kind else {
            unreachable!()
        };
        rectangle.stroke = Some(Stroke::default());
        assert!(direct_fill(&element, 1.).is_none());
    }

    #[test]
    fn a_transparent_shader_is_not_direct() {
        let mut element = shader_element();
        let ElementKind::Rectangle(rectangle) = &mut element.kind else {
            unreachable!()
        };
        let Fill::Shader(shader) = &mut rectangle.fill else {
            unreachable!()
        };
        shader.opacity = 0.5;
        assert!(direct_fill(&element, 1.).is_none());
        assert!(direct_fill(&element, 0.5).is_none());
    }

    /// Two slides: the first with an on-click shader at the bottom, an
    /// auto shader and an on-click shader on top.
    fn show() -> Show {
        let mut presentation = Presentation::new();
        let ops = script::parse(
            r#"[{"op": "add_element", "slide": 1, "element": {"id": "$low",
                  "frame": {"x": 0, "y": 0, "width": 800, "height": 900},
                  "rectangle": {"fill": {"shader": {"start": "on_click"}}}}},
                {"op": "add_element", "slide": 1, "element": {"id": "$auto",
                  "frame": {"x": 800, "y": 0, "width": 800, "height": 450},
                  "rectangle": {"fill": {"shader": {}}}}},
                {"op": "add_element", "slide": 1, "element": {"id": "$high",
                  "frame": {"x": 800, "y": 450, "width": 800, "height": 450},
                  "rectangle": {"fill": {"shader": {"start": "on_click"}}}}},
                {"op": "add_slide"}]"#,
        )
        .unwrap();
        script::apply(&mut presentation, ops).unwrap();
        Show::new(presentation, 0, Playback::default())
    }

    #[test]
    fn clicks_start_the_on_click_fills_in_layer_order_then_change_slide() {
        let mut show = show();
        assert!(show.playback.is_playing(ElementId(2)), "auto starts");
        assert!(!show.playback.is_live(ElementId(1)));
        assert_eq!(show.advance(), Step::Started(ElementId(1)));
        assert_eq!(show.advance(), Step::Started(ElementId(3)));
        assert_eq!(show.advance(), Step::Slide(1));
        assert!(
            !show.playback.is_live(ElementId(2)),
            "the slide before stops"
        );
        assert_eq!(show.advance(), Step::None);
        assert_eq!(show.back(), Step::Slide(0));
        assert_eq!(show.queue, [ElementId(1), ElementId(3)]);
        assert_eq!(show.back(), Step::None);
    }

    #[test]
    fn a_click_on_a_fill_starts_or_pauses_it() {
        let mut show = show();
        // The on-click fill on top, out of its turn.
        assert_eq!(show.click(1200., 700.), Step::Started(ElementId(3)));
        assert_eq!(show.queue, [ElementId(1)]);
        assert_eq!(show.click(1200., 700.), Step::Toggled(ElementId(3)));
        assert!(!show.playback.is_playing(ElementId(3)));
        assert_eq!(show.click(1200., 200.), Step::Toggled(ElementId(2)));
        // Elsewhere: the next of the queue.
        show.queue.clear();
        assert_eq!(show.click(-10., -10.), Step::Slide(1));
    }
}
