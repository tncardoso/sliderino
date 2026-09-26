//! Center area: the slide on the canvas ground, with zoom and pan, its
//! elements, direct manipulation (create, select, move, resize, edit text)
//! and the tool palette.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{Selectable as _, h_flex};
use gpui_kit::{
    AnyElement, App, BorderStyle, BoxShadow, Context, CursorStyle, DispatchPhase, Edges,
    ElementInputHandler, Entity, FocusHandle, FontId, FontWeight, GlyphId, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, ScrollDelta, ScrollWheelEvent, Styled,
    TestSupportExt as _, Window, canvas as paint_canvas, div, fill, hsla, outline, point, px, size,
};

use crate::camera::Camera;
use crate::document::{
    ElementId, Frame, Operation, SlideId, SlideSize, TextElement, TextSizing, TextStyle,
};
use crate::editor::{Drag, EditorView, SlidePoint, Tool};
use crate::shortcuts::WheelAction;
use crate::snap::{Guide, Handle, ResizeMode, Targets, resize, snap_move, snap_resize};
use crate::text_layout::{BoxRect, TextLayout};
use crate::theme;

/// Space kept around a fitted slide; the bottom clears the tool palette.
const FIT_INSETS: Edges<gpui_kit::Pixels> = Edges {
    top: px(48.),
    right: px(48.),
    bottom: px(104.),
    left: px(48.),
};

/// Pixel scroll deltas (trackpads) count as one wheel line per this distance.
const PIXELS_PER_LINE: f32 = 50.;

/// Distance on screen within which a drag snaps.
const SNAP_DISTANCE: f32 = 6.;

/// Distance on screen the pointer travels before a press becomes a drag.
const DRAG_START: f32 = 3.;

/// Half the side of the square around a handle that picks it.
const HANDLE_REACH: f32 = 6.;

/// Side of a drawn resize handle.
const HANDLE_SIZE: f32 = 7.;

/// A text element ready to paint: its layout and the GPUI font of its face.
#[derive(Clone)]
pub struct PaintText {
    pub frame: Frame,
    pub layout: Arc<TextLayout>,
    /// None when GPUI cannot load the embedded face; the text is skipped.
    pub font_id: Option<FontId>,
    pub color: Hsla,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Everything the canvas paints over the slide, in slide units.
pub struct CanvasScene {
    texts: Vec<PaintText>,
    /// Outline of the selected element, whether it shows resize handles and
    /// whether its text overflows.
    selection: Option<(Frame, bool, bool)>,
    highlight: Vec<BoxRect>,
    caret: Option<BoxRect>,
    /// Stretch of text an input method is composing, underlined.
    marked: Vec<BoxRect>,
    preview: Option<Frame>,
    guides: Vec<Guide>,
    /// Size label, "216 × 148", and the area it sits under.
    badge: Option<(Frame, String)>,
    editing: bool,
}

impl EditorView {
    /// Text elements of a slide, laid out and ready to paint. With
    /// `preview`, a dragged element shows its drag preview; without, the
    /// document (the thumbnails change when the drag ends).
    pub fn paint_texts(&mut self, slide: SlideId, preview: bool, cx: &App) -> Vec<PaintText> {
        let Some(slide) = self.presentation.slide(slide) else {
            return Vec::new();
        };
        let dragged = self.drag_preview().filter(|_| preview);
        let mut texts = Vec::new();
        for (element, text) in slide.visible_texts() {
            let (frame, sizing) = match dragged {
                Some((id, frame, sizing)) if id == element.id => (frame, sizing),
                _ => (element.frame, text.sizing),
            };
            let Some(layout) = self
                .layouts
                .get_for(&self.presentation, element.id, frame, sizing)
            else {
                continue;
            };
            let style = &text.style;
            let font_id = self
                .presentation
                .fonts
                .get(&style.font)
                .and_then(|data| self.fonts.font_id(&style.font, data, cx));
            let color: Hsla = gpui_kit::rgb(style.color.0).into();
            texts.push(PaintText {
                frame,
                layout,
                font_id,
                color: color.opacity(style.opacity),
                underline: style.underline,
                strikethrough: style.strikethrough,
            });
        }
        texts
    }

    pub fn canvas_scene(&mut self, cx: &App) -> CanvasScene {
        let _span = crate::perf::span("canvas_scene");
        let texts = self.paint_texts(self.current_slide, true, cx);
        let mut scene = CanvasScene {
            texts,
            selection: None,
            highlight: Vec::new(),
            caret: None,
            marked: Vec::new(),
            preview: None,
            guides: self.guides.clone(),
            badge: None,
            editing: self.text_edit.is_some(),
        };
        if let Some(id) = self.selection
            && let Some(frame) = self.shown_frame(id)
        {
            let overflow = self
                .shown_layout(id)
                .is_some_and(|layout| layout.overflow() > 0.);
            scene.selection = Some((frame, self.text_edit.is_none(), overflow));
            if self.text_edit.is_none() {
                // Below the text that overflows the box, so it stays readable.
                let bottom = self.shown_layout(id).map_or(0., |layout| {
                    layout
                        .lines
                        .last()
                        .map_or(0., |line| line.top + line.height)
                });
                let under = Frame {
                    height: frame.height.max(bottom),
                    ..frame
                };
                scene.badge = Some((
                    under,
                    format!("{} × {}", frame.width.round(), frame.height.round()),
                ));
            }
        }
        if let Some(edit) = self.text_edit.clone()
            && let Some(layout) = self.layout_of(edit.id)
            && let Some(frame) = self.frame_of(edit.id)
        {
            let offset = |rect: BoxRect| BoxRect {
                left: rect.left + frame.x,
                top: rect.top + frame.y,
                right: rect.right + frame.x,
                bottom: rect.bottom + frame.y,
            };
            scene.highlight = layout
                .selection_rects(edit.selection())
                .into_iter()
                .map(offset)
                .collect();
            if let Some(marked) = &edit.marked {
                scene.marked = layout
                    .selection_rects(marked.clone())
                    .into_iter()
                    .map(offset)
                    .collect();
            }
            scene.caret = Some(offset(layout.caret(edit.caret)));
        }
        if let Some(Drag::Create { start, current }) = &self.drag {
            scene.preview = Some(normalized(*start, *current));
        }
        scene
    }

    pub fn frame_of(&self, id: ElementId) -> Option<Frame> {
        self.presentation.element(id).map(|element| element.frame)
    }

    /// Topmost element of the current slide under a slide point.
    fn element_at(&self, at: SlidePoint) -> Option<ElementId> {
        let reach = self.camera.map_or(0., |camera| DRAG_START / camera.zoom);
        self.current_slide()
            .elements
            .iter()
            .rev()
            .find(|element| inflate(&element.frame, reach).contains(at.x, at.y))
            .map(|element| element.id)
    }

    /// The resize handle of the selection under a window position.
    fn handle_at(&self, position: Point<Pixels>) -> Option<Handle> {
        if self.text_edit.is_some() {
            return None;
        }
        let frame = self.frame_of(self.selection?)?;
        Handle::ALL.into_iter().find(|handle| {
            let (x, y) = handle.position(&frame);
            self.to_window(x, y).is_some_and(|at| {
                (f32::from(at.x - position.x)).abs() <= HANDLE_REACH
                    && (f32::from(at.y - position.y)).abs() <= HANDLE_REACH
            })
        })
    }

    /// Lines and baselines the element at `id` snaps to.
    fn snap_targets(&mut self, id: ElementId) -> Targets {
        let size = self.presentation.size;
        let mut targets = Targets::new(size.width as f32, size.height as f32);
        let others: Vec<(ElementId, Frame)> = self
            .current_slide()
            .elements
            .iter()
            .filter(|element| element.id != id)
            .map(|element| (element.id, element.frame))
            .collect();
        for (other, frame) in others {
            targets.add_frame(&frame);
            if let Some(layout) = self.layout_of(other) {
                targets.baselines.push(frame.y + layout.first_baseline());
            }
        }
        targets
    }

    /// Height of one empty line of text in the default style.
    fn default_line_height(&self) -> f32 {
        let style = TextStyle::default();
        let text = TextElement {
            content: String::new(),
            style: style.clone(),
            sizing: TextSizing::AutoWidth,
        };
        self.presentation
            .fonts
            .get(&style.font)
            .cloned()
            .or_else(|| crate::fonts::data(&style.font))
            .and_then(|data| crate::text_layout::measure(&text, 0., &data).ok())
            .map_or(style.size * 1.2, |(_, height)| height)
    }

    /// Enters editing of a text box at a pointer press, placing the caret or,
    /// on a double click, selecting the word.
    fn press_text(&mut self, id: ElementId, at: SlidePoint, event: &MouseDownEvent) {
        let (Some(layout), Some(frame)) = (self.layout_of(id), self.frame_of(id)) else {
            return;
        };
        let index = layout.index_at(at.x - frame.x, at.y - frame.y);
        let editing = self.text_edit.as_ref().is_some_and(|edit| edit.id == id);
        if !editing {
            self.begin_text_edit(id, index, index);
        }
        if event.click_count >= 2 {
            self.select_word(index);
        } else {
            self.move_caret(index, editing && event.modifiers.shift);
        }
        self.drag = Some(Drag::SelectText { id });
    }

    fn press_with_move_tool(&mut self, at: SlidePoint, event: &MouseDownEvent) {
        if let Some(edit) = &self.text_edit {
            let id = edit.id;
            if self.element_at(at) == Some(id) {
                self.press_text(id, at, event);
                return;
            }
            self.end_text_edit();
        }
        if let Some(handle) = self.handle_at(event.position)
            && let Some(id) = self.selection
            && let Some(element) = self.presentation.element(id)
            && let Some(text) = element.as_text()
        {
            self.drag = Some(Drag::Resize {
                id,
                handle,
                grab: at,
                origin: element.frame,
                sizing: text.sizing,
                current: element.frame,
                current_sizing: text.sizing,
            });
            return;
        }
        match self.element_at(at) {
            Some(id) if event.click_count >= 2 => {
                self.selection = Some(id);
                self.press_text(id, at, event);
            }
            Some(id) => {
                self.selection = Some(id);
                let origin = self.frame_of(id).expect("the hit element exists");
                self.drag = Some(Drag::Move {
                    id,
                    grab: at,
                    origin,
                    current: origin,
                    moved: false,
                });
            }
            None => self.selection = None,
        }
    }

    fn on_canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.pan_drag.is_some() {
            return;
        }
        let pans = self.shortcuts.pan_button.matches(event.button)
            || (event.button == MouseButton::Left && self.effective_tool() == Tool::Hand);
        if pans {
            self.pan_drag = Some((event.button, event.position));
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let Some(at) = self.to_slide(event.position) else {
            return;
        };
        match self.effective_tool() {
            Tool::Text => {
                if let Some(id) = self.element_at(at) {
                    self.active_tool = Tool::Move;
                    self.press_text(id, at, event);
                } else {
                    self.end_text_edit();
                    self.selection = None;
                    self.drag = Some(Drag::Create {
                        start: at,
                        current: at,
                    });
                }
            }
            Tool::Move => self.press_with_move_tool(at, event),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let _span = crate::perf::span("pointer_move");
        if let Some((button, last)) = self.pan_drag {
            if let Some(camera) = &mut self.camera {
                camera.pan_by(event.position - last);
            }
            self.pan_drag = Some((button, event.position));
            cx.notify();
            return;
        }
        let (Some(drag), Some(at), Some(camera)) = (
            self.drag.clone(),
            self.to_slide(event.position),
            self.camera,
        ) else {
            return;
        };
        let threshold = SNAP_DISTANCE / camera.zoom;
        let snap = !self.shortcuts.snap_off.held(&event.modifiers);
        match drag {
            Drag::Create { start, .. } => {
                self.drag = Some(Drag::Create { start, current: at });
            }
            Drag::Move {
                id,
                grab,
                origin,
                moved,
                ..
            } => {
                let distance = (at.x - grab.x).hypot(at.y - grab.y) * camera.zoom;
                if !moved && distance < DRAG_START {
                    return;
                }
                let mut frame = Frame {
                    x: origin.x + at.x - grab.x,
                    y: origin.y + at.y - grab.y,
                    ..origin
                };
                self.guides.clear();
                if snap {
                    let _span = crate::perf::span("snap");
                    let targets = self.snap_targets(id);
                    let baseline = self.layout_of(id).map(|layout| layout.first_baseline());
                    (frame, self.guides) = snap_move(&frame, baseline, &targets, threshold);
                }
                // The document changes once, when the drag ends.
                self.drag = Some(Drag::Move {
                    id,
                    grab,
                    origin,
                    current: frame,
                    moved: true,
                });
            }
            Drag::Resize {
                id,
                handle,
                grab,
                origin,
                sizing,
                ..
            } => {
                let mode = ResizeMode {
                    keep_ratio: event.modifiers.shift,
                    from_center: event.modifiers.alt,
                };
                let mut frame = resize(&origin, handle, at.x - grab.x, at.y - grab.y, mode);
                self.guides.clear();
                if snap && mode == ResizeMode::default() {
                    let targets = self.snap_targets(id);
                    (frame, self.guides) = snap_resize(&frame, handle, &targets, threshold);
                }
                let current_sizing = resized_sizing(sizing, handle);
                let current = self.fit_preview(id, frame, current_sizing);
                self.drag = Some(Drag::Resize {
                    id,
                    handle,
                    grab,
                    origin,
                    sizing,
                    current,
                    current_sizing,
                });
            }
            Drag::SelectText { id } => {
                if let (Some(layout), Some(frame)) = (self.layout_of(id), self.frame_of(id)) {
                    let index = layout.index_at(at.x - frame.x, at.y - frame.y);
                    self.move_caret(index, true);
                }
            }
        }
        cx.notify();
    }

    fn on_pointer_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self
            .pan_drag
            .is_some_and(|(button, _)| button == event.button)
        {
            self.pan_drag = None;
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.guides.clear();
        let selection = self.selection;
        match drag {
            Drag::Create { start, current } => {
                let zoom = self.camera.map_or(1., |camera| camera.zoom);
                let line_height = self.default_line_height();
                let dragged = (current.x - start.x).hypot(current.y - start.y) * zoom >= DRAG_START;
                if dragged {
                    let mut frame = normalized(start, current);
                    frame.height = frame.height.max(line_height);
                    self.create_text(frame, TextSizing::Fixed);
                } else {
                    // The caret's middle lands where the author clicked.
                    let frame = Frame {
                        x: start.x,
                        y: start.y - line_height / 2.,
                        ..Frame::default()
                    };
                    self.create_text(frame, TextSizing::AutoWidth);
                }
                self.active_tool = Tool::Move;
            }
            Drag::Move {
                id,
                origin,
                current,
                moved: true,
                ..
            } => {
                if current != origin {
                    self.commit(
                        "Move",
                        Operation::SetFrame { id, frame: current },
                        selection,
                    );
                }
            }
            Drag::Resize {
                id,
                origin,
                sizing,
                current,
                current_sizing,
                ..
            } => {
                if current != origin || current_sizing != sizing {
                    let mut operations = Vec::new();
                    if current_sizing != sizing {
                        operations.push(Operation::SetTextSizing {
                            id,
                            sizing: current_sizing,
                        });
                    }
                    operations.push(Operation::SetFrame { id, frame: current });
                    self.commit("Resize", Operation::Batch(operations), selection);
                }
            }
            Drag::Move { .. } | Drag::SelectText { .. } => {}
        }
        cx.notify();
    }
}

/// The sizing a text box takes when resized from `handle`: dragging a side
/// of an auto-width box makes it wrap; dragging a top, bottom or corner
/// handle fixes its height.
pub fn resized_sizing(sizing: TextSizing, handle: Handle) -> TextSizing {
    match (handle.is_side(), sizing) {
        (true, TextSizing::AutoWidth) => TextSizing::AutoHeight,
        (true, sizing) => sizing,
        (false, _) => TextSizing::Fixed,
    }
}

fn normalized(a: SlidePoint, b: SlidePoint) -> Frame {
    Frame {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        width: (a.x - b.x).abs(),
        height: (a.y - b.y).abs(),
        rotation: 0.,
    }
}

fn inflate(frame: &Frame, by: f32) -> Frame {
    Frame {
        x: frame.x - by,
        y: frame.y - by,
        width: frame.width + 2. * by,
        height: frame.height + 2. * by,
        rotation: frame.rotation,
    }
}

pub fn canvas(
    editor: &EditorView,
    scene: CanvasScene,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let cursor = match (
        editor.pan_drag.is_some(),
        editor.effective_tool(),
        &editor.drag,
    ) {
        (true, _, _) => CursorStyle::ClosedHand,
        (false, Tool::Hand, _) => CursorStyle::OpenHand,
        (false, _, Some(Drag::Resize { handle, .. })) => resize_cursor(*handle),
        (false, Tool::Text, _) | (false, _, Some(Drag::SelectText { .. })) => CursorStyle::IBeam,
        _ => CursorStyle::Arrow,
    };

    div()
        .id("canvas")
        .test_support()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .bg(theme::canvas())
        .cursor(cursor)
        .on_any_mouse_down(cx.listener(EditorView::on_canvas_mouse_down))
        .on_scroll_wheel(cx.listener(EditorView::on_canvas_scroll))
        .child(viewport_tracker(cx))
        .children(editor.camera.map(|camera| slide(editor, camera)))
        .children(editor.camera.map(|camera| {
            let badge = scene.badge.clone();
            let layer = content_layer(
                scene,
                camera,
                editor.presentation.size,
                editor.focus.clone(),
                cx.entity(),
            );
            div()
                .absolute()
                .size_full()
                .child(layer)
                .children(badge.map(|(frame, label)| size_badge(editor, camera, frame, label)))
        }))
        .child(
            div()
                .absolute()
                .bottom(px(20.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(toolbar(editor, cx)),
        )
}

fn resize_cursor(handle: Handle) -> CursorStyle {
    match handle {
        Handle::Left | Handle::Right => CursorStyle::ResizeLeftRight,
        Handle::Top | Handle::Bottom => CursorStyle::ResizeUpDown,
        Handle::TopLeft | Handle::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        Handle::TopRight | Handle::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Invisible layer covering the canvas. It records the canvas bounds for the
/// camera and listens to the pointer at window level, so a pan or a drag keeps
/// going when the pointer leaves the canvas and ends on a release anywhere.
fn viewport_tracker(cx: &mut Context<EditorView>) -> impl IntoElement {
    let prepaint_view = cx.entity().downgrade();
    let paint_view = prepaint_view.clone();
    paint_canvas(
        move |bounds, _, cx: &mut App| {
            prepaint_view
                .update(cx, |this, cx| {
                    this.viewport = bounds;
                    if this.camera.is_none() {
                        this.zoom_to_fit();
                        cx.notify();
                    }
                })
                .ok();
        },
        move |_, _, window, cx| {
            let panning = paint_view
                .read_with(cx, |this, _| this.pan_drag.is_some())
                .unwrap_or(false);
            if panning {
                window.set_window_cursor_style(CursorStyle::ClosedHand);
            }
            let view = paint_view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    view.update(cx, |this, cx| this.on_pointer_move(event, cx))
                        .ok();
                }
            });
            let view = paint_view.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    view.update(cx, |this, cx| this.on_pointer_up(event, cx))
                        .ok();
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// Paints text elements with the slide's top-left corner at `origin`,
/// scaled by `zoom`. Shared by the canvas and the slide thumbnails.
pub fn paint_texts(texts: &[PaintText], origin: Point<Pixels>, zoom: f32, window: &mut Window) {
    let _span = crate::perf::span("paint_glyphs");
    let at = |x: f32, y: f32| origin + point(px(x * zoom), px(y * zoom));
    for text in texts {
        let Some(font_id) = text.font_id else {
            continue;
        };
        let layout = &text.layout;
        let font_size = px(layout.font_size * zoom);
        let decorations = layout.decorations;
        for line in &layout.lines {
            let baseline = text.frame.y + line.baseline;
            for glyph in &line.glyphs {
                let position = at(text.frame.x + glyph.x, baseline + glyph.y);
                // A glyph the atlas cannot take is dropped, not fatal.
                let _ = window.paint_glyph(
                    position,
                    font_id,
                    GlyphId(glyph.id as u32),
                    font_size,
                    text.color,
                );
            }
            if line.right <= line.left {
                continue;
            }
            let mut stroke = |offset: f32, thickness: f32| {
                let top_left = at(text.frame.x + line.left, baseline + offset);
                let width = (line.right - line.left) * zoom;
                let height = (thickness * zoom).max(1.);
                window.paint_quad(fill(
                    gpui_kit::Bounds::new(top_left, size(px(width), px(height))),
                    text.color,
                ));
            };
            if text.underline {
                stroke(
                    decorations.underline_offset,
                    decorations.underline_thickness,
                );
            }
            if text.strikethrough {
                stroke(
                    decorations.strikeout_offset,
                    decorations.strikeout_thickness,
                );
            }
        }
    }
}

/// Paints the slide's elements and, above them, the editing overlay:
/// selection, handles, caret, snap guides. Registers the text input handler
/// while a text box is being edited.
fn content_layer(
    scene: CanvasScene,
    camera: Camera,
    slide_size: SlideSize,
    focus: FocusHandle,
    view: Entity<EditorView>,
) -> impl IntoElement {
    paint_canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let slide = camera.slide_rect(slide_size, bounds);
            let zoom = camera.zoom;
            let at = |x: f32, y: f32| slide.origin + point(px(x * zoom), px(y * zoom));
            let rect = |left: f32, top: f32, right: f32, bottom: f32| {
                gpui_kit::Bounds::from_corners(at(left, top), at(right, bottom))
            };

            for highlight in &scene.highlight {
                window.paint_quad(fill(
                    rect(
                        highlight.left,
                        highlight.top,
                        highlight.right,
                        highlight.bottom,
                    ),
                    theme::accent().opacity(0.22),
                ));
            }
            paint_texts(&scene.texts, slide.origin, zoom, window);

            // Content past the slide edge stays visible, faded: the export
            // cuts it off.
            let fade = theme::canvas().opacity(0.7);
            for band in [
                gpui_kit::Bounds::from_corners(bounds.origin, point(bounds.right(), slide.top())),
                gpui_kit::Bounds::from_corners(
                    point(bounds.left(), slide.bottom()),
                    bounds.bottom_right(),
                ),
                gpui_kit::Bounds::from_corners(
                    point(bounds.left(), slide.top()),
                    point(slide.left(), slide.bottom()),
                ),
                gpui_kit::Bounds::from_corners(
                    point(slide.right(), slide.top()),
                    point(bounds.right(), slide.bottom()),
                ),
            ] {
                window.paint_quad(fill(band, fade));
            }

            for marked in &scene.marked {
                let left = at(marked.left, marked.bottom);
                window.paint_quad(fill(
                    gpui_kit::Bounds::new(
                        left,
                        size(px((marked.right - marked.left) * zoom), px(1.)),
                    ),
                    theme::ink(),
                ));
            }
            if let Some(caret) = &scene.caret {
                let top = at(caret.left, caret.top);
                window.paint_quad(fill(
                    gpui_kit::Bounds::new(
                        top - point(px(0.75), px(0.)),
                        size(px(1.5), px((caret.bottom - caret.top) * zoom)),
                    ),
                    theme::accent(),
                ));
            }
            if let Some((frame, handles, overflow)) = scene.selection {
                let color = if overflow {
                    theme::warn()
                } else {
                    theme::accent()
                };
                let bounds = rect(
                    frame.x,
                    frame.y,
                    frame.x + frame.width,
                    frame.y + frame.height,
                );
                window.paint_quad(outline(bounds, color, BorderStyle::Solid));
                if handles {
                    for handle in Handle::ALL {
                        let (x, y) = handle.position(&frame);
                        let center = at(x, y);
                        let half = px(HANDLE_SIZE / 2.);
                        window.paint_quad(gpui_kit::quad(
                            gpui_kit::Bounds::from_corners(
                                center - point(half, half),
                                center + point(half, half),
                            ),
                            px(1.),
                            theme::background(),
                            px(1.),
                            color,
                            BorderStyle::Solid,
                        ));
                    }
                }
            }
            if let Some(frame) = scene.preview {
                let bounds = rect(
                    frame.x,
                    frame.y,
                    frame.x + frame.width,
                    frame.y + frame.height,
                );
                window.paint_quad(outline(bounds, theme::accent(), BorderStyle::Solid));
            }
            for guide in &scene.guides {
                let line = match *guide {
                    Guide::Vertical(x) => gpui_kit::Bounds::new(
                        point(at(x, 0.).x, bounds.top()),
                        size(px(1.), bounds.size.height),
                    ),
                    Guide::Horizontal(y) => gpui_kit::Bounds::new(
                        point(bounds.left(), at(0., y).y),
                        size(bounds.size.width, px(1.)),
                    ),
                };
                window.paint_quad(fill(line, theme::guide()));
            }
            if scene.editing {
                window.handle_input(&focus, ElementInputHandler::new(bounds, view.clone()), cx);
            }
        },
    )
    .absolute()
    .size_full()
}

/// The "W × H" label centered under the selection.
fn size_badge(
    editor: &EditorView,
    camera: Camera,
    frame: Frame,
    label: String,
) -> impl IntoElement {
    const WIDTH: f32 = 160.;
    let slide = camera.slide_size(editor.presentation.size);
    // Positions relative to the canvas center, like the slide itself.
    let x = camera.pan.x - slide.width / 2. + px((frame.x + frame.width / 2.) * camera.zoom);
    let y = camera.pan.y - slide.height / 2. + px((frame.y + frame.height) * camera.zoom);
    div()
        .absolute()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div().relative().size_0().child(
                div()
                    .absolute()
                    .left(x - px(WIDTH / 2.))
                    .top(y + px(8.))
                    .w(px(WIDTH))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("size-badge")
                            .test_support()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(2.))
                            .bg(theme::accent())
                            .text_color(theme::background())
                            .text_size(px(11.))
                            .line_height(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(label),
                    ),
            ),
        )
}

/// The slide, positioned from a zero-size anchor at the canvas center so the
/// layout needs only the camera, not the measured bounds.
fn slide(editor: &EditorView, camera: Camera) -> impl IntoElement {
    let size = camera.slide_size(editor.presentation.size);
    div()
        .absolute()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div().relative().size_0().child(
                div()
                    .id("slide")
                    .test_support()
                    .absolute()
                    .left(camera.pan.x - size.width / 2.)
                    .top(camera.pan.y - size.height / 2.)
                    .w(size.width)
                    .h(size.height)
                    .bg(theme::background())
                    .shadow(vec![shadow(1., 2., 0., 0.06), shadow(8., 24., 0., 0.06)]),
            ),
        )
}

impl EditorView {
    /// Fits the slide into the last measured canvas bounds.
    pub fn zoom_to_fit(&mut self) {
        self.camera = Some(Camera::fit(
            self.viewport.size,
            self.presentation.size,
            FIT_INSETS,
        ));
    }

    fn on_canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcuts.wheel != WheelAction::Zoom {
            return;
        }
        let Some(camera) = &mut self.camera else {
            return;
        };
        // Positive y is a scroll up, which zooms in.
        let lines = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / PIXELS_PER_LINE,
        };
        if lines == 0. {
            return;
        }
        let anchor = event.position - self.viewport.center();
        camera.zoom_at(anchor, self.shortcuts.wheel_zoom_step.powf(lines));
        cx.stop_propagation();
        cx.notify();
    }
}

fn shadow(offset_y: f32, blur: f32, spread: f32, alpha: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(0., 0., 0., alpha),
        offset: point(px(0.), px(offset_y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

fn toolbar(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let active = editor.effective_tool();
    let mut items: Vec<AnyElement> = Vec::new();
    for group in Tool::GROUPS {
        if !items.is_empty() {
            items.push(divider().into_any_element());
        }
        for tool in group {
            items.push(tool_button(*tool, active == *tool, cx).into_any_element());
        }
    }

    // Presses on the palette must not start a pan on the canvas below.
    h_flex()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .gap(px(2.))
        .p(px(6.))
        .rounded(px(12.))
        .bg(theme::background())
        .shadow(vec![shadow(0., 0., 1., 0.06), shadow(6., 20., 0., 0.10)])
        .children(items)
}

fn divider() -> impl IntoElement {
    div()
        .w(px(12.))
        .flex()
        .justify_center()
        .child(div().w(px(1.)).h(px(20.)).bg(theme::border()))
}

fn tool_button(tool: Tool, active: bool, cx: &mut Context<EditorView>) -> impl IntoElement {
    let button = Button::new(tool.id())
        .icon(tool.icon())
        .tooltip(tool.label())
        .size(px(36.))
        .rounded(px(8.))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.active_tool = tool;
            cx.notify();
        }));
    if active {
        // A custom variant paints its solid colors only in the selected state.
        button
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(theme::accent())
                    .foreground(theme::background())
                    .hover(theme::accent())
                    .active(theme::accent()),
            )
            .selected(true)
    } else {
        button.ghost()
    }
}

impl Tool {
    /// Tools in palette order; a divider separates each group.
    const GROUPS: [&[Tool]; 3] = [
        &[Tool::Move, Tool::Hand],
        &[
            Tool::Text,
            Tool::Rectangle,
            Tool::Ellipse,
            Tool::Line,
            Tool::Image,
        ],
        &[Tool::Component],
    ];

    fn id(self) -> &'static str {
        match self {
            Tool::Move => "tool-move",
            Tool::Hand => "tool-hand",
            Tool::Text => "tool-text",
            Tool::Rectangle => "tool-rectangle",
            Tool::Ellipse => "tool-ellipse",
            Tool::Line => "tool-line",
            Tool::Image => "tool-image",
            Tool::Component => "tool-component",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tool::Move => "Move",
            Tool::Hand => "Hand",
            Tool::Text => "Text",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Image => "Image",
            Tool::Component => "Insert component",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Tool::Move => IconName::MousePointer2,
            Tool::Hand => IconName::Hand,
            Tool::Text => IconName::Type,
            Tool::Rectangle => IconName::Square,
            Tool::Ellipse => IconName::Circle,
            Tool::Line => IconName::Slash,
            Tool::Image => IconName::Image,
            Tool::Component => IconName::Component,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{InputEvent as _, MouseButton, Pixels, ScrollDelta, TestAppContext, point, px};

    use crate::editor::Tool;
    use crate::ui::test_support::{
        assert_moved, canvas_center, close, drag, key, open, slide, with_window,
    };

    #[gpui_kit::test]
    fn opens_with_the_slide_fitted_and_centered(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, _| {
            let canvas = window.find("canvas").bounds();
            let slide = slide(window);
            let ratio = f32::from(slide.size.width) / f32::from(slide.size.height);
            assert!((ratio - 16. / 9.).abs() < 0.01, "{slide:?}");
            assert!(close(slide.center().x, canvas.center().x));
            assert!(slide.origin.x >= canvas.origin.x + px(48.) - px(0.5));
            assert!(slide.right() <= canvas.right() - px(48.) + px(0.5));
            assert!(slide.bottom() <= canvas.bottom() - px(104.) + px(0.5));
        });
        let label = handle
            .read_with(cx, |editor, _| editor.camera.unwrap().label())
            .unwrap();
        assert!(label.ends_with('%') && label != "100%", "{label}");
    }

    #[gpui_kit::test]
    fn wheel_zooms_around_the_pointer(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let pointer = before.origin + point(before.size.width / 4., before.size.height / 3.);
            let offset = pointer - before.origin;
            let fraction = |v: Pixels| f32::from(v) / f32::from(before.size.width);
            let (fx, fy) = (fraction(offset.x), fraction(offset.y));
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: pointer,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: pointer,
                    delta: ScrollDelta::Lines(point(0., 1.)),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let after = slide(window);
            let ratio = f32::from(after.size.width) / f32::from(before.size.width);
            assert!((ratio - 1.1).abs() < 0.001, "zoomed by {ratio}");
            let anchored = after.origin + point(after.size.width * fx, after.size.width * fy);
            assert!(close(anchored.x, pointer.x) && close(anchored.y, pointer.y));

            for _ in 0..100 {
                window.dispatch_event(
                    gpui_kit::ScrollWheelEvent {
                        position: pointer,
                        delta: ScrollDelta::Lines(point(0., -1.)),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            window.render_frame(cx);
            assert!(close(slide(window).size.width, px(160.)), "stops at 10%");
        });
    }

    #[gpui_kit::test]
    fn middle_button_pans_with_any_tool(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let from = canvas_center(window);
            let delta = point(px(120.), px(-45.));
            drag(window, MouseButton::Middle, from, from + delta, cx);
            assert_moved(before, slide(window), delta);
        });
    }

    #[gpui_kit::test]
    fn left_drag_pans_only_with_the_hand_tool(cx: &mut TestAppContext) {
        let handle = open(cx);
        let delta = point(px(-80.), px(60.));
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let from = canvas_center(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), point(px(0.), px(0.)));

            window.click("tool-hand", cx);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), delta);
        });
        let tool = handle
            .read_with(cx, |editor, _| editor.active_tool)
            .unwrap();
        assert_eq!(tool, Tool::Hand);
    }

    #[gpui_kit::test]
    fn holding_space_over_the_canvas_enables_the_hand(cx: &mut TestAppContext) {
        let handle = open(cx);
        let delta = point(px(50.), px(30.));
        with_window(cx, handle, |window, cx| {
            let from = canvas_center(window);
            window.hover("canvas", cx);
            key(window, "space", true, cx);
            let before = slide(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), delta);

            key(window, "space", false, cx);
            let before = slide(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), point(px(0.), px(0.)));
        });
        let tool = handle
            .read_with(cx, |editor, _| editor.active_tool)
            .unwrap();
        assert_eq!(tool, Tool::Move);
    }

    #[gpui_kit::test]
    fn space_outside_the_canvas_does_nothing(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            // Over the slides panel, left of the canvas.
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: point(px(100.), px(300.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            key(window, "space", true, cx);
        });
        let held = handle
            .read_with(cx, |editor, _| editor.hand_key_held)
            .unwrap();
        assert!(!held);
    }

    #[gpui_kit::test]
    fn ctrl_0_shows_100_percent_and_ctrl_1_fits(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let fitted = slide(window);
            window.hover("canvas", cx);
            key(window, "ctrl-0", true, cx);
            let full = slide(window);
            assert!(close(full.size.width, px(1600.)) && close(full.size.height, px(900.)));
            key(window, "ctrl-1", true, cx);
            assert_eq!(slide(window), fitted);
        });
    }
}
