//! Center area: the slide on the canvas ground, with zoom and pan, its
//! elements, direct manipulation (create, select, move, resize, edit text)
//! and the tool palette.

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{Selectable as _, h_flex};
use gpui_kit::{
    AnyElement, App, BorderStyle, BoxShadow, Context, CursorStyle, DispatchPhase, Edges,
    ElementInputHandler, Entity, ExternalPaths, FillOptions, FocusHandle, FontId, FontWeight,
    GlyphId, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PathStyle, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, Styled, TestSupportExt as _, Window, canvas as paint_canvas,
    div, fill, hsla, outline, point, px, size,
};

use crate::camera::Camera;
use crate::document::{
    Element, ElementId, ElementKind, EllipseElement, FontData, Frame, LineElement, Operation,
    RectangleElement, SlideId, SlideSize, TextElement, TextSizing, TextStyle, normalize_degrees,
};
use crate::editor::{Drag, EditorView, Preview, SlidePoint, Tool};
use crate::shortcuts::WheelAction;
use crate::snap::{Guide, Handle, ResizeMode, Targets, resize_rotated, snap_move, snap_resize};
use crate::text_layout::{BoxRect, TextLayout};
use crate::theme;
use crate::ui::hierarchy_panel::layer_menu;
use crate::ui::image_insert::ImageTarget;
use crate::ui::shape_paint::{PaintShape, paint_shape, render_image, translated};

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

/// Distance on screen outside a corner of the selection within which a drag
/// rotates it.
const ROTATE_REACH: f32 = 18.;

/// Step of the angle while Shift is held, in degrees.
const ROTATE_STEP: f32 = 15.;

/// Distance to a quarter turn within which the angle snaps to it, in degrees.
const ROTATE_SNAP: f32 = 2.;

/// A text element ready to paint: its layout and the GPUI font of its face.
#[derive(Clone)]
pub struct PaintText {
    pub id: ElementId,
    pub frame: Frame,
    pub layout: Arc<TextLayout>,
    /// None when GPUI cannot load the embedded face: the text is drawn from
    /// the outlines of `font`.
    pub font_id: Option<FontId>,
    /// The embedded face, whose outlines draw rotated text (the glyph atlas
    /// only draws upright glyphs) and text in a face that GPUI cannot load.
    pub font: Option<FontData>,
    pub color: Hsla,
    pub underline: bool,
    pub strikethrough: bool,
}

/// An element ready to paint, in paint order.
#[derive(Clone)]
pub enum PaintItem {
    Text(PaintText),
    Shape(PaintShape),
}

/// Everything the canvas paints over the slide, in slide units.
pub struct CanvasScene {
    items: Vec<PaintItem>,
    /// Box of the selection (see [`EditorView::selection_box`]), whether it
    /// shows resize handles and whether its text overflows.
    selection: Option<(Frame, bool, bool)>,
    /// Thin outlines: each element of a multiple selection, and the layer
    /// under the pointer in the hierarchy.
    outlines: Vec<Frame>,
    /// The text box being edited and its frame; the text selection, the
    /// caret and the marked text are placed in it.
    edit_id: Option<ElementId>,
    edit_frame: Frame,
    highlight: Vec<BoxRect>,
    caret: Option<BoxRect>,
    /// Stretch of text an input method is composing, underlined.
    marked: Vec<BoxRect>,
    preview: Option<Frame>,
    /// The shape being drawn with a shape tool, in the default style.
    ghost: Option<PaintShape>,
    /// The ends of the selected line, drawn as round handles.
    line_ends: Option<((f32, f32), (f32, f32))>,
    marquee: Option<Frame>,
    guides: Vec<Guide>,
    /// Size label, "216 × 148", and the area it sits under.
    badge: Option<(Frame, String)>,
    editing: bool,
}

impl EditorView {
    /// Visible elements of a slide in paint order, texts laid out, ready to
    /// paint. With `preview`, dragged elements show their drag preview;
    /// without, the document (the thumbnails change when the drag ends).
    pub fn paint_items(&mut self, slide: SlideId, preview: bool, cx: &App) -> Vec<PaintItem> {
        let dragged = if preview {
            self.drag_preview()
        } else {
            Preview::None
        };
        let Some(slide) = self.presentation.slide(slide) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for node in slide.visible_leaves() {
            let element = node.element;
            let Some(text) = element.as_text() else {
                if element.kind.is_shape() {
                    let (frame, _) = dragged.apply(element);
                    let picture = element.kind.fill().and_then(|fill| {
                        if preview {
                            self.canvas_picture(element.id, fill, &frame)
                        } else {
                            self.fill_picture(fill, &frame)
                        }
                    });
                    items.push(PaintItem::Shape(PaintShape {
                        element: Element {
                            frame,
                            ..element.clone()
                        },
                        opacity: node.opacity,
                        picture,
                    }));
                }
                continue;
            };
            let (frame, sizing) = dragged.apply(element);
            let sizing = sizing.unwrap_or(text.sizing);
            let Some(layout) = self
                .layouts
                .get_for(&self.presentation, element.id, frame, sizing)
            else {
                continue;
            };
            let style = &text.style;
            let font = self.presentation.fonts.get(&style.font);
            let font_id = font.and_then(|data| self.fonts.font_id(&style.font, data, cx));
            let font = font.filter(|_| outlined(frame.rotation, font_id)).cloned();
            let color: Hsla = gpui_kit::rgb(style.color.0).into();
            items.push(PaintItem::Text(PaintText {
                id: element.id,
                frame,
                layout,
                font_id,
                font,
                color: color.opacity(node.opacity),
                underline: style.underline,
                strikethrough: style.strikethrough,
            }));
        }
        items
    }

    pub fn canvas_scene(&mut self, cx: &App) -> CanvasScene {
        let _span = crate::perf::span("canvas_scene");
        let items = self.paint_items(self.current_slide, true, cx);
        let mut scene = CanvasScene {
            items,
            selection: None,
            outlines: Vec::new(),
            edit_id: None,
            edit_frame: Frame::default(),
            highlight: Vec::new(),
            caret: None,
            marked: Vec::new(),
            preview: None,
            ghost: None,
            line_ends: None,
            marquee: None,
            guides: self.guides.clone(),
            badge: None,
            editing: self.text_edit.is_some(),
        };
        if let Some(frame) = self.selection_box() {
            let text = self.single_selection().filter(|id| {
                self.presentation
                    .element(*id)
                    .is_some_and(|element| element.as_text().is_some())
            });
            let layout = text.and_then(|id| self.shown_layout(id));
            let overflow = layout.as_ref().is_some_and(|layout| layout.overflow() > 0.);
            let handles = self.text_edit.is_none() && !self.selection_locked();
            let line = self.selected_line();
            scene.selection = Some((frame, handles && line.is_none(), overflow));
            if handles && line.is_some() {
                scene.line_ends = Some(frame.line_ends());
            }
            if self.selection.len() > 1 {
                scene.outlines = self
                    .selection_roots()
                    .into_iter()
                    .filter_map(|id| self.shown_frame(id))
                    .collect();
            }
            if self.text_edit.is_none() {
                // Below the text that overflows the box, so it stays readable.
                let bottom = layout.map_or(0., |layout| {
                    layout
                        .lines
                        .last()
                        .map_or(0., |line| line.top + line.height)
                });
                let under = Frame {
                    height: frame.height.max(bottom),
                    ..frame
                }
                .bounds();
                let label = match &self.drag {
                    Some(Drag::Rotate {
                        origin_rotation,
                        delta,
                        ..
                    }) => format!(
                        "{}°",
                        crate::ui::inspector::number(normalize_degrees(origin_rotation + delta))
                    ),
                    _ if line.is_some() => format!(
                        "{} · {}°",
                        frame.width.round(),
                        crate::ui::inspector::number(frame.rotation)
                    ),
                    _ => format!("{} × {}", frame.width.round(), frame.height.round()),
                };
                scene.badge = Some((under, label));
            }
        }
        if let Some(id) = self.hovered_layer
            && !self.selection.contains(&id)
            && !self.presentation.is_hidden(id)
            && let Some(frame) = self.shown_frame(id)
        {
            scene.outlines.push(frame);
        }
        if let Some(edit) = self.text_edit.clone()
            && let Some(layout) = self.layout_of(edit.id)
            && let Some(frame) = self.frame_of(edit.id)
        {
            scene.edit_id = Some(edit.id);
            scene.edit_frame = frame;
            scene.highlight = layout.selection_rects(edit.selection());
            if let Some(marked) = &edit.marked {
                scene.marked = layout.selection_rects(marked.clone());
            }
            scene.caret = Some(layout.caret(edit.caret));
        }
        match &self.drag {
            Some(Drag::Create { start, current }) => {
                scene.preview = Some(normalized(*start, *current));
            }
            Some(Drag::Marquee { start, current, .. }) => {
                scene.marquee = Some(normalized(*start, *current));
            }
            Some(Drag::Draw {
                tool,
                start,
                current,
                square,
                from_center,
            }) => {
                let frame = drawn_frame(*tool, *start, *current, *square, *from_center);
                let kind = match tool {
                    Tool::Rectangle => ElementKind::Rectangle(RectangleElement::default()),
                    Tool::Ellipse => ElementKind::Ellipse(EllipseElement::default()),
                    _ => ElementKind::Line(LineElement::default()),
                };
                scene.ghost = Some(PaintShape {
                    element: Element::new(ElementId(u64::MAX), frame, kind),
                    opacity: 1.,
                    picture: None,
                });
                scene.badge = Some((
                    frame.bounds(),
                    if *tool == Tool::Line {
                        format!(
                            "{} · {}°",
                            frame.width.round(),
                            crate::ui::inspector::number(frame.rotation)
                        )
                    } else {
                        format!("{} × {}", frame.width.round(), frame.height.round())
                    },
                ));
            }
            _ => {}
        }
        scene
    }

    pub fn frame_of(&self, id: ElementId) -> Option<Frame> {
        self.presentation.element(id).map(|element| element.frame)
    }

    /// Topmost visible, unlocked leaf of the current slide under a slide
    /// point.
    fn leaf_at(&self, at: SlidePoint) -> Option<ElementId> {
        let reach = self.camera.map_or(0., |camera| DRAG_START / camera.zoom);
        self.current_slide()
            .walk()
            .into_iter()
            .rev()
            .filter(|node| !node.hidden && !node.locked && node.element.as_group().is_none())
            .find(|node| {
                let element = node.element;
                if element.kind.is_shape() {
                    crate::shape::hit(element, at.x, at.y, reach)
                } else {
                    inflate(&element.frame, reach).contains(at.x, at.y)
                }
            })
            .map(|node| node.element.id)
    }

    /// The element a click on `leaf` selects, like Figma: the outermost
    /// group holding it, unless the author has entered a group by selecting
    /// inside it (`context`); then the element at that depth.
    pub fn selectable_for(&self, leaf: ElementId, context: &[ElementId]) -> ElementId {
        let mut path = self.presentation.ancestors(leaf);
        path.reverse();
        path.push(leaf);
        let entered: HashSet<ElementId> = context
            .iter()
            .flat_map(|id| self.presentation.ancestors(*id))
            .collect();
        let mut target = path[0];
        for pair in path.windows(2) {
            if entered.contains(&pair[0]) {
                target = pair[1];
            }
        }
        target
    }

    /// The child of `group` on the way down to `leaf`.
    fn child_toward(&self, group: ElementId, leaf: ElementId) -> Option<ElementId> {
        let mut path = self.presentation.ancestors(leaf);
        path.reverse();
        path.push(leaf);
        let index = path.iter().position(|id| *id == group)?;
        path.get(index + 1).copied()
    }

    /// Elements a selection rectangle touches: the elements of the context
    /// holding the touched leaves, or the leaves themselves when `deep`.
    fn marquee_hits(&self, rect: Frame, context: &[ElementId], deep: bool) -> Vec<ElementId> {
        let leaves: Vec<ElementId> = self
            .current_slide()
            .walk()
            .into_iter()
            .filter(|node| !node.hidden && !node.locked && node.element.as_group().is_none())
            .filter(|node| node.element.frame.intersects(&rect))
            .map(|node| node.element.id)
            .collect();
        let mut hits = Vec::new();
        for leaf in leaves {
            let target = if deep {
                leaf
            } else {
                self.selectable_for(leaf, context)
            };
            if !hits.contains(&target) {
                hits.push(target);
            }
        }
        hits
    }

    /// The selected line, when the selection is one line.
    pub fn selected_line(&self) -> Option<ElementId> {
        self.single_selection().filter(|id| {
            self.presentation
                .element(*id)
                .is_some_and(|element| element.is_line())
        })
    }

    /// The end of the selected line under a window position: true for its
    /// end, false for its start.
    fn line_end_at(&self, position: Point<Pixels>) -> Option<bool> {
        if self.text_edit.is_some() || self.selection_locked() {
            return None;
        }
        let frame = self.shown_frame(self.selected_line()?)?;
        let (start, end) = frame.line_ends();
        let near = |(x, y): (f32, f32)| {
            self.to_window(x, y).is_some_and(|at| {
                f32::from(at.x - position.x).hypot(f32::from(at.y - position.y)) <= HANDLE_REACH
            })
        };
        // The end wins when both are under the pointer, so that a line of
        // no length can grow.
        if near(end) {
            Some(true)
        } else if near(start) {
            Some(false)
        } else {
            None
        }
    }

    /// The resize handle of the selection under a window position. A line
    /// has its two ends instead.
    fn handle_at(&self, position: Point<Pixels>) -> Option<Handle> {
        if self.text_edit.is_some() || self.selection_locked() || self.selected_line().is_some() {
            return None;
        }
        let frame = self.selection_box()?;
        Handle::ALL.into_iter().find(|handle| {
            let (x, y) = handle.slide_position(&frame);
            self.to_window(x, y).is_some_and(|at| {
                (f32::from(at.x - position.x)).abs() <= HANDLE_REACH
                    && (f32::from(at.y - position.y)).abs() <= HANDLE_REACH
            })
        })
    }

    /// Whether a window position is in a rotation zone of the selection:
    /// near a corner, outside the box and off the handles.
    pub fn rotation_zone_at(&self, position: Point<Pixels>) -> bool {
        if self.effective_tool() != Tool::Move
            || self.text_edit.is_some()
            || self.selection_locked()
            || self.selected_line().is_some()
            || self.handle_at(position).is_some()
        {
            return false;
        }
        let (Some(frame), Some(at)) = (self.selection_box(), self.to_slide(position)) else {
            return false;
        };
        if frame.contains(at.x, at.y) {
            return false;
        }
        frame.corners().into_iter().any(|(x, y)| {
            self.to_window(x, y).is_some_and(|corner| {
                f32::from(corner.x - position.x).hypot(f32::from(corner.y - position.y))
                    <= ROTATE_REACH
            })
        })
    }

    /// Starts turning the selection around the center of its box.
    fn start_rotate(&mut self, at: SlidePoint) {
        let Some(origin) = self.selection_box() else {
            return;
        };
        let ids = self.selection_roots();
        let (cx, cy) = origin.center();
        let pivot = point(cx, cy);
        let origin_rotation = match ids.as_slice() {
            [_] => origin.rotation,
            _ => 0.,
        };
        self.drag = Some(Drag::Rotate {
            ids,
            pivot,
            grab_angle: angle_to(pivot, at),
            origin,
            origin_rotation,
            delta: 0.,
        });
    }

    /// Lines and baselines the dragged elements snap to: every other visible
    /// element, leaving out their descendants and the groups holding them.
    fn snap_targets(&mut self, ids: &[ElementId]) -> Targets {
        let size = self.presentation.size;
        let mut targets = Targets::new(size.width as f32, size.height as f32);
        let mut excluded: HashSet<ElementId> = HashSet::new();
        for id in ids {
            excluded.extend(self.presentation.ancestors(*id));
        }
        let others: Vec<(ElementId, Frame, bool)> = self
            .current_slide()
            .walk()
            .into_iter()
            .filter(|node| !node.hidden)
            .filter(|node| {
                let id = node.element.id;
                !excluded.contains(&id)
                    && !ids.contains(&id)
                    && !self
                        .presentation
                        .ancestors(id)
                        .iter()
                        .any(|ancestor| ids.contains(ancestor))
            })
            .map(|node| {
                let text = node.element.as_text().is_some();
                (node.element.id, node.element.frame, text)
            })
            .collect();
        for (other, frame, text) in others {
            targets.add_frame(&frame.bounds());
            if text
                && frame.rotation == 0.
                && let Some(layout) = self.layout_of(other)
            {
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
        let (x, y) = frame.to_local(at.x, at.y);
        let index = layout.index_at(x, y);
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

    /// Starts moving the selected roots, unless one is locked.
    fn start_move(&mut self, at: SlidePoint, pressed: Option<ElementId>) {
        if self.selection_locked() {
            return;
        }
        let ids = self.selection_roots();
        if let Some(origin) = self.selection_frame() {
            self.drag = Some(Drag::Move {
                ids,
                pressed,
                grab: at,
                origin,
                current: origin,
                moved: false,
            });
        }
    }

    fn press_with_move_tool(&mut self, at: SlidePoint, event: &MouseDownEvent) {
        if let Some(edit) = &self.text_edit {
            let id = edit.id;
            if self.leaf_at(at) == Some(id) {
                self.press_text(id, at, event);
                return;
            }
            self.end_text_edit();
        }
        if let Some(end) = self.line_end_at(event.position)
            && let Some(id) = self.selected_line()
            && let Some(origin) = self.frame_of(id)
        {
            self.drag = Some(Drag::LineEnd {
                id,
                end,
                origin,
                current: origin,
            });
            return;
        }
        if let Some(handle) = self.handle_at(event.position) {
            let text = self.single_selection().and_then(|id| {
                let element = self.presentation.element(id)?;
                Some((id, element.frame, element.as_text()?.sizing))
            });
            if let Some((id, frame, sizing)) = text {
                self.drag = Some(Drag::Resize {
                    id,
                    handle,
                    grab: at,
                    origin: frame,
                    sizing,
                    current: frame,
                    current_sizing: sizing,
                });
            } else if let Some(origin) = self.selection_box() {
                self.drag = Some(Drag::ResizeGroup {
                    ids: self.selection_roots(),
                    handle,
                    grab: at,
                    origin,
                    current: origin,
                });
            }
            return;
        }
        if self.rotation_zone_at(event.position) {
            self.start_rotate(at);
            return;
        }
        let deep = event.modifiers.secondary();
        let shift = event.modifiers.shift;
        let Some(leaf) = self.leaf_at(at) else {
            let context = std::mem::take(&mut self.selection);
            let base = if shift { context.clone() } else { Vec::new() };
            self.selection = base.clone();
            self.drag = Some(Drag::Marquee {
                start: at,
                current: at,
                base,
                context,
                deep,
            });
            return;
        };
        let target = if deep {
            leaf
        } else {
            self.selectable_for(leaf, &self.selection)
        };
        let is_group = |this: &Self, id| {
            this.presentation
                .element(id)
                .is_some_and(|element| element.as_group().is_some())
        };
        let is_text = |this: &Self, id| {
            this.presentation
                .element(id)
                .is_some_and(|element| element.as_text().is_some())
        };
        if event.click_count >= 2 && !shift {
            if is_group(self, target) {
                // Enters the group: selects its child under the pointer.
                let child = self.child_toward(target, leaf).unwrap_or(leaf);
                self.selection = vec![child];
                self.start_move(at, None);
            } else if is_text(self, target) {
                self.selection = vec![target];
                self.press_text(target, at, event);
            } else {
                self.selection = vec![target];
                self.start_move(at, None);
            }
            return;
        }
        if shift {
            if let Some(index) = self.selection.iter().position(|id| *id == target) {
                self.selection.remove(index);
            } else {
                self.selection.push(target);
                self.start_move(at, None);
            }
        } else if self.selection.contains(&target) {
            self.start_move(at, Some(target));
        } else {
            self.selection = vec![target];
            self.start_move(at, None);
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
        self.collapse_slide_selection();
        if event.button == MouseButton::Right
            && self.effective_tool() == Tool::Move
            && let Some(at) = self.to_slide(event.position)
        {
            // Selects what the menu will act on.
            self.end_text_edit();
            match self.leaf_at(at) {
                Some(leaf) => {
                    let target = self.selectable_for(leaf, &self.selection);
                    if !self.selection.contains(&target) {
                        self.selection = vec![target];
                    }
                }
                None => self.selection.clear(),
            }
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
                let text = self.leaf_at(at).filter(|id| {
                    self.presentation
                        .element(*id)
                        .is_some_and(|element| element.as_text().is_some())
                });
                if let Some(id) = text {
                    self.active_tool = Tool::Move;
                    self.press_text(id, at, event);
                } else {
                    self.end_text_edit();
                    self.selection.clear();
                    self.drag = Some(Drag::Create {
                        start: at,
                        current: at,
                    });
                }
            }
            Tool::Rectangle | Tool::Ellipse | Tool::Line => {
                self.end_text_edit();
                self.selection.clear();
                self.drag = Some(Drag::Draw {
                    tool: self.effective_tool(),
                    start: at,
                    current: at,
                    square: event.modifiers.shift,
                    from_center: event.modifiers.alt,
                });
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
        if self.drag.is_none() {
            let hover = self.rotation_zone_at(event.position);
            if hover != self.hover_rotate {
                self.hover_rotate = hover;
                cx.notify();
            }
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
            Drag::Draw { tool, start, .. } => {
                self.drag = Some(Drag::Draw {
                    tool,
                    start,
                    current: at,
                    square: event.modifiers.shift,
                    from_center: event.modifiers.alt,
                });
            }
            Drag::LineEnd {
                id, end, origin, ..
            } => {
                let (start, finish) = origin.line_ends();
                let fixed = if end { start } else { finish };
                let mut moved = (at.x, at.y);
                self.guides.clear();
                if event.modifiers.shift {
                    moved = snap_direction(fixed, moved, ROTATE_STEP);
                } else if snap {
                    let targets = self.snap_targets(&[id]);
                    let point = Frame {
                        x: moved.0,
                        y: moved.1,
                        ..Frame::default()
                    };
                    let (snapped, guides) = snap_move(&point, None, &targets, threshold);
                    moved = (snapped.x, snapped.y);
                    self.guides = guides;
                }
                let current = if end {
                    Frame::from_line(fixed, moved, origin.rotation)
                } else {
                    Frame::from_line(moved, fixed, origin.rotation)
                };
                self.drag = Some(Drag::LineEnd {
                    id,
                    end,
                    origin,
                    current,
                });
            }
            Drag::Marquee {
                start,
                base,
                context,
                deep,
                ..
            } => {
                let hits = self.marquee_hits(normalized(start, at), &context, deep);
                let mut selection = base.clone();
                selection.extend(hits.into_iter().filter(|id| !base.contains(id)));
                self.selection = selection;
                self.drag = Some(Drag::Marquee {
                    start,
                    current: at,
                    base,
                    context,
                    deep,
                });
            }
            Drag::Move {
                ids,
                pressed,
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
                    let targets = self.snap_targets(&ids);
                    let baseline = match ids.as_slice() {
                        [id] if self.frame_of(*id).is_some_and(|frame| frame.rotation == 0.) => {
                            self.layout_of(*id).map(|layout| layout.first_baseline())
                        }
                        _ => None,
                    };
                    (frame, self.guides) = snap_move(&frame, baseline, &targets, threshold);
                }
                // The document changes once, when the drag ends.
                self.drag = Some(Drag::Move {
                    ids,
                    pressed,
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
                let mut frame = resize_rotated(&origin, handle, at.x - grab.x, at.y - grab.y, mode);
                self.guides.clear();
                if snap && mode == ResizeMode::default() && origin.rotation == 0. {
                    let targets = self.snap_targets(&[id]);
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
            Drag::ResizeGroup {
                ids,
                handle,
                grab,
                origin,
                ..
            } => {
                let mode = ResizeMode {
                    keep_ratio: event.modifiers.shift,
                    from_center: event.modifiers.alt,
                };
                let mut frame = resize_rotated(&origin, handle, at.x - grab.x, at.y - grab.y, mode);
                self.guides.clear();
                if snap && mode == ResizeMode::default() && origin.rotation == 0. {
                    let targets = self.snap_targets(&ids);
                    (frame, self.guides) = snap_resize(&frame, handle, &targets, threshold);
                }
                self.drag = Some(Drag::ResizeGroup {
                    ids,
                    handle,
                    grab,
                    origin,
                    current: frame,
                });
            }
            Drag::SelectText { id } => {
                if let (Some(layout), Some(frame)) = (self.layout_of(id), self.frame_of(id)) {
                    let (x, y) = frame.to_local(at.x, at.y);
                    let index = layout.index_at(x, y);
                    self.move_caret(index, true);
                }
            }
            Drag::Rotate {
                ids,
                pivot,
                grab_angle,
                origin,
                origin_rotation,
                ..
            } => {
                let turned = origin_rotation + angle_to(pivot, at) - grab_angle;
                let rotation = snap_angle(turned, event.modifiers.shift, snap);
                self.drag = Some(Drag::Rotate {
                    ids,
                    pivot,
                    grab_angle,
                    origin,
                    origin_rotation,
                    delta: normalize_degrees(rotation - origin_rotation),
                });
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
        let selection = self.selection.clone();
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
            Drag::Draw {
                tool,
                start,
                current,
                square,
                from_center,
            } => {
                let zoom = self.camera.map_or(1., |camera| camera.zoom);
                let dragged = (current.x - start.x).hypot(current.y - start.y) * zoom >= DRAG_START;
                let frame = if dragged {
                    drawn_frame(tool, start, current, square, from_center)
                } else {
                    default_shape_frame(tool, start)
                };
                self.create_shape(tool, frame);
            }
            Drag::LineEnd {
                id,
                origin,
                current,
                ..
            } => {
                if current != origin {
                    self.commit(
                        "Move line end",
                        Operation::SetFrame { id, frame: current },
                        selection,
                    );
                }
            }
            Drag::Move {
                ids,
                origin,
                current,
                moved: true,
                ..
            } => {
                let operations = self.transform_operations(&ids, origin, current);
                if !operations.is_empty() {
                    self.commit_pruning("Move", operations, selection);
                }
            }
            Drag::Move {
                pressed: Some(pressed),
                moved: false,
                ..
            } => self.selection = vec![pressed],
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
            Drag::ResizeGroup {
                ids,
                origin,
                current,
                ..
            } => {
                let operations = self.transform_operations(&ids, origin, current);
                if !operations.is_empty() {
                    self.commit_pruning("Resize", operations, selection);
                }
            }
            Drag::Rotate {
                ids, pivot, delta, ..
            } => {
                let operations = self.turn_operations(&ids, pivot, delta);
                if !operations.is_empty() {
                    self.commit_pruning("Rotate", operations, selection);
                }
            }
            Drag::Move { .. } | Drag::SelectText { .. } | Drag::Marquee { .. } => {}
        }
        cx.notify();
    }
}

/// Direction from `pivot` to `at`, in degrees clockwise from the x axis.
fn angle_to(pivot: SlidePoint, at: SlidePoint) -> f32 {
    (at.y - pivot.y).atan2(at.x - pivot.x).to_degrees()
}

/// `to` turned around `from` to the nearest multiple of `step` degrees,
/// at the same distance.
fn snap_direction(from: (f32, f32), to: (f32, f32), step: f32) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    let angle = (dy.atan2(dx).to_degrees() / step).round() * step;
    let (x, y) = crate::document::rotate_vector(length, 0., angle);
    (from.0 + x, from.1 + y)
}

/// Side of a shape, and length of a line, made by a click without a drag.
const DEFAULT_SHAPE_SIZE: f32 = 100.;

/// The frame of a shape drawn from `start` to `current`: `square` makes
/// the sides equal, or turns a line to a multiple of 45°; `from_center`
/// grows it both ways from `start`.
pub fn drawn_frame(
    tool: Tool,
    start: SlidePoint,
    current: SlidePoint,
    square: bool,
    from_center: bool,
) -> Frame {
    let (mut dx, mut dy) = (current.x - start.x, current.y - start.y);
    if tool == Tool::Line {
        let from = (start.x, start.y);
        let mut to = (current.x, current.y);
        if square {
            to = snap_direction(from, to, 45.);
        }
        if from_center {
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            return Frame::from_line((from.0 - dx, from.1 - dy), to, 0.);
        }
        return Frame::from_line(from, to, 0.);
    }
    if square {
        let side = dx.abs().max(dy.abs());
        dx = side.copysign(dx);
        dy = side.copysign(dy);
    }
    let (a, b) = if from_center {
        (
            point(start.x - dx, start.y - dy),
            point(start.x + dx, start.y + dy),
        )
    } else {
        (start, point(start.x + dx, start.y + dy))
    };
    normalized(a, b)
}

/// The shape a click makes: a square centered on the click, or a
/// horizontal line.
fn default_shape_frame(tool: Tool, at: SlidePoint) -> Frame {
    let half = DEFAULT_SHAPE_SIZE / 2.;
    if tool == Tool::Line {
        return Frame::from_line((at.x - half, at.y), (at.x + half, at.y), 0.);
    }
    Frame {
        x: at.x - half,
        y: at.y - half,
        width: DEFAULT_SHAPE_SIZE,
        height: DEFAULT_SHAPE_SIZE,
        rotation: 0.,
    }
}

/// The angle a rotation drag gives: steps of [`ROTATE_STEP`] with `step`
/// (Shift), else pulled to a quarter turn within [`ROTATE_SNAP`] when
/// snapping is on.
pub fn snap_angle(degrees: f32, step: bool, snap: bool) -> f32 {
    let nearest = |step: f32| (degrees / step).round() * step;
    let angle = if step {
        nearest(ROTATE_STEP)
    } else if snap && (degrees - nearest(90.)).abs() <= ROTATE_SNAP {
        nearest(90.)
    } else {
        degrees
    };
    normalize_degrees(angle)
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
        (false, _, Some(Drag::Resize { handle, .. } | Drag::ResizeGroup { handle, .. })) => {
            let rotation = editor.selection_box().map_or(0., |frame| frame.rotation);
            resize_cursor(*handle, rotation)
        }
        (false, _, Some(Drag::Rotate { .. })) => CursorStyle::Crosshair,
        (false, Tool::Move, None) if editor.hover_rotate => CursorStyle::Crosshair,
        (false, Tool::Text, _) | (false, _, Some(Drag::SelectText { .. })) => CursorStyle::IBeam,
        (false, Tool::Rectangle | Tool::Ellipse | Tool::Line, _) => CursorStyle::Crosshair,
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
        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
            let (fonts, images): (Vec<_>, Vec<_>) = paths
                .paths()
                .iter()
                .cloned()
                .partition(|path| crate::fonts::is_font_path(path));
            if !fonts.is_empty() {
                this.load_font_files(fonts, false, window, cx);
            }
            if !images.is_empty() {
                // Images land where they are dropped.
                let at = this
                    .to_slide(window.mouse_position())
                    .map(|at| (at.x, at.y));
                this.load_image_files(images, ImageTarget::Insert(at), window, cx);
            }
        }))
        .context_menu({
            let editor = cx.entity().downgrade();
            move |menu, _, cx| layer_menu(menu, &editor, cx)
        })
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

/// The resize cursor nearest to the direction of `handle` on a frame turned
/// by `rotation` degrees.
fn resize_cursor(handle: Handle, rotation: f32) -> CursorStyle {
    let direction = match handle {
        Handle::Left | Handle::Right => 0.,
        Handle::TopLeft | Handle::BottomRight => 45.,
        Handle::Top | Handle::Bottom => 90.,
        Handle::TopRight | Handle::BottomLeft => 135.,
    };
    // A cursor points both ways, so directions repeat every half turn.
    let eighth = ((direction + rotation) / 45.).round() as i32;
    match eighth.rem_euclid(4) {
        0 => CursorStyle::ResizeLeftRight,
        1 => CursorStyle::ResizeUpLeftDownRight,
        2 => CursorStyle::ResizeUpDown,
        _ => CursorStyle::ResizeUpRightDownLeft,
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

/// Whether a text is drawn from the outlines of its face instead of the
/// glyph atlas: the atlas draws no rotated glyphs, and no glyphs of a face
/// that GPUI cannot load. GPUI finds faces only by family name, so a face
/// without the letter "m" or with the name of another installed font is
/// not loaded.
pub fn outlined(rotation: f32, font_id: Option<FontId>) -> bool {
    rotation != 0. || font_id.is_none()
}

/// Paints elements with the slide's top-left corner at `origin`, scaled by
/// `zoom`, in order. Shared by the canvas and the slide thumbnails; with
/// `raster_turned` (the thumbnails), rotated texts are cached images instead
/// of paths.
pub fn paint_items(
    items: &[PaintItem],
    origin: Point<Pixels>,
    zoom: f32,
    raster_turned: bool,
    window: &mut Window,
) {
    for item in items {
        paint_item(item, origin, zoom, raster_turned, window);
    }
}

fn paint_item(
    item: &PaintItem,
    origin: Point<Pixels>,
    zoom: f32,
    raster_turned: bool,
    window: &mut Window,
) {
    match item {
        PaintItem::Text(text) => paint_text(text, origin, zoom, raster_turned, window),
        PaintItem::Shape(shape) => paint_shape(shape, origin, zoom, raster_turned, window),
    }
}

fn paint_text(
    text: &PaintText,
    origin: Point<Pixels>,
    zoom: f32,
    raster_turned: bool,
    window: &mut Window,
) {
    let _span = crate::perf::span("paint_glyphs");
    let at = |x: f32, y: f32| origin + point(px(x * zoom), px(y * zoom));
    let Some(font_id) = text.font_id.filter(|_| text.frame.rotation == 0.) else {
        paint_turned_text(text, origin, zoom, raster_turned, window);
        return;
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

/// Builds glyph outlines into a GPUI path, from the frame's local
/// coordinates through `place`.
struct TurnedGlyphs<'a> {
    builder: PathBuilder,
    place: &'a dyn Fn(f32, f32) -> Point<Pixels>,
    scale: f32,
    x: f32,
    y: f32,
}

impl TurnedGlyphs<'_> {
    fn at(&self, x: f32, y: f32) -> Point<Pixels> {
        (self.place)(self.x + x * self.scale, self.y - y * self.scale)
    }
}

impl ttf_parser::OutlineBuilder for TurnedGlyphs<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let to = self.at(x, y);
        self.builder.move_to(to);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.at(x, y);
        self.builder.line_to(to);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (ctrl, to) = (self.at(x1, y1), self.at(x, y));
        self.builder.curve_to(to, ctrl);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b, to) = (self.at(x1, y1), self.at(x2, y2), self.at(x, y));
        self.builder.cubic_bezier_to(to, a, b);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

/// What a tessellated rotated text depends on, besides its layout: its
/// position is left out, so moving it, panning or drawing it in a thumbnail
/// reuses the tessellation.
#[derive(Clone, Copy, PartialEq)]
struct TurnedKey {
    width: f32,
    height: f32,
    rotation: f32,
    zoom: f32,
    underline: bool,
    strikethrough: bool,
    /// Device pixels per window pixel for an image; 0 for paths.
    raster: f32,
    /// Baked into an image; paths take it when painted.
    color: Hsla,
}

/// How a rotated text is drawn, placed from the center of its frame.
enum Turned {
    /// Glyphs and decorations as paths, in window pixels. Sharp at any size,
    /// but each glyph is hundreds of vertices.
    Paths {
        glyphs: Box<gpui_kit::Path<Pixels>>,
        decorations: Option<Box<gpui_kit::Path<Pixels>>>,
    },
    /// One image, and the area it covers in slide units. One sprite per box,
    /// for the thumbnails, which draw every slide at every frame.
    Image {
        image: Arc<gpui_kit::RenderImage>,
        area: Frame,
    },
}

/// A rotated text ready to place.
struct TurnedEntry {
    /// Held so that the address of the layout is not reused by another.
    layout: Arc<TextLayout>,
    key: TurnedKey,
    turned: Turned,
    used: u64,
}

/// Tessellations and images of rotated texts. Tessellating glyph outlines
/// costs milliseconds per text box: done every frame for every box on the
/// canvas and in the thumbnails, it stalls the editor.
#[derive(Default)]
struct TurnedCache {
    entries: Vec<TurnedEntry>,
    /// Counts the calls to [`paint_turned_text`]; entries unused for
    /// [`TURNED_KEEP`] calls go when the cache grows past [`TURNED_ENTRIES`].
    clock: u64,
}

const TURNED_ENTRIES: usize = 256;
const TURNED_KEEP: u64 = 1024;

thread_local! {
    static TURNED: std::cell::RefCell<TurnedCache> = std::cell::RefCell::default();
}

/// Tessellates the glyphs and decorations of a rotated text, in window
/// pixels from the center of its frame.
fn tessellate_turned(
    text: &PaintText,
    zoom: f32,
) -> Option<(gpui_kit::Path<Pixels>, Option<gpui_kit::Path<Pixels>>)> {
    let _span = crate::perf::span("tessellate_turned_text");
    let data = text.font.as_ref()?;
    let face = ttf_parser::Face::parse(&data.bytes, data.index).ok()?;
    let frame = text.frame;
    let layout = &text.layout;
    let (cx, cy) = frame.center();
    let place = |x: f32, y: f32| {
        let (x, y) = frame.to_slide(x, y);
        point(px((x - cx) * zoom), px((y - cy) * zoom))
    };
    let mut glyphs = TurnedGlyphs {
        builder: PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero())),
        place: &place,
        scale: layout.font_size / face.units_per_em() as f32,
        x: 0.,
        y: 0.,
    };
    for line in &layout.lines {
        for glyph in &line.glyphs {
            glyphs.x = glyph.x;
            glyphs.y = line.baseline + glyph.y;
            face.outline_glyph(ttf_parser::GlyphId(glyph.id), &mut glyphs);
        }
    }
    let glyphs = glyphs.builder.build().ok()?;

    // Apart from the glyphs: with the non-zero rule, a contour of a glyph
    // that winds the other way would cut a hole in the line.
    let mut lines = PathBuilder::fill();
    let mut any = false;
    let decorations = layout.decorations;
    for line in &layout.lines {
        if line.right <= line.left {
            continue;
        }
        let mut stroke = |offset: f32, thickness: f32| {
            let top = line.baseline + offset;
            // At least one screen pixel thick, like the upright text.
            let bottom = top + thickness.max(1. / zoom);
            let corners = [
                (line.left, top),
                (line.right, top),
                (line.right, bottom),
                (line.left, bottom),
            ]
            .map(|(x, y)| place(x, y));
            lines.add_polygon(&corners, true);
            any = true;
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
    let decorations = if any { lines.build().ok() } else { None };
    Some((glyphs, decorations))
}

/// Renders a rotated text into an image at `scale` device pixels per slide
/// unit, as GPUI takes it: BGRA with straight alpha.
fn rasterize_turned(text: &PaintText, scale: f32) -> Option<Turned> {
    let _span = crate::perf::span("rasterize_turned_text");
    let font = text.font.as_ref()?;
    let rgba = text.color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    let mut color = crate::render::Paint::default();
    color.set_color_rgba8(
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b),
        channel(rgba.a),
    );
    color.anti_alias = true;
    let ink = crate::render::TextInk {
        font,
        layout: &text.layout,
        color,
        underline: text.underline,
        strikethrough: text.strikethrough,
    };
    let (pixmap, area) = crate::render::render_text_box(&ink, &text.frame, scale)?;
    let image = render_image(&pixmap)?;
    let (cx, cy) = text.frame.center();
    Some(Turned::Image {
        image: Arc::new(image),
        area: Frame {
            x: area.x - cx,
            y: area.y - cy,
            ..area
        },
    })
}

/// Paints a rotated text element: the glyph atlas only draws upright
/// glyphs. With `raster`, as a cached image (for the thumbnails); else from
/// the cached tessellation of its glyph outlines. See [`TurnedCache`].
fn paint_turned_text(
    text: &PaintText,
    origin: Point<Pixels>,
    zoom: f32,
    raster: bool,
    window: &mut Window,
) {
    let _span = crate::perf::span("paint_turned_text");
    let frame = text.frame;
    let key = TurnedKey {
        width: frame.width,
        height: frame.height,
        rotation: frame.rotation,
        zoom,
        underline: text.underline,
        strikethrough: text.strikethrough,
        raster: if raster { window.scale_factor() } else { 0. },
        color: if raster { text.color } else { Hsla::default() },
    };
    let (cx, cy) = frame.center();
    let center = origin + point(px(cx * zoom), px(cy * zoom));
    let mut evicted = Vec::new();
    let placed = TURNED.with_borrow_mut(|cache| {
        cache.clock += 1;
        let clock = cache.clock;
        let found = cache
            .entries
            .iter_mut()
            .position(|entry| entry.key == key && Arc::ptr_eq(&entry.layout, &text.layout));
        let index = match found {
            Some(index) => index,
            None => {
                let turned = if raster {
                    rasterize_turned(text, zoom * key.raster)?
                } else {
                    let (glyphs, decorations) = tessellate_turned(text, zoom)?;
                    Turned::Paths {
                        glyphs: Box::new(glyphs),
                        decorations: decorations.map(Box::new),
                    }
                };
                if cache.entries.len() >= TURNED_ENTRIES {
                    let (kept, old): (Vec<_>, Vec<_>) = std::mem::take(&mut cache.entries)
                        .into_iter()
                        .partition(|entry| clock - entry.used < TURNED_KEEP);
                    cache.entries = kept;
                    evicted.extend(old);
                    if cache.entries.len() >= TURNED_ENTRIES {
                        evicted.append(&mut cache.entries);
                    }
                }
                cache.entries.push(TurnedEntry {
                    layout: text.layout.clone(),
                    key,
                    turned,
                    used: clock,
                });
                cache.entries.len() - 1
            }
        };
        let entry = &mut cache.entries[index];
        entry.used = clock;
        Some(match &entry.turned {
            Turned::Paths {
                glyphs,
                decorations,
            } => Turned::Paths {
                glyphs: Box::new(translated(glyphs, center)),
                decorations: decorations
                    .as_ref()
                    .map(|path| Box::new(translated(path, center))),
            },
            Turned::Image { image, area } => Turned::Image {
                image: image.clone(),
                area: *area,
            },
        })
    });
    // Images leave the sprite atlas with the cache entry.
    for entry in evicted {
        if let Turned::Image { image, .. } = entry.turned {
            window.drop_image(image).ok();
        }
    }
    match placed {
        Some(Turned::Paths {
            glyphs,
            decorations,
        }) => {
            window.paint_path(*glyphs, text.color);
            if let Some(decorations) = decorations {
                window.paint_path(*decorations, text.color);
            }
        }
        Some(Turned::Image { image, area }) => {
            let bounds = gpui_kit::Bounds::new(
                center + point(px(area.x * zoom), px(area.y * zoom)),
                size(px(area.width * zoom), px(area.height * zoom)),
            );
            window
                .paint_image(bounds, bounds, Default::default(), image, 0, false)
                .ok();
        }
        None => {}
    }
}

/// The window point of a point given in the local coordinates of `frame`.
fn slide_point(frame: &Frame, x: f32, y: f32, origin: Point<Pixels>, zoom: f32) -> Point<Pixels> {
    let (x, y) = frame.to_slide(x, y);
    origin + point(px(x * zoom), px(y * zoom))
}

/// The corners of `rect`, given in the local coordinates of `frame`, in the
/// window.
fn turned_corners(
    frame: &Frame,
    rect: &BoxRect,
    origin: Point<Pixels>,
    zoom: f32,
) -> [Point<Pixels>; 4] {
    [
        (rect.left, rect.top),
        (rect.right, rect.top),
        (rect.right, rect.bottom),
        (rect.left, rect.bottom),
    ]
    .map(|(x, y)| slide_point(frame, x, y, origin, zoom))
}

/// Fills `rect`, given in the local coordinates of `frame`, turned with it.
fn fill_turned(
    window: &mut Window,
    frame: &Frame,
    rect: &BoxRect,
    origin: Point<Pixels>,
    zoom: f32,
    color: Hsla,
) {
    if frame.rotation == 0. {
        let at = |x: f32, y: f32| origin + point(px(x * zoom), px(y * zoom));
        window.paint_quad(fill(
            gpui_kit::Bounds::from_corners(
                at(frame.x + rect.left, frame.y + rect.top),
                at(frame.x + rect.right, frame.y + rect.bottom),
            ),
            color,
        ));
        return;
    }
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&turned_corners(frame, rect, origin, zoom), true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Draws the one-pixel outline of `frame`, turned or not.
fn outline_turned(
    window: &mut Window,
    frame: &Frame,
    origin: Point<Pixels>,
    zoom: f32,
    color: Hsla,
) {
    let local = BoxRect {
        left: 0.,
        top: 0.,
        right: frame.width,
        bottom: frame.height,
    };
    if frame.rotation == 0. {
        let [top_left, _, bottom_right, _] = turned_corners(frame, &local, origin, zoom);
        window.paint_quad(outline(
            gpui_kit::Bounds::from_corners(top_left, bottom_right),
            color,
            BorderStyle::Solid,
        ));
        return;
    }
    let mut builder = PathBuilder::stroke(px(1.));
    builder.add_polygon(&turned_corners(frame, &local, origin, zoom), true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
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

            let edit = scene.edit_frame;
            for item in &scene.items {
                // Under the edited text, above what is under it.
                if let PaintItem::Text(text) = item
                    && Some(text.id) == scene.edit_id
                {
                    for highlight in &scene.highlight {
                        let color = theme::accent().opacity(0.22);
                        fill_turned(window, &edit, highlight, slide.origin, zoom, color);
                    }
                }
                paint_item(item, slide.origin, zoom, false, window);
            }

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

            // One screen pixel, in slide units.
            let pixel = 1. / zoom;
            for marked in &scene.marked {
                let line = BoxRect {
                    top: marked.bottom,
                    bottom: marked.bottom + pixel,
                    ..*marked
                };
                fill_turned(window, &edit, &line, slide.origin, zoom, theme::ink());
            }
            if let Some(caret) = &scene.caret {
                let bar = BoxRect {
                    left: caret.left - 0.75 * pixel,
                    right: caret.left + 0.75 * pixel,
                    ..*caret
                };
                fill_turned(window, &edit, &bar, slide.origin, zoom, theme::accent());
            }
            for frame in &scene.outlines {
                outline_turned(window, frame, slide.origin, zoom, theme::accent());
            }
            if let Some(frame) = scene.marquee {
                window.paint_quad(fill(
                    rect(
                        frame.x,
                        frame.y,
                        frame.x + frame.width,
                        frame.y + frame.height,
                    ),
                    theme::accent().opacity(0.08),
                ));
                outline_turned(window, &frame, slide.origin, zoom, theme::accent());
            }
            if let Some((frame, handles, overflow)) = scene.selection {
                let color = if overflow {
                    theme::warn()
                } else {
                    theme::accent()
                };
                outline_turned(window, &frame, slide.origin, zoom, color);
                if handles {
                    for handle in Handle::ALL {
                        let (x, y) = handle.slide_position(&frame);
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
            if let Some(ghost) = &scene.ghost {
                paint_shape(ghost, slide.origin, zoom, false, window);
                outline_turned(
                    window,
                    &ghost.element.frame,
                    slide.origin,
                    zoom,
                    theme::accent(),
                );
            }
            if let Some((start, end)) = scene.line_ends {
                for (x, y) in [start, end] {
                    let center = at(x, y);
                    let half = px(HANDLE_SIZE / 2. + 1.);
                    window.paint_quad(gpui_kit::quad(
                        gpui_kit::Bounds::from_corners(
                            center - point(half, half),
                            center + point(half, half),
                        ),
                        half,
                        theme::background(),
                        px(1.),
                        theme::accent(),
                        BorderStyle::Solid,
                    ));
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
        .on_click(cx.listener(move |this, _, window, cx| {
            if tool == Tool::Image {
                // Not a mode: the image goes in the middle of the slide.
                this.choose_image(ImageTarget::Insert(None), window, cx);
            } else {
                this.active_tool = tool;
            }
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
