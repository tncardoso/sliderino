//! The editor window: title bar over the slides panel, canvas and inspector.
//!
//! `EditorView` owns the presentation and is the only place the UI changes
//! it: every change is an [`Operation`] applied through
//! [`EditorView::commit`] (or typed into a text box), so it lands in the
//! shared undo history.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::{AppContext as _, EventEmitter};
use gpui_kit::{
    Bounds, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, KeyUpEvent, Keystroke, MouseButton, ParentElement, Pixels, Point, Render, Styled,
    Subscription, Task, Window, point, px,
};

use crate::camera::Camera;
use crate::document::{
    ApplyError, Element, ElementId, ElementKind, EllipseElement, Fill, Frame, LineElement,
    Operation, Presentation, RectangleElement, Slide, SlideId, TableElement, TextElement,
    TextSizing, TextStyle,
};
use crate::fonts::FontRegistry;
use crate::history::{History, Selection};
use crate::pictures::Picture;
use crate::render::Pixmap;
use crate::shortcuts::Shortcuts;
use crate::snap::{Guide, Handle};
use crate::text_layout::TextLayout;
use crate::theme;
use crate::ui::canvas::canvas;
use crate::ui::inspector::Inspector;
use crate::ui::playback::Playback;
use crate::ui::properties_panel::properties_panel;
use crate::ui::shape_inspector::ShapeInspector;
use crate::ui::slides_panel::slides_panel;
use crate::ui::top_bar::top_bar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Move,
    Hand,
    Text,
    Rectangle,
    Ellipse,
    Line,
    Table,
    Image,
}

gpui_kit::actions!(editor, [NextCell, PreviousCell]);

/// The key context of the editor; its bindings win over those of the
/// window root, such as Tab moving the focus.
const KEY_CONTEXT: &str = "SliderinoEditor";

/// Binds Tab and Shift-Tab to the cells of a table while one is edited.
pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("tab", NextCell, Some(KEY_CONTEXT)),
        gpui_kit::KeyBinding::new("shift-tab", PreviousCell, Some(KEY_CONTEXT)),
    ]);
}

/// A point on the slide, in slide units.
pub type SlidePoint = Point<f32>;

/// A text box, or the text of a table cell, being edited in place.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub id: ElementId,
    /// The row and column of the cell when `id` is a table.
    pub cell: Option<(usize, usize)>,
    /// Byte index of the caret in the content.
    pub caret: usize,
    /// Other end of the selection; equal to `caret` when nothing is selected.
    pub anchor: usize,
    /// Text being composed by an input method, drawn but not yet final.
    pub marked: Option<Range<usize>>,
    /// X the caret keeps while moving up and down across lines.
    pub goal_x: Option<f32>,
}

impl TextEdit {
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    fn collapse(&mut self, index: usize) {
        self.caret = index;
        self.anchor = index;
        self.goal_x = None;
    }
}

/// Cells of a table selected on the canvas: from `anchor` to `focus`, grown
/// to hold whole merges. The table itself is the selected element.
#[derive(Clone, Debug, PartialEq)]
pub struct TableEdit {
    pub id: ElementId,
    pub anchor: (usize, usize),
    pub focus: (usize, usize),
}

impl TableEdit {
    pub fn cell(id: ElementId, row: usize, column: usize) -> Self {
        Self {
            id,
            anchor: (row, column),
            focus: (row, column),
        }
    }

    /// The selected range of `table`.
    pub fn range(&self, table: &TableElement) -> crate::table::CellRange {
        table.expand(&crate::table::CellRange::between(self.anchor, self.focus))
    }
}

/// A pointer drag on the canvas, started with the left button.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    /// Drawing a new text box with the text tool.
    Create {
        start: SlidePoint,
        current: SlidePoint,
    },
    /// Drawing a new rectangle, ellipse or line with its tool. `square`
    /// (Shift) draws a square, a circle or a line at a multiple of 45°;
    /// `from_center` (Alt) draws from the press point outward.
    Draw {
        tool: Tool,
        start: SlidePoint,
        current: SlidePoint,
        square: bool,
        from_center: bool,
    },
    /// Moving one end of a line; the other end stays. The document keeps
    /// `origin` until the drag ends and the canvas shows `current`.
    LineEnd {
        id: ElementId,
        /// The end of the line, not its start.
        end: bool,
        origin: Frame,
        current: Frame,
    },
    /// Moving the selection. `origin` is the union of the moved frames; the
    /// document keeps it until the drag ends and the canvas shows `current`.
    /// `pressed` is the element under the press: a click without a move
    /// selects only it.
    Move {
        ids: Vec<ElementId>,
        pressed: Option<ElementId>,
        grab: SlidePoint,
        origin: Frame,
        current: Frame,
        moved: bool,
    },
    /// Resizing a group or several elements from a handle of their union
    /// `origin`: positions and boxes scale, fonts do not.
    ResizeGroup {
        ids: Vec<ElementId>,
        handle: Handle,
        grab: SlidePoint,
        origin: Frame,
        current: Frame,
    },
    /// Drawing a selection rectangle. `base` stays selected (Shift adds);
    /// `context` is the selection before the press, the groups the author
    /// has entered; `deep` selects leaves instead.
    Marquee {
        start: SlidePoint,
        current: SlidePoint,
        base: Vec<ElementId>,
        context: Vec<ElementId>,
        deep: bool,
    },
    /// Resizing a text box from a handle. The document keeps `origin` and
    /// `sizing` until the drag ends; the canvas shows `current` laid out
    /// with `current_sizing`.
    Resize {
        id: ElementId,
        handle: Handle,
        grab: SlidePoint,
        origin: Frame,
        sizing: TextSizing,
        current: Frame,
        current_sizing: TextSizing,
    },
    /// Extending the text selection of the box being edited.
    SelectText { id: ElementId },
    /// Drawing a new table with the table tool: every [`DRAW_STEP`] of the
    /// drag adds a column or a row.
    ///
    /// [`DRAW_STEP`]: crate::table::DRAW_STEP
    DrawTable {
        start: SlidePoint,
        current: SlidePoint,
    },
    /// Extending the cell selection of a table from `anchor`.
    SelectCells {
        id: ElementId,
        anchor: (usize, usize),
    },
    /// Moving whole rows (`rows`) or columns of a table: `range` goes
    /// before line `to`.
    MoveCells {
        id: ElementId,
        rows: bool,
        range: Range<usize>,
        to: usize,
    },
    /// Turning the selection around `pivot`, the center of its box `origin`.
    /// `origin_rotation` is the angle of a single element, 0 for several;
    /// `delta` is the turn so far. The document keeps the frames until the
    /// drag ends.
    Rotate {
        ids: Vec<ElementId>,
        pivot: SlidePoint,
        grab_angle: f32,
        origin: Frame,
        origin_rotation: f32,
        delta: f32,
    },
}

/// How a drag changes the frames the canvas shows; see
/// [`EditorView::drag_preview`].
#[derive(Clone, Debug, Default)]
pub enum Preview {
    #[default]
    None,
    /// The elements and their descendants move by `dx`, `dy`.
    Move {
        ids: HashSet<ElementId>,
        dx: f32,
        dy: f32,
    },
    /// One element takes another frame and, for text, sizing.
    Resize {
        id: ElementId,
        frame: Frame,
        sizing: Option<TextSizing>,
    },
    /// The elements and their descendants scale from `from` to `to`.
    Scale {
        ids: HashSet<ElementId>,
        from: Frame,
        to: Frame,
    },
    /// The elements and their descendants turn by `degrees` around `pivot`.
    Turn {
        ids: HashSet<ElementId>,
        pivot: (f32, f32),
        degrees: f32,
    },
}

impl Preview {
    /// The frame and, for text, the sizing the canvas shows for `element`.
    pub fn apply(&self, element: &Element) -> (Frame, Option<TextSizing>) {
        let sizing = element.as_text().map(|text| text.sizing);
        match self {
            Preview::Move { ids, dx, dy } if ids.contains(&element.id) => (
                Frame {
                    x: element.frame.x + dx,
                    y: element.frame.y + dy,
                    ..element.frame
                },
                sizing,
            ),
            Preview::Resize {
                id,
                frame,
                sizing: resized,
            } if *id == element.id => (*frame, resized.or(sizing)),
            Preview::Scale { ids, from, to } if ids.contains(&element.id) => (
                crate::document::map_frame(
                    &element.frame,
                    from,
                    to,
                    crate::document::MapMode::of(element),
                ),
                sizing,
            ),
            Preview::Turn {
                ids,
                pivot,
                degrees,
            } if ids.contains(&element.id) => (
                crate::document::turn_frame(&element.frame, *pivot, *degrees),
                sizing,
            ),
            _ => (element.frame, sizing),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Preview::None)
    }
}

/// Adds the ids of the element and its descendants to `set`.
fn collect_ids(element: &Element, set: &mut HashSet<ElementId>) {
    set.insert(element.id);
    for child in element.children() {
        collect_ids(child, set);
    }
}

/// Text layouts of the presentation's elements. A layout depends on the
/// text, its sizing mode and the frame size, not on the frame position, so
/// moving an element reuses its layout.
///
/// Each element keeps its [`LAYOUTS_PER_ELEMENT`] most recent layouts: during
/// a resize the canvas asks for the preview while the panel asks for the
/// document's layout, and one entry would make them evict each other.
#[derive(Default)]
pub struct LayoutCache {
    entries: HashMap<ElementId, Vec<CachedLayout>>,
}

const LAYOUTS_PER_ELEMENT: usize = 2;

struct CachedLayout {
    text: TextElement,
    width: f32,
    height: f32,
    layout: Arc<TextLayout>,
}

impl LayoutCache {
    pub fn get(&mut self, presentation: &Presentation, id: ElementId) -> Option<Arc<TextLayout>> {
        let element = presentation.element(id)?;
        let sizing = element.as_text()?.sizing;
        self.get_for(presentation, id, element.frame, sizing)
    }

    /// The layout of an element's text in another frame or sizing mode, such
    /// as the preview of a resize.
    pub fn get_for(
        &mut self,
        presentation: &Presentation,
        id: ElementId,
        frame: Frame,
        sizing: TextSizing,
    ) -> Option<Arc<TextLayout>> {
        let text = presentation.element(id)?.as_text()?;
        // Only a fixed box places its text by its height.
        let height = if sizing == TextSizing::Fixed {
            frame.height
        } else {
            0.
        };
        let cached = self.entries.entry(id).or_default();
        if let Some(position) = cached.iter().position(|cached| {
            cached.width == frame.width
                && cached.height == height
                && cached.text.sizing == sizing
                && cached.text.content == text.content
                && cached.text.style == text.style
        }) {
            // Most recently used first.
            let hit = cached.remove(position);
            let layout = hit.layout.clone();
            cached.insert(0, hit);
            return Some(layout);
        }
        let _span = crate::perf::span("layout_cache_miss");
        let text = TextElement {
            sizing,
            ..text.clone()
        };
        let font = presentation.fonts.get(&text.style.font)?;
        let layout = Arc::new(crate::text_layout::layout(&text, &frame, font).ok()?);
        cached.insert(
            0,
            CachedLayout {
                text,
                width: frame.width,
                height,
                layout: layout.clone(),
            },
        );
        cached.truncate(LAYOUTS_PER_ELEMENT);
        Some(layout)
    }
}

/// Layouts of the tables of the presentation. The text of each drawn cell
/// is shared as an `Arc`, so the canvas caches keyed by layout keep their
/// entries from frame to frame.
#[derive(Default)]
pub struct TableLayoutCache {
    entries: HashMap<ElementId, Vec<TableView>>,
}

/// A table laid out, with the text layout of each drawn cell, in the order
/// of [`TableLayout::cells`](crate::table::TableLayout::cells).
#[derive(Clone)]
pub struct TableView {
    pub table: TableElement,
    pub layout: Arc<crate::table::TableLayout>,
    pub texts: Vec<Arc<TextLayout>>,
}

impl TableView {
    /// The text layout of the anchor (`row`, `column`).
    pub fn text(&self, row: usize, column: usize) -> Option<&Arc<TextLayout>> {
        let index = self
            .layout
            .cells
            .iter()
            .position(|cell| cell.row == row && cell.column == column)?;
        self.texts.get(index)
    }
}

impl TableLayoutCache {
    /// The layout of a table of the document.
    pub fn get(&mut self, presentation: &Presentation, id: ElementId) -> Option<TableView> {
        let table = presentation.element(id)?.as_table()?;
        self.get_for(presentation, id, table)
    }

    /// The layout of `table`, a version of the table `id` such as the
    /// preview of a resize.
    pub fn get_for(
        &mut self,
        presentation: &Presentation,
        id: ElementId,
        table: &TableElement,
    ) -> Option<TableView> {
        let cached = self.entries.entry(id).or_default();
        if let Some(position) = cached.iter().position(|view| view.table == *table) {
            let hit = cached.remove(position);
            cached.insert(0, hit.clone());
            return Some(hit);
        }
        let _span = crate::perf::span("table_layout_cache_miss");
        let layout = crate::table::layout(table, &presentation.fonts).ok()?;
        let view = TableView {
            table: table.clone(),
            texts: layout
                .cells
                .iter()
                .map(|cell| Arc::new(cell.layout.clone()))
                .collect(),
            layout: Arc::new(layout),
        };
        cached.insert(0, view.clone());
        cached.truncate(LAYOUTS_PER_ELEMENT);
        Some(view)
    }
}

pub struct EditorView {
    /// Slide shown on the canvas.
    pub current_slide: SlideId,
    /// Slides selected in the slides panel, in selection order. Always holds
    /// the current slide; see [`EditorView::click_slide`].
    pub slide_selection: Vec<SlideId>,
    /// Slide a Shift+click in the slides panel selects from.
    pub slide_anchor: SlideId,
    /// Where the slides dragged in the slides panel would land.
    pub slide_drop: Option<(SlideId, crate::ui::slides_panel::SlideDrop)>,
    /// Tab of the left panel: 0 = Slides, 1 = Hierarchy.
    pub library_tab: usize,
    /// Tab of the right panel: 0 = Design, 1 = Notes, 2 = History.
    pub inspector_tab: usize,
    /// Tool picked in the palette; see [`EditorView::effective_tool`].
    pub active_tool: Tool,
    pub presentation: Presentation,
    pub history: History,
    /// Selected elements, always on the current slide, in selection order.
    pub selection: Vec<ElementId>,
    pub text_edit: Option<TextEdit>,
    /// Cells selected in the selected table.
    pub table_edit: Option<TableEdit>,
    /// The last cells copied, with the text they put on the clipboard: a
    /// paste of that text brings their styles and merges back.
    pub table_clip: Option<(String, crate::table::Clip)>,
    /// Where a click inserts a row (`true`) or a column of the selected
    /// table, shown as a "+" button while the pointer is near.
    pub table_insert: Option<(bool, usize)>,
    /// The edges of the selected cells that the stroke of the properties
    /// panel sets.
    pub border_sides: crate::table::Sides,
    pub drag: Option<Drag>,
    /// Layer under the pointer in the hierarchy, outlined on the canvas.
    pub hovered_layer: Option<ElementId>,
    /// Groups whose children the hierarchy hides.
    pub collapsed: HashSet<ElementId>,
    /// Row a Shift+click in the hierarchy selects from.
    pub tree_anchor: Option<ElementId>,
    /// Where the layers dragged in the hierarchy would land.
    pub layer_drop: Option<(ElementId, crate::ui::hierarchy_panel::DropZone)>,
    /// Layer being renamed in the hierarchy.
    pub renaming: Option<crate::ui::hierarchy_panel::Renaming>,
    /// Snap guides of the drag in progress.
    pub guides: Vec<Guide>,
    /// The pointer is in a rotation zone of the selection, outside a corner.
    pub hover_rotate: bool,
    pub layouts: LayoutCache,
    pub tables: TableLayoutCache,
    pub fonts: FontRegistry,
    pub inspector: Inspector,
    pub shape_inspector: ShapeInspector,
    pub table_inspector: crate::ui::table_edit::TableInspector,
    pub shortcuts: Shortcuts,
    /// None until the canvas is first measured; then it is fitted.
    pub camera: Option<Camera>,
    /// Canvas bounds in window coordinates, from the last prepaint.
    pub viewport: Bounds<Pixels>,
    /// Button that started the pan in progress and the last pointer position.
    pub pan_drag: Option<(MouseButton, Point<Pixels>)>,
    /// The hand-hold key (space by default) is down over the canvas.
    pub hand_key_held: bool,
    /// Keyboard focus of the editor; key events reach the canvas through it.
    pub focus: FocusHandle,
    /// Show the slide and select the element of each agent edit.
    pub follow_agents: bool,
    /// The file the presentation was opened from or last saved to.
    pub file: Option<PathBuf>,
    /// The revision of the presentation when it was opened or last saved.
    pub saved_revision: u64,
    /// Pictures of fills that the canvas or the thumbnails would draw but
    /// that are not decoded or rendered yet; see
    /// [`EditorView::load_pictures`].
    pictures_wanted: RefCell<HashSet<Picture>>,
    /// Pictures being loaded in the background.
    pictures_pending: HashSet<Picture>,
    /// Pictures that cannot be loaded; they show as a placeholder.
    pictures_failed: HashSet<Picture>,
    /// The done share, 0 to 1, of the videos being converted for an insert.
    pub converting: Option<f32>,
    /// The video and shader fills of selected shapes that play on the
    /// canvas: the Preview of the inspector. Videos play without sound.
    pub preview: RefCell<Playback>,
    _activation: Subscription,
}

impl EditorView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_document(Presentation::new(), History::default(), window, cx)
    }

    /// An editor on an existing presentation and its undo history, showing
    /// the first slide.
    pub fn with_document(
        presentation: Presentation,
        history: History,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A key up or mouse up lost while the window is inactive would leave
        // the hand tool or a pan stuck on.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.hand_key_held = false;
                this.pan_drag = None;
                cx.notify();
            }
        });
        // Scan the fonts off the UI thread, then show them in the pickers.
        cx.spawn(async move |this, cx| {
            gpui_kit::AppContext::background_spawn(cx, async {
                crate::fonts::catalog();
            })
            .await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        let saved_revision = presentation.revision();
        let first = presentation.slides[0].id;
        Self {
            current_slide: first,
            slide_selection: vec![first],
            slide_anchor: first,
            slide_drop: None,
            library_tab: 0,
            inspector_tab: 0,
            active_tool: Tool::Move,
            presentation,
            history,
            selection: Vec::new(),
            text_edit: None,
            table_edit: None,
            table_clip: None,
            table_insert: None,
            border_sides: crate::table::Sides::All,
            drag: None,
            hovered_layer: None,
            collapsed: HashSet::new(),
            tree_anchor: None,
            layer_drop: None,
            renaming: None,
            guides: Vec::new(),
            hover_rotate: false,
            layouts: LayoutCache::default(),
            tables: TableLayoutCache::default(),
            fonts: FontRegistry::default(),
            inspector: Inspector::new(window, cx),
            shape_inspector: ShapeInspector::new(window, cx),
            table_inspector: crate::ui::table_edit::TableInspector::new(window, cx),
            shortcuts: Shortcuts::default(),
            camera: None,
            viewport: Bounds::default(),
            pan_drag: None,
            hand_key_held: false,
            focus: cx.focus_handle(),
            follow_agents: false,
            file: None,
            saved_revision,
            pictures_wanted: RefCell::new(HashSet::new()),
            pictures_pending: HashSet::new(),
            pictures_failed: HashSet::new(),
            converting: None,
            preview: RefCell::new(Playback::muted()),
            _activation: activation,
        }
    }

    /// The tool that pointer input uses: the hand while the hand key is held
    /// or a pan is in progress, otherwise the palette tool.
    pub fn effective_tool(&self) -> Tool {
        if self.hand_key_held || self.pan_drag.is_some() {
            Tool::Hand
        } else {
            self.active_tool
        }
    }

    pub fn current_slide(&self) -> &Slide {
        self.presentation
            .slide(self.current_slide)
            .expect("the current slide exists")
    }

    pub fn layout_of(&mut self, id: ElementId) -> Option<Arc<TextLayout>> {
        self.layouts.get(&self.presentation, id)
    }

    /// How the drag in progress changes the frames the canvas shows.
    pub fn drag_preview(&self) -> Preview {
        let subtree = |ids: &[ElementId]| {
            let mut set = HashSet::new();
            for id in ids {
                if let Some(element) = self.presentation.element(*id) {
                    collect_ids(element, &mut set);
                }
            }
            set
        };
        match &self.drag {
            Some(Drag::Move {
                ids,
                origin,
                current,
                moved: true,
                ..
            }) => Preview::Move {
                ids: subtree(ids),
                dx: current.x - origin.x,
                dy: current.y - origin.y,
            },
            Some(Drag::Resize {
                id,
                current,
                current_sizing,
                ..
            }) => Preview::Resize {
                id: *id,
                frame: *current,
                sizing: Some(*current_sizing),
            },
            Some(Drag::LineEnd { id, current, .. }) => Preview::Resize {
                id: *id,
                frame: *current,
                sizing: None,
            },
            Some(Drag::ResizeGroup {
                ids,
                origin,
                current,
                ..
            }) => Preview::Scale {
                ids: subtree(ids),
                from: *origin,
                to: *current,
            },
            Some(Drag::Rotate {
                ids, pivot, delta, ..
            }) => Preview::Turn {
                ids: subtree(ids),
                pivot: (pivot.x, pivot.y),
                degrees: *delta,
            },
            _ => Preview::None,
        }
    }

    /// The frame the canvas shows for an element: the drag preview while it
    /// is dragged, else its frame in the document.
    pub fn shown_frame(&self, id: ElementId) -> Option<Frame> {
        let element = self.presentation.element(id)?;
        Some(self.drag_preview().apply(element).0)
    }

    /// The layout the canvas shows for an element; see [`Self::shown_frame`].
    pub fn shown_layout(&mut self, id: ElementId) -> Option<Arc<TextLayout>> {
        let element = self.presentation.element(id)?;
        match self.drag_preview().apply(element) {
            (frame, Some(sizing)) => self.layouts.get_for(&self.presentation, id, frame, sizing),
            (_, None) => None,
        }
    }

    /// The selection if it is one element.
    pub fn single_selection(&self) -> Option<ElementId> {
        match self.selection.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    /// Selected elements without a selected ancestor: the ones a move or a
    /// resize changes.
    pub fn selection_roots(&self) -> Vec<ElementId> {
        self.selection
            .iter()
            .copied()
            .filter(|id| {
                !self
                    .presentation
                    .ancestors(*id)
                    .iter()
                    .any(|ancestor| self.selection.contains(ancestor))
            })
            .collect()
    }

    /// Union of the frames the canvas shows for the selection.
    pub fn selection_frame(&self) -> Option<Frame> {
        let frames: Vec<Frame> = self
            .selection_roots()
            .into_iter()
            .filter_map(|id| self.shown_frame(id))
            .collect();
        crate::document::union(&frames)
    }

    /// The box the canvas draws around the selection, with its handles: the
    /// frame of a single element, rotated or not; the union of the frames for
    /// several, turned with them while they rotate.
    pub fn selection_box(&self) -> Option<Frame> {
        if let Some(Drag::Rotate {
            ids,
            pivot,
            origin,
            delta,
            ..
        }) = &self.drag
            && ids.len() > 1
        {
            return Some(crate::document::turn_frame(
                origin,
                (pivot.x, pivot.y),
                *delta,
            ));
        }
        match self.selection_roots().as_slice() {
            [id] => self.shown_frame(*id),
            _ => self.selection_frame(),
        }
    }

    /// The operations that turn `ids` by `degrees` around `pivot`.
    pub fn turn_operations(
        &self,
        ids: &[ElementId],
        pivot: SlidePoint,
        degrees: f32,
    ) -> Vec<Operation> {
        ids.iter()
            .filter_map(|id| {
                let element = self.presentation.element(*id)?;
                let frame =
                    crate::document::turn_frame(&element.frame, (pivot.x, pivot.y), degrees);
                (frame != element.frame).then_some(Operation::SetFrame { id: *id, frame })
            })
            .collect()
    }

    /// Some selected element is locked, directly or by an ancestor.
    pub fn selection_locked(&self) -> bool {
        self.selection
            .iter()
            .any(|id| self.presentation.is_locked(*id))
    }

    /// The operations that take `ids` from their union `from` to `to`:
    /// positions and boxes scale, fonts do not. A move when the sizes match.
    pub fn transform_operations(
        &self,
        ids: &[ElementId],
        from: Frame,
        to: Frame,
    ) -> Vec<Operation> {
        ids.iter()
            .filter_map(|id| {
                let element = self.presentation.element(*id)?;
                let mode = crate::document::MapMode::of(element);
                let frame = crate::document::map_frame(&element.frame, &from, &to, mode);
                (frame != element.frame).then_some(Operation::SetFrame { id: *id, frame })
            })
            .collect()
    }

    /// Fits a previewed frame to its text like [`Presentation::apply`] does:
    /// an auto-sized box takes the size of its content.
    pub fn fit_preview(&mut self, id: ElementId, frame: Frame, sizing: TextSizing) -> Frame {
        if sizing == TextSizing::Fixed {
            return frame;
        }
        let Some(layout) = self.layouts.get_for(&self.presentation, id, frame, sizing) else {
            return frame;
        };
        let mut fitted = frame;
        if sizing == TextSizing::AutoWidth {
            fitted.width = layout.content_width;
        }
        fitted.height = layout.content_height;
        if frame.rotation != 0. {
            let (x, y) = frame.to_slide(0., 0.);
            fitted = fitted.with_top_left_at(x, y);
        }
        fitted
    }

    /// Ends a move or resize without changing the document. Returns false
    /// when no drag was in progress.
    pub fn cancel_drag(&mut self) -> bool {
        self.guides.clear();
        self.drag.take().is_some_and(|drag| {
            matches!(
                drag,
                Drag::Move { .. }
                    | Drag::Resize { .. }
                    | Drag::ResizeGroup { .. }
                    | Drag::Create { .. }
                    | Drag::Draw { .. }
                    | Drag::LineEnd { .. }
                    | Drag::Marquee { .. }
                    | Drag::Rotate { .. }
                    | Drag::DrawTable { .. }
                    | Drag::MoveCells { .. }
            )
        })
    }

    /// Applies an edit and records it as one undo step, leaving `select`
    /// selected. Returns false (and changes nothing) when the edit fails.
    pub fn commit(&mut self, label: &str, operation: Operation, select: Vec<ElementId>) -> bool {
        self.commit_pruning(label, vec![operation], select)
    }

    /// Like [`Self::commit`] for a batch of operations, and removes the groups
    /// the batch leaves empty in the same undo step.
    pub fn commit_pruning(
        &mut self,
        label: &str,
        operations: Vec<Operation>,
        select: Vec<ElementId>,
    ) -> bool {
        let mut candidates = Vec::new();
        for operation in &operations {
            if let Operation::MoveElement { id, .. } | Operation::RemoveElement { id } = operation {
                candidates.extend(self.presentation.ancestors(*id));
            }
        }
        candidates.sort();
        candidates.dedup();
        let operation = match <[Operation; 1]>::try_from(operations) {
            Ok([operation]) => operation,
            Err(operations) => Operation::Batch(operations),
        };
        let before = self.selection.clone();
        let mut inverses = match self.presentation.apply(operation) {
            Ok(inverse) => vec![inverse],
            Err(error) => {
                eprintln!("sliderino: {label} failed: {error}");
                return false;
            }
        };
        // Removing an empty group may empty its parent in turn.
        loop {
            let empty: Vec<Operation> = candidates
                .iter()
                .filter(|id| {
                    self.presentation
                        .element(**id)
                        .and_then(Element::as_group)
                        .is_some_and(|group| group.children.is_empty())
                })
                .map(|id| Operation::RemoveElement { id: *id })
                .collect();
            if empty.is_empty() {
                break;
            }
            match self.presentation.apply(Operation::Batch(empty)) {
                Ok(inverse) => inverses.push(inverse),
                Err(error) => {
                    eprintln!("sliderino: removing empty groups failed: {error}");
                    break;
                }
            }
        }
        let inverse = if inverses.len() == 1 {
            inverses.pop().expect("one inverse")
        } else {
            inverses.reverse();
            Operation::Batch(inverses)
        };
        let select: Vec<ElementId> = select
            .into_iter()
            .filter(|id| self.presentation.element(*id).is_some())
            .collect();
        self.selection = select.clone();
        self.history.record(label, inverse, before, select);
        true
    }

    /// Reverts the latest step. `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<Result<(), ApplyError>> {
        let index = self.presentation.index_of(self.current_slide);
        let result = self.history.undo(&mut self.presentation)?;
        Some(self.after_history(result, index))
    }

    /// Repeats the latest undone step. `None` when there is nothing to redo.
    pub fn redo(&mut self) -> Option<Result<(), ApplyError>> {
        let index = self.presentation.index_of(self.current_slide);
        let result = self.history.redo(&mut self.presentation)?;
        Some(self.after_history(result, index))
    }

    /// Restores the selection after undo or redo and shows the slide it is on.
    fn after_history(
        &mut self,
        result: Result<Selection, ApplyError>,
        slide_index: Option<usize>,
    ) -> Result<(), ApplyError> {
        let selection = result.inspect_err(|error| eprintln!("sliderino: undo failed: {error}"))?;
        let slides: Vec<SlideId> = selection
            .slides
            .into_iter()
            .filter(|id| self.presentation.slide(*id).is_some())
            .collect();
        if let Some(first) = slides.first() {
            if !slides.contains(&self.current_slide) {
                self.current_slide = *first;
            }
            self.slide_selection = slides;
        }
        self.selection = selection
            .elements
            .into_iter()
            .filter(|id| self.presentation.element(*id).is_some())
            .collect();
        if let Some(location) = self
            .selection
            .first()
            .and_then(|id| self.presentation.locate(*id))
        {
            self.current_slide = location.slide;
        }
        self.repair_view(slide_index);
        Ok(())
    }

    /// Makes the view state valid again after a change the view did not
    /// make: shows a neighbor when the current slide is gone (`slide_index`
    /// is its index before the change), drops a selection or text edit whose
    /// element is gone and keeps the caret inside the edited text.
    pub fn repair_view(&mut self, slide_index: Option<usize>) {
        if self.presentation.slide(self.current_slide).is_none() {
            let last = self.presentation.slides.len() - 1;
            self.current_slide = self.presentation.slides[slide_index.unwrap_or(0).min(last)].id;
        }
        self.slide_selection
            .retain(|id| self.presentation.slide(*id).is_some());
        if !self.slide_selection.contains(&self.current_slide) {
            self.slide_selection = vec![self.current_slide];
        }
        if self.presentation.slide(self.slide_anchor).is_none() {
            self.slide_anchor = self.current_slide;
        }
        let on_slide = |this: &Self, id: ElementId| {
            this.presentation
                .locate(id)
                .is_some_and(|location| location.slide == this.current_slide)
        };
        let selection = std::mem::take(&mut self.selection);
        self.selection = selection
            .into_iter()
            .filter(|id| on_slide(self, *id))
            .collect();
        self.repair_table_edit();
        let edited = self.edit_content().map(str::to_string);
        if let Some(edit) = &mut self.text_edit {
            let content = edited.as_deref();
            match content {
                Some(content) if self.selection == [edit.id] => {
                    let clamp = |index: usize| floor_boundary(content, index.min(content.len()));
                    edit.caret = clamp(edit.caret);
                    edit.anchor = clamp(edit.anchor);
                    edit.marked = None;
                }
                _ => self.text_edit = None,
            }
        }
    }

    /// Shows another slide and selects only it, leaving any text being
    /// edited.
    pub fn select_slide(&mut self, id: SlideId) {
        self.show_slide(id);
        self.slide_selection = vec![id];
        self.slide_anchor = id;
    }

    /// Shows another slide without changing the slide selection.
    fn show_slide(&mut self, id: SlideId) {
        if id != self.current_slide {
            self.end_text_edit();
            self.selection.clear();
            self.current_slide = id;
        }
    }

    /// A click on a slide thumbnail. A plain click selects only the slide,
    /// Ctrl adds or removes it from the selection and Shift selects the
    /// slides from the last plain or Ctrl click to it. The canvas shows the
    /// clicked slide, or another selected one when Ctrl removes it.
    pub fn click_slide(&mut self, id: SlideId, shift: bool, toggle: bool) {
        if shift {
            let (Some(from), Some(to)) = (
                self.presentation.index_of(self.slide_anchor),
                self.presentation.index_of(id),
            ) else {
                return self.select_slide(id);
            };
            self.slide_selection = self.presentation.slides[from.min(to)..=from.max(to)]
                .iter()
                .map(|slide| slide.id)
                .collect();
            self.show_slide(id);
        } else if toggle {
            self.slide_anchor = id;
            if let Some(ix) = self.slide_selection.iter().position(|s| *s == id) {
                if self.slide_selection.len() > 1 {
                    self.slide_selection.remove(ix);
                    if id == self.current_slide {
                        let last = *self.slide_selection.last().expect("a selected slide");
                        self.show_slide(last);
                    }
                }
            } else {
                self.slide_selection.push(id);
                self.show_slide(id);
            }
        } else {
            self.select_slide(id);
        }
    }

    /// Keeps only the current slide selected in the slides panel.
    pub fn collapse_slide_selection(&mut self) {
        self.slide_selection = vec![self.current_slide];
        self.slide_anchor = self.current_slide;
    }

    /// The selected slides in presentation order.
    pub fn selected_slides(&self) -> Vec<SlideId> {
        self.presentation
            .slides
            .iter()
            .map(|slide| slide.id)
            .filter(|id| self.slide_selection.contains(id))
            .collect()
    }

    /// Applies an edit of the slides and records it as one undo step, then
    /// selects `select` and shows the first of them, unless the current
    /// slide is one of them. Returns false (and changes nothing) when the
    /// edit fails.
    pub fn commit_slides(
        &mut self,
        label: &str,
        operations: Vec<Operation>,
        select: Vec<SlideId>,
    ) -> bool {
        self.end_text_edit();
        let before = Selection {
            elements: self.selection.clone(),
            slides: self.selected_slides(),
        };
        let slide_index = self.presentation.index_of(self.current_slide);
        let operation = match <[Operation; 1]>::try_from(operations) {
            Ok([operation]) => operation,
            Err(operations) => Operation::Batch(operations),
        };
        let inverse = match self.presentation.apply(operation) {
            Ok(inverse) => inverse,
            Err(error) => {
                eprintln!("sliderino: {label} failed: {error}");
                return false;
            }
        };
        let select: Vec<SlideId> = select
            .into_iter()
            .filter(|id| self.presentation.slide(*id).is_some())
            .collect();
        if let Some(first) = select.first()
            && !select.contains(&self.current_slide)
        {
            self.show_slide(*first);
        }
        if !select.is_empty() {
            self.slide_anchor = self.current_slide;
            self.slide_selection = select;
        }
        self.repair_view(slide_index);
        let after = Selection {
            elements: self.selection.clone(),
            slides: self.selected_slides(),
        };
        self.history.record(label, inverse, before, after);
        true
    }

    /// Index after the last selected slide.
    fn after_selected_slides(&self) -> usize {
        self.selected_slides()
            .last()
            .and_then(|id| self.presentation.index_of(*id))
            .map_or(usize::MAX, |ix| ix + 1)
    }

    /// Adds an empty slide after the selected ones and shows it.
    pub fn add_slide(&mut self) {
        let id = self.presentation.new_slide_id();
        let index = self.after_selected_slides();
        let slide = Slide::new(id);
        self.commit_slides(
            "Add slide",
            vec![Operation::AddSlide { index, slide }],
            vec![id],
        );
    }

    /// Copies the selected slides, puts the copies after the last of them
    /// and selects the copies.
    pub fn duplicate_slides(&mut self) {
        let sources = self.selected_slides();
        let index = self.after_selected_slides();
        let mut operations = Vec::new();
        let mut copies = Vec::new();
        for (offset, id) in sources.iter().enumerate() {
            let Some(slide) = self.presentation.copy_slide(*id) else {
                continue;
            };
            copies.push(slide.id);
            operations.push(Operation::AddSlide {
                index: index.saturating_add(offset),
                slide,
            });
        }
        if operations.is_empty() {
            return;
        }
        let label = if copies.len() > 1 {
            "Duplicate slides"
        } else {
            "Duplicate slide"
        };
        self.commit_slides(label, operations, copies);
    }

    /// Removes the selected slides and shows the slide that takes the place
    /// of the first one. When all slides are selected, an empty slide
    /// replaces them.
    pub fn delete_slides(&mut self) {
        let doomed = self.selected_slides();
        let Some(first) = doomed
            .first()
            .and_then(|id| self.presentation.index_of(*id))
        else {
            return;
        };
        let remaining: Vec<SlideId> = self
            .presentation
            .slides
            .iter()
            .map(|slide| slide.id)
            .filter(|id| !doomed.contains(id))
            .collect();
        let mut operations = Vec::new();
        let select = match remaining.get(first).or(remaining.last()) {
            Some(neighbor) => *neighbor,
            None => {
                let id = self.presentation.new_slide_id();
                operations.push(Operation::AddSlide {
                    index: usize::MAX,
                    slide: Slide::new(id),
                });
                id
            }
        };
        let label = if doomed.len() > 1 {
            "Delete slides"
        } else {
            "Delete slide"
        };
        operations.extend(doomed.into_iter().map(|id| Operation::RemoveSlide { id }));
        self.commit_slides(label, operations, vec![select]);
    }

    /// Moves the slides `moved` together, in their present order, to `to`:
    /// an index in the present list of slides. They stay selected.
    pub fn move_slides(&mut self, moved: Vec<SlideId>, to: usize) {
        let Some(operations) = self.presentation.slide_moves(&moved, to) else {
            return;
        };
        let label = if moved.len() > 1 {
            "Move slides"
        } else {
            "Move slide"
        };
        self.commit_slides(label, operations, moved);
    }

    /// Creates a text box on the current slide and starts editing it.
    pub fn create_text(&mut self, frame: Frame, sizing: TextSizing) {
        let style = TextStyle::default();
        let mut operations = Vec::new();
        if !self.presentation.fonts.contains(&style.font) {
            let Some(data) = crate::fonts::data(&style.font) else {
                eprintln!("sliderino: the default font is missing");
                return;
            };
            operations.push(Operation::AddFont {
                face: style.font.clone(),
                data,
            });
        }
        let id = self.presentation.new_element_id();
        operations.push(Operation::AddElement {
            slide: self.current_slide,
            parent: None,
            index: usize::MAX,
            element: Element::new(
                id,
                frame,
                ElementKind::Text(TextElement {
                    content: String::new(),
                    style,
                    sizing,
                }),
            ),
        });
        self.end_text_edit();
        if self.commit("Create text", Operation::Batch(operations), vec![id]) {
            self.begin_text_edit(id, 0, 0);
        }
    }

    /// The still picture of a fill in a frame when it is ready. When it is
    /// not, asks for it: it loads in the background after this render, and
    /// the editor renders again when it is ready.
    pub fn fill_picture(&self, fill: &Fill, frame: &Frame) -> Option<Arc<Pixmap>> {
        let picture = Picture::of(fill, frame)?;
        let pixels = picture.cached(&self.presentation);
        if pixels.is_none() && !self.pictures_failed.contains(&picture) {
            self.pictures_wanted.borrow_mut().insert(picture);
        }
        pixels
    }

    /// The picture a fill of the canvas shows: its live frame while it
    /// plays in the Preview, or else its still picture.
    pub fn canvas_picture(&self, id: ElementId, fill: &Fill, frame: &Frame) -> Option<Arc<Pixmap>> {
        let mut preview = self.preview.borrow_mut();
        if fill.is_animated() && preview.is_live(id) {
            let size = |value: f32| value.round().max(1.) as u32;
            let live = preview.picture(
                id,
                fill,
                &self.presentation,
                (size(frame.width), size(frame.height)),
            );
            if live.is_some() {
                return live;
            }
        }
        drop(preview);
        self.fill_picture(fill, frame)
    }

    /// Whether the selected video and shader fills play in the Preview.
    pub fn previewing(&self) -> bool {
        let preview = self.preview.borrow();
        self.selection.iter().any(|id| preview.is_live(*id))
    }

    /// Plays the video and shader fills of the selected shapes on the
    /// canvas, or stops them when they play.
    pub fn toggle_preview(&mut self) {
        if self.previewing() {
            self.preview.borrow_mut().clear();
            return;
        }
        let mut preview = self.preview.borrow_mut();
        for id in &self.selection {
            if let Some(fill) = self
                .presentation
                .element(*id)
                .and_then(|element| element.kind.fill())
            {
                preview.start(*id, fill, &self.presentation);
            }
        }
    }

    /// Stops the Preview of fills that are no longer selected, or no longer
    /// video or shader fills.
    fn sync_preview(&mut self) {
        let mut preview = self.preview.borrow_mut();
        let stale: Vec<ElementId> = preview
            .ids()
            .filter(|id| {
                !self.selection.contains(id)
                    || !self
                        .presentation
                        .element(*id)
                        .and_then(|element| element.kind.fill())
                        .is_some_and(Fill::is_animated)
            })
            .collect();
        for id in stale {
            preview.stop(id);
        }
    }

    /// Whether pictures the canvas or the thumbnails draw are still being
    /// loaded.
    pub fn pictures_loading(&self) -> bool {
        !self.pictures_pending.is_empty() || !self.pictures_wanted.borrow().is_empty()
    }

    /// Loads the pictures asked for since the last render, off the UI
    /// thread.
    fn load_pictures(&mut self, cx: &mut Context<Self>) {
        for picture in self.pictures_wanted.take() {
            if self.pictures_pending.contains(&picture) {
                continue;
            }
            let Some(load) = picture.load(&self.presentation) else {
                continue;
            };
            self.pictures_pending.insert(picture.clone());
            cx.spawn(async move |this, cx| {
                let loaded =
                    gpui_kit::AppContext::background_spawn(cx, async move { load.run() }).await;
                this.update(cx, |this, cx| {
                    this.pictures_pending.remove(&picture);
                    if !loaded {
                        this.pictures_failed.insert(picture);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Adds a rectangle, an ellipse or a line in the default style on top of
    /// the current slide, selects it and goes back to the Move tool.
    pub fn create_shape(&mut self, tool: Tool, frame: Frame) {
        let (label, kind) = match tool {
            Tool::Rectangle => (
                "Create rectangle",
                ElementKind::Rectangle(RectangleElement::default()),
            ),
            Tool::Ellipse => (
                "Create ellipse",
                ElementKind::Ellipse(EllipseElement::default()),
            ),
            Tool::Line => ("Create line", ElementKind::Line(LineElement::default())),
            _ => return,
        };
        let id = self.presentation.new_element_id();
        self.end_text_edit();
        self.commit(
            label,
            Operation::AddElement {
                slide: self.current_slide,
                parent: None,
                index: usize::MAX,
                element: Element::new(id, frame, kind),
            },
            vec![id],
        );
        self.active_tool = Tool::Move;
    }

    /// Enters text editing with the given selection.
    pub fn begin_text_edit(&mut self, id: ElementId, anchor: usize, caret: usize) {
        if self
            .text_edit
            .as_ref()
            .is_some_and(|edit| edit.id != id || edit.cell.is_some())
        {
            self.end_text_edit();
        }
        self.history.close_burst();
        self.selection = vec![id];
        self.table_edit = None;
        self.text_edit = Some(TextEdit {
            id,
            cell: None,
            caret,
            anchor,
            marked: None,
            goal_x: None,
        });
    }

    /// Enters editing of the text of a table cell, which becomes the cell
    /// selection.
    pub fn begin_cell_edit(
        &mut self,
        id: ElementId,
        (row, column): (usize, usize),
        anchor: usize,
        caret: usize,
    ) {
        self.end_text_edit();
        self.history.close_burst();
        self.selection = vec![id];
        self.table_edit = Some(TableEdit::cell(id, row, column));
        self.text_edit = Some(TextEdit {
            id,
            cell: Some((row, column)),
            caret,
            anchor,
            marked: None,
            goal_x: None,
        });
    }

    /// Leaves text editing. A box left empty is deleted; a table cell keeps
    /// its cell selected.
    pub fn end_text_edit(&mut self) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        self.history.close_burst();
        if edit.cell.is_some() {
            return;
        }
        let empty = self
            .presentation
            .element(edit.id)
            .and_then(Element::as_text)
            .is_some_and(|text| text.content.is_empty());
        if empty {
            self.commit(
                "Delete text",
                Operation::RemoveElement { id: edit.id },
                vec![],
            );
        }
    }

    /// Content of the text being edited.
    pub fn edit_content(&self) -> Option<&str> {
        let edit = self.text_edit.as_ref()?;
        let element = self.presentation.element(edit.id)?;
        match edit.cell {
            Some((row, column)) => Some(&element.as_table()?.cell(row, column)?.content),
            None => Some(&element.as_text()?.content),
        }
    }

    /// The layout of the text being edited and the frame of its box on the
    /// slide: the text element, or the inside of the table cell.
    pub fn edit_box(&mut self) -> Option<(Arc<TextLayout>, Frame)> {
        let edit = self.text_edit.clone()?;
        match edit.cell {
            Some((row, column)) => self.cell_box(edit.id, row, column),
            None => Some((self.layout_of(edit.id)?, self.frame_of(edit.id)?)),
        }
    }

    /// The layout of the text of a table cell and the frame of its box on
    /// the slide.
    pub fn cell_box(
        &mut self,
        id: ElementId,
        row: usize,
        column: usize,
    ) -> Option<(Arc<TextLayout>, Frame)> {
        let frame = self.presentation.element(id)?.frame;
        let view = self.tables.get(&self.presentation, id)?;
        let cell = view.layout.cell(row, column)?;
        let layout = view.text(row, column)?.clone();
        Some((layout, crate::table::place(&frame, &cell.inner)))
    }

    /// Replaces `range` of the edited text, as typing: the change joins the
    /// current typing burst. The caret lands after the new text.
    pub fn type_text(&mut self, range: Range<usize>, text: &str) {
        let Some(edit) = &self.text_edit else {
            return;
        };
        let id = edit.id;
        let operation = match edit.cell {
            Some((row, column)) => Operation::ReplaceCellText {
                id,
                row,
                column,
                range: range.clone(),
                text: text.into(),
            },
            None => Operation::ReplaceText {
                id,
                range: range.clone(),
                text: text.into(),
            },
        };
        match self.presentation.apply(operation) {
            Ok(inverse) => {
                self.history.record_typing(id, inverse, Instant::now());
                if let Some(edit) = &mut self.text_edit {
                    edit.collapse(range.start + text.len());
                    edit.marked = None;
                }
            }
            Err(error) => eprintln!("sliderino: typing failed: {error}"),
        }
    }

    /// Handles a key while a text box is being edited. Returns true when the
    /// key was used; other keys reach the input handler as text.
    fn on_text_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.clone() else {
            return false;
        };
        let Some(content) = self.edit_content().map(str::to_string) else {
            return false;
        };
        let modifiers = &keystroke.modifiers;
        let (shift, word) = (modifiers.shift, modifiers.control || modifiers.alt);
        let selection = edit.selection();

        if self.shortcuts.undo.matches(keystroke) {
            self.undo();
            return true;
        }
        if self
            .shortcuts
            .redo
            .iter()
            .any(|chord| chord.matches(keystroke))
        {
            self.redo();
            return true;
        }
        if modifiers.control && !modifiers.alt {
            match keystroke.key.as_str() {
                "a" => {
                    self.move_caret(0, false);
                    self.move_caret(content.len(), true);
                }
                "c" | "x" if !selection.is_empty() => {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        content[selection.clone()].to_string(),
                    ));
                    if keystroke.key == "x" {
                        self.history.close_burst();
                        self.type_text(selection.clone(), "");
                        self.history.close_burst();
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.history.close_burst();
                        self.type_text(selection.clone(), &text);
                        self.history.close_burst();
                    }
                }
                "left" | "right" | "backspace" | "delete" | "home" | "end" => {}
                _ => return false,
            }
            if matches!(keystroke.key.as_str(), "a" | "c" | "x" | "v") {
                return true;
            }
        }

        match keystroke.key.as_str() {
            "escape" => self.end_text_edit(),
            "tab" if edit.cell.is_some() && !modifiers.control && !modifiers.alt => {
                self.tab_cell(shift);
            }
            "enter" => self.type_text(selection, "\n"),
            "backspace" | "delete" => {
                let range = if !selection.is_empty() {
                    self.history.close_burst();
                    selection
                } else if keystroke.key == "backspace" {
                    let start = if word {
                        word_start(&content, edit.caret)
                    } else {
                        previous_boundary(&content, edit.caret)
                    };
                    start..edit.caret
                } else {
                    let end = if word {
                        word_end(&content, edit.caret)
                    } else {
                        next_boundary(&content, edit.caret)
                    };
                    edit.caret..end
                };
                if !range.is_empty() {
                    self.type_text(range, "");
                }
            }
            "left" | "right" => {
                let left = keystroke.key == "left";
                let target = if !shift && !selection.is_empty() && !word {
                    if left { selection.start } else { selection.end }
                } else if left {
                    if word {
                        word_start(&content, edit.caret)
                    } else {
                        previous_boundary(&content, edit.caret)
                    }
                } else if word {
                    word_end(&content, edit.caret)
                } else {
                    next_boundary(&content, edit.caret)
                };
                self.move_caret(target, shift);
            }
            "up" | "down" => self.move_vertically(keystroke.key == "down", shift),
            "home" | "end" => {
                let target = if word {
                    if keystroke.key == "home" {
                        0
                    } else {
                        content.len()
                    }
                } else {
                    self.line_edge(keystroke.key == "end")
                };
                self.move_caret(target, shift);
            }
            _ => return false,
        }
        true
    }

    /// Moves the caret, extending the selection when `extend` is set. Ends
    /// the typing burst.
    pub fn move_caret(&mut self, index: usize, extend: bool) {
        self.history.close_burst();
        if let Some(edit) = &mut self.text_edit {
            edit.caret = index;
            if !extend {
                edit.anchor = index;
            }
            edit.goal_x = None;
            edit.marked = None;
        }
    }

    fn move_vertically(&mut self, down: bool, extend: bool) {
        let Some(edit) = self.text_edit.clone() else {
            return;
        };
        let Some((layout, _)) = self.edit_box() else {
            return;
        };
        let line = layout.line_of(edit.caret);
        let x = edit.goal_x.unwrap_or_else(|| layout.caret(edit.caret).left);
        let target = match (down, line) {
            (false, 0) => 0,
            (false, line) => layout.index_on_line(line - 1, x),
            (true, line) if line + 1 >= layout.lines.len() => {
                self.edit_content().map_or(0, str::len)
            }
            (true, line) => layout.index_on_line(line + 1, x),
        };
        self.move_caret(target, extend);
        if let Some(edit) = &mut self.text_edit {
            edit.goal_x = Some(x);
        }
    }

    /// Start or end of the caret's line; the end of a wrapped line stays
    /// before its trailing space so the caret remains on that line.
    fn line_edge(&mut self, end: bool) -> usize {
        let Some(edit) = self.text_edit.clone() else {
            return 0;
        };
        let Some((layout, _)) = self.edit_box() else {
            return edit.caret;
        };
        let line = &layout.lines[layout.line_of(edit.caret)];
        match (end, line.ends_paragraph) {
            (false, _) => line.range.start,
            (true, true) => line.range.end,
            (true, false) => {
                let content = self.edit_content().unwrap_or_default();
                previous_boundary(content, line.range.end).max(line.range.start)
            }
        }
    }

    /// Selects the word around a byte index of the edited text.
    pub fn select_word(&mut self, index: usize) {
        let Some(content) = self.edit_content() else {
            return;
        };
        let start = word_start(content, next_boundary(content, index).min(content.len()));
        let end = word_end(content, start);
        self.move_caret(start, false);
        self.move_caret(end, true);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "escape"
            && matches!(
                self.drag,
                Some(
                    Drag::Move { .. }
                        | Drag::Resize { .. }
                        | Drag::Create { .. }
                        | Drag::Draw { .. }
                        | Drag::LineEnd { .. }
                        | Drag::DrawTable { .. }
                        | Drag::MoveCells { .. }
                )
            )
        {
            self.cancel_drag();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.on_file_key(keystroke, window, cx) {
            cx.stop_propagation();
            return;
        }
        // Keys typed into the inspector's fields belong to them.
        let focused = self.focus.is_focused(window);
        if focused && self.text_edit.is_some() {
            if self.on_text_key(keystroke, cx) {
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if focused && self.table_edit.is_some() && self.on_table_key(keystroke, cx) {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if focused && self.on_editor_key(keystroke) {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if focused
            && keystroke.key == "v"
            && keystroke.modifiers.secondary()
            && !keystroke.modifiers.shift
            && (self.paste_image(window, cx) || self.paste_table(cx))
        {
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if !self.viewport.contains(&window.mouse_position()) {
            return;
        }
        if self.shortcuts.hand_hold.matches(keystroke) {
            if !event.is_held && !self.hand_key_held {
                self.hand_key_held = true;
                cx.notify();
            }
        } else if self.shortcuts.zoom_to_fit.matches(keystroke) {
            self.zoom_to_fit();
            cx.notify();
        } else if self.shortcuts.zoom_to_100.matches(keystroke) {
            if let Some(camera) = &mut self.camera {
                camera.set_zoom(1.);
            }
            cx.notify();
        } else {
            return;
        }
        cx.stop_propagation();
    }

    /// Tab and Shift-Tab: the next or previous cell while table cells are
    /// selected or edited; otherwise the key moves the focus as usual.
    fn on_tab_action(&mut self, back: bool, cx: &mut Context<Self>) {
        if self.table_edit.is_some() {
            self.tab_cell(back);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Undo, redo and the keys that act on the selection.
    fn on_editor_key(&mut self, keystroke: &Keystroke) -> bool {
        if self.shortcuts.undo.matches(keystroke) {
            self.undo();
            return true;
        }
        if self
            .shortcuts
            .redo
            .iter()
            .any(|chord| chord.matches(keystroke))
        {
            self.redo();
            return true;
        }
        if self.selection.is_empty() {
            return false;
        }
        if self.shortcuts.group.matches(keystroke) {
            self.group_selection();
            return true;
        }
        if self.shortcuts.ungroup.matches(keystroke) {
            self.ungroup_selection();
            return true;
        }
        let plain = !keystroke.modifiers.modified();
        match keystroke.key.as_str() {
            "backspace" | "delete" if plain => self.delete_selection(),
            "enter" if plain => return self.enter_selection(),
            "escape" => self.select_parent(),
            _ => return false,
        }
        true
    }

    /// Selected roots that are not locked: the ones group, ungroup and
    /// delete act on.
    fn editable_roots(&self) -> Vec<ElementId> {
        self.selection_roots()
            .into_iter()
            .filter(|id| !self.presentation.is_locked(*id))
            .collect()
    }

    /// Puts the selection in a new group and selects it.
    pub fn group_selection(&mut self) {
        let ids = self.editable_roots();
        if ids.is_empty() {
            return;
        }
        self.end_text_edit();
        let group = self.presentation.new_element_id();
        match self.presentation.group_operations(group, &ids) {
            Ok(operations) => {
                self.commit_pruning("Group", operations, vec![group]);
            }
            Err(error) => eprintln!("sliderino: Group failed: {error}"),
        }
    }

    /// Puts the children of each selected group in its place and selects
    /// them.
    pub fn ungroup_selection(&mut self) {
        let mut operations = Vec::new();
        let mut select = Vec::new();
        for id in self.editable_roots() {
            let Some(group) = self.presentation.element(id).and_then(Element::as_group) else {
                continue;
            };
            select.extend(group.children.iter().map(|child| child.id));
            match self.presentation.ungroup_operations(id) {
                Ok(ungroup) => operations.push(Operation::Batch(ungroup)),
                Err(error) => eprintln!("sliderino: Ungroup failed: {error}"),
            }
        }
        if !operations.is_empty() {
            self.end_text_edit();
            self.commit_pruning("Ungroup", operations, select);
        }
    }

    /// Removes the selection, and the groups it leaves empty.
    pub fn delete_selection(&mut self) {
        let operations: Vec<Operation> = self
            .editable_roots()
            .into_iter()
            .map(|id| Operation::RemoveElement { id })
            .collect();
        if !operations.is_empty() {
            self.commit_pruning("Delete", operations, vec![]);
        }
    }

    /// Enter: selects the children of a selected group, or edits a selected
    /// text. Returns false when the key does nothing.
    fn enter_selection(&mut self) -> bool {
        let Some(id) = self.single_selection() else {
            return false;
        };
        if self.presentation.is_locked(id) {
            return false;
        }
        let Some(element) = self.presentation.element(id) else {
            return false;
        };
        if let Some(group) = element.as_group() {
            self.selection = group.children.iter().map(|child| child.id).collect();
            return true;
        }
        if let Some(table) = element.as_table() {
            let len = table.cell(0, 0).map_or(0, |cell| cell.content.len());
            self.begin_cell_edit(id, (0, 0), 0, len);
            return true;
        }
        match element.as_text().map(|text| text.content.len()) {
            Some(len) => {
                self.begin_text_edit(id, 0, len);
                true
            }
            None => false,
        }
    }

    /// Escape: selects the group holding the selection, or clears the
    /// selection at the top level.
    pub fn select_parent(&mut self) {
        let parent = self
            .selection
            .first()
            .and_then(|id| self.presentation.locate(*id))
            .and_then(|location| location.parent);
        self.selection = parent.into_iter().collect();
        self.collapse_slide_selection();
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.hand_key_held && self.shortcuts.hand_hold.matches_key(&event.keystroke) {
            self.hand_key_held = false;
            cx.notify();
        }
    }

    /// Converts a window position to slide units.
    pub fn to_slide(&self, position: Point<Pixels>) -> Option<SlidePoint> {
        let camera = self.camera?;
        let rect = camera.slide_rect(self.presentation.size, self.viewport);
        let offset = position - rect.origin;
        Some(point(
            f32::from(offset.x) / camera.zoom,
            f32::from(offset.y) / camera.zoom,
        ))
    }

    /// The part of the slide visible in the canvas, in window coordinates.
    pub fn slide_bounds(&self) -> Option<Bounds<Pixels>> {
        let camera = self.camera?;
        let slide = camera.slide_rect(self.presentation.size, self.viewport);
        let visible = slide.intersect(&self.viewport);
        (!visible.is_empty()).then_some(visible)
    }

    /// Converts slide units to a window position.
    pub fn to_window(&self, x: f32, y: f32) -> Option<Point<Pixels>> {
        let camera = self.camera?;
        let rect = camera.slide_rect(self.presentation.size, self.viewport);
        Some(rect.origin + point(px(x * camera.zoom), px(y * camera.zoom)))
    }
}

/// Largest char boundary of `text` at or before `index`.
pub fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub fn previous_boundary(text: &str, index: usize) -> usize {
    text[..index].char_indices().last().map_or(0, |(ix, _)| ix)
}

pub fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index, |ch| index + ch.len_utf8())
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Start of the word before `index`, skipping spaces first.
pub fn word_start(text: &str, index: usize) -> usize {
    let before = &text[..index];
    let trimmed = before.trim_end_matches(|ch: char| !is_word(ch));
    trimmed
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_word(*ch))
        .last()
        .map_or(trimmed.len(), |(ix, _)| ix)
}

/// End of the word after `index`, skipping spaces first.
pub fn word_end(text: &str, index: usize) -> usize {
    let after = &text[index..];
    let skipped = after.len() - after.trim_start_matches(|ch: char| !is_word(ch)).len();
    let word = after[skipped..]
        .char_indices()
        .find(|(_, ch)| !is_word(*ch))
        .map_or(after.len() - skipped, |(ix, _)| ix);
    index + skipped + word
}

/// What the editor asks of the window around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// Show the Home screen.
    GoHome,
    /// Show a new presentation.
    New,
    /// Ask for a file and show it.
    Open,
}

impl EventEmitter<EditorEvent> for EditorView {}

impl EditorView {
    /// Whether the presentation changed since it was opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.presentation.revision() != self.saved_revision
    }

    /// Records that the presentation, as it is now, is saved in `path`.
    pub fn mark_saved(&mut self, path: PathBuf) {
        self.file = Some(path);
        self.saved_revision = self.presentation.revision();
    }

    /// The name of the presentation: its file name without the extension,
    /// or "Untitled".
    pub fn title(&self) -> String {
        self.file
            .as_deref()
            .and_then(|path| path.file_stem())
            .map_or_else(|| "Untitled".into(), |stem| stem.to_string_lossy().into())
    }

    /// Saves the presentation to its file. Without a file, or with
    /// `save_as`, asks for one first. The task gives true when the file is
    /// written; errors show as a notification.
    /// Asks where to write a PowerPoint file and exports the presentation
    /// to it, off the UI thread. Tells the author what the deck shows
    /// differently.
    pub fn export_pptx(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self
            .file
            .as_deref()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        let suggested = format!("{}.pptx", self.title());
        let presentation = self.presentation.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(chosen) =
                cx.update(|_, cx| cx.prompt_for_new_path(&directory, Some(&suggested)))
            else {
                return;
            };
            let path = match chosen.await {
                Ok(Ok(Some(path))) if path.extension().is_some() => path,
                Ok(Ok(Some(path))) => path.with_extension("pptx"),
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    this.update_in(cx, |_, window, cx| {
                        crate::ui::show_error(format!("Cannot export: {error}"), window, cx);
                    })
                    .ok();
                    return;
                }
            };
            let target = path.clone();
            let written = cx
                .background_spawn(async move {
                    let export =
                        crate::pptx::export(&presentation, &crate::pptx::Options::default())?;
                    crate::pptx::save(&export, &target)?;
                    Ok::<_, crate::pptx::ExportError>(export.warnings)
                })
                .await;
            this.update_in(cx, |_, window, cx| match written {
                Ok(warnings) if warnings.is_empty() => {}
                Ok(warnings) => {
                    let mut message = format!(
                        "Exported {}. PowerPoint shows {} thing(s) differently:",
                        path.display(),
                        warnings.len()
                    );
                    for warning in warnings.iter().take(5) {
                        message.push_str(&format!("\n• {warning}"));
                    }
                    if warnings.len() > 5 {
                        message.push_str(&format!("\n• and {} more", warnings.len() - 5));
                    }
                    crate::ui::show_warning(message, window, cx);
                }
                Err(error) => {
                    let message = format!("Cannot export {}: {error}", path.display());
                    crate::ui::show_error(message, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn save(
        &mut self,
        save_as: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.history.close_burst();
        let file = self.file.clone().filter(|_| !save_as);
        let directory = self
            .file
            .as_deref()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        let suggested = format!("{}.{}", self.title(), crate::file::EXTENSION);
        cx.spawn_in(window, async move |this, cx| {
            let path = match file {
                Some(path) => path,
                None => {
                    let Ok(chosen) =
                        cx.update(|_, cx| cx.prompt_for_new_path(&directory, Some(&suggested)))
                    else {
                        return false;
                    };
                    match chosen.await {
                        Ok(Ok(Some(path))) => crate::file::with_extension(&path),
                        Ok(Ok(None)) | Err(_) => return false,
                        Ok(Err(error)) => {
                            this.update_in(cx, |_, window, cx| {
                                crate::ui::show_error(format!("Cannot save: {error}"), window, cx);
                            })
                            .ok();
                            return false;
                        }
                    }
                }
            };
            let Ok((presentation, revision)) = this.read_with(cx, |editor, _| {
                (editor.presentation.clone(), editor.presentation.revision())
            }) else {
                return false;
            };
            let target = path.clone();
            let written = cx
                .background_spawn(async move { crate::file::save(&presentation, &target) })
                .await;
            this.update_in(cx, |editor, window, cx| match written {
                Ok(()) => {
                    editor.file = Some(path);
                    editor.saved_revision = revision;
                    cx.notify();
                    true
                }
                Err(error) => {
                    let message = format!("Cannot save {}: {error}", path.display());
                    crate::ui::show_error(message, window, cx);
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    /// Handles the file shortcuts: save, save as, new and open. Returns
    /// true when the key was one of them.
    fn on_file_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.shortcuts.save.matches(keystroke) {
            self.save(false, window, cx).detach();
        } else if self.shortcuts.save_as.matches(keystroke) {
            self.save(true, window, cx).detach();
        } else if self.shortcuts.new.matches(keystroke) {
            cx.emit(EditorEvent::New);
        } else if self.shortcuts.open.matches(keystroke) {
            cx.emit(EditorEvent::Open);
        } else {
            return false;
        }
        true
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("render");
        self.sync_inspector(window, cx);
        self.sync_shape_inspector(window, cx);
        self.sync_table_inspector(window, cx);
        self.sync_preview();
        let scene = self.canvas_scene(cx);
        if self.preview.borrow().animating() {
            window.request_animation_frame();
        }
        let problems = crate::ui::properties_panel::diagnostics(self);
        let root = v_flex()
            .id("editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextCell, _, cx| this.on_tab_action(false, cx)))
            .on_action(cx.listener(|this, _: &PreviousCell, _, cx| this.on_tab_action(true, cx)))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .size_full()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .child(top_bar(self, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(slides_panel(self, cx))
                    .child(canvas(self, scene, cx))
                    .child(properties_panel(self, problems, cx)),
            );
        // The canvas and the thumbnails asked for pictures; load them now.
        self.load_pictures(cx);
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_an_element_reuses_its_layout() {
        let mut presentation = crate::document::tests::with_inter();
        let id = crate::document::tests::add_text(
            &mut presentation,
            "Hello",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let mut cache = LayoutCache::default();
        let before = cache.get(&presentation, id).unwrap();
        let frame = Frame {
            x: 300.,
            y: 200.,
            ..presentation.element(id).unwrap().frame
        };
        presentation
            .apply(Operation::SetFrame { id, frame })
            .unwrap();
        let after = cache.get(&presentation, id).unwrap();
        assert!(Arc::ptr_eq(&before, &after), "the move reused the layout");

        let wider = Frame {
            width: frame.width + 100.,
            ..frame
        };
        let resized = cache
            .get_for(&presentation, id, wider, TextSizing::AutoHeight)
            .unwrap();
        assert!(!Arc::ptr_eq(&after, &resized), "a new size lays out again");
        // The document's layout is still cached next to the preview's.
        let again = cache.get(&presentation, id).unwrap();
        assert!(Arc::ptr_eq(&after, &again));
    }

    #[test]
    fn word_boundaries_skip_spaces_and_punctuation() {
        let text = "Hello, big world";
        assert_eq!(word_start(text, text.len()), 11);
        assert_eq!(word_start(text, 11), 7);
        assert_eq!(word_start(text, 7), 0);
        assert_eq!(word_end(text, 0), 5);
        assert_eq!(word_end(text, 5), 10);
        assert_eq!(word_end(text, 10), text.len());
    }

    #[test]
    fn char_boundaries_step_over_multibyte_characters() {
        let text = "aé€";
        assert_eq!(next_boundary(text, 1), 3);
        assert_eq!(previous_boundary(text, 6), 3);
        assert_eq!(floor_boundary(text, 5), 3);
        assert_eq!(next_boundary(text, text.len()), text.len());
    }
}
