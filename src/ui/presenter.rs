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
    /// The address of the picture; 0 for none.
    key: usize,
    scale: f32,
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
            let live = self.show.playback.picture(id, fill, presentation, device);
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
            let pixels = |_: &Fill, _| picture.clone();
            let Some((pixmap, area)) = render::render_shape_box(&element, opacity, scale, &pixels)
            else {
                continue;
            };
            if let Some(image) = render_image(&pixmap) {
                shown.insert(
                    id,
                    Frame {
                        image: Arc::new(image),
                        area,
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
        self.want_layers(device_scale, cx);
        self.render_frames(device_scale);
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
                                    let Some(Frame { image, area, .. }) = frames.get(id) else {
                                        continue;
                                    };
                                    let area_bounds = Bounds::new(
                                        origin + point(px(area.x * scale), px(area.y * scale)),
                                        size(px(area.width * scale), px(area.height * scale)),
                                    );
                                    window
                                        .paint_image(
                                            area_bounds,
                                            area_bounds,
                                            Default::default(),
                                            image.clone(),
                                            0,
                                            false,
                                        )
                                        .ok();
                                }
                            }
                        }
                    },
                )
                .size_full(),
            )
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
    use crate::script;

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
