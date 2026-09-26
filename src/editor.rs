//! The editor window: title bar over the slides panel, canvas and inspector.
//!
//! `EditorView` owns the presentation and is the only place the UI changes
//! it: every change is an [`Operation`] applied through
//! [`EditorView::commit`] (or typed into a text box), so it lands in the
//! shared undo history.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::{
    Bounds, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, KeyUpEvent, Keystroke, MouseButton, ParentElement, Pixels, Point, Render, Styled,
    Subscription, Window, point, px,
};

use crate::camera::Camera;
use crate::document::{
    Element, ElementId, ElementKind, Frame, Operation, Presentation, Slide, SlideId, TextElement,
    TextSizing, TextStyle,
};
use crate::fonts::FontRegistry;
use crate::history::History;
use crate::shortcuts::Shortcuts;
use crate::snap::{Guide, Handle};
use crate::text_layout::TextLayout;
use crate::theme;
use crate::ui::canvas::canvas;
use crate::ui::inspector::Inspector;
use crate::ui::properties_panel::properties_panel;
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
    Image,
    Component,
}

/// A point on the slide, in slide units.
pub type SlidePoint = Point<f32>;

/// A text box being edited in place.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub id: ElementId,
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

/// A pointer drag on the canvas, started with the left button.
#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    /// Drawing a new text box with the text tool.
    Create {
        start: SlidePoint,
        current: SlidePoint,
    },
    Move {
        id: ElementId,
        grab: SlidePoint,
        origin: Frame,
        moved: bool,
    },
    Resize {
        id: ElementId,
        handle: Handle,
        grab: SlidePoint,
        origin: Frame,
        sizing: TextSizing,
    },
    /// Extending the text selection of the box being edited.
    SelectText { id: ElementId },
}

/// Text layouts of the presentation's elements, recomputed when the text or
/// its frame changes.
#[derive(Default)]
pub struct LayoutCache {
    entries: HashMap<ElementId, (TextElement, Frame, Arc<TextLayout>)>,
}

impl LayoutCache {
    pub fn get(&mut self, presentation: &Presentation, id: ElementId) -> Option<Arc<TextLayout>> {
        let element = presentation.element(id)?;
        let text = element.as_text()?;
        if let Some((cached_text, frame, layout)) = self.entries.get(&id)
            && cached_text == text
            && *frame == element.frame
        {
            return Some(layout.clone());
        }
        let layout = Arc::new(presentation.text_layout(id)?);
        self.entries
            .insert(id, (text.clone(), element.frame, layout.clone()));
        Some(layout)
    }
}

pub struct EditorView {
    /// Slide shown on the canvas.
    pub current_slide: SlideId,
    /// Tab of the left panel: 0 = Slides, 1 = Components.
    pub library_tab: usize,
    /// Tab of the right panel: 0 = Design, 1 = Notes, 2 = History.
    pub inspector_tab: usize,
    /// Tool picked in the palette; see [`EditorView::effective_tool`].
    pub active_tool: Tool,
    pub presentation: Presentation,
    pub history: History,
    /// Selected element, always on the current slide.
    pub selection: Option<ElementId>,
    pub text_edit: Option<TextEdit>,
    pub drag: Option<Drag>,
    /// Snap guides of the drag in progress.
    pub guides: Vec<Guide>,
    pub layouts: LayoutCache,
    pub fonts: FontRegistry,
    pub inspector: Inspector,
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
        Self {
            current_slide: presentation.slides[0].id,
            library_tab: 0,
            inspector_tab: 0,
            active_tool: Tool::Move,
            presentation,
            history,
            selection: None,
            text_edit: None,
            drag: None,
            guides: Vec::new(),
            layouts: LayoutCache::default(),
            fonts: FontRegistry::default(),
            inspector: Inspector::new(window, cx),
            shortcuts: Shortcuts::default(),
            camera: None,
            viewport: Bounds::default(),
            pan_drag: None,
            hand_key_held: false,
            focus: cx.focus_handle(),
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

    /// Applies an edit and records it as one undo step, leaving `select`
    /// selected. Returns false (and changes nothing) when the edit fails.
    pub fn commit(&mut self, label: &str, operation: Operation, select: Option<ElementId>) -> bool {
        let before = self.selection;
        match self.presentation.apply(operation) {
            Ok(inverse) => {
                self.selection = select;
                self.history.record(label, inverse, before, select);
                true
            }
            Err(error) => {
                eprintln!("sliderino: {label} failed: {error}");
                false
            }
        }
    }

    pub fn undo(&mut self) {
        let index = self.presentation.index_of(self.current_slide);
        if let Some(result) = self.history.undo(&mut self.presentation) {
            self.after_history(result, index);
        }
    }

    pub fn redo(&mut self) {
        let index = self.presentation.index_of(self.current_slide);
        if let Some(result) = self.history.redo(&mut self.presentation) {
            self.after_history(result, index);
        }
    }

    /// Restores the selection after undo or redo and shows the slide it is on.
    fn after_history(
        &mut self,
        result: Result<Option<ElementId>, crate::document::ApplyError>,
        slide_index: Option<usize>,
    ) {
        let selection = match result {
            Ok(selection) => selection,
            Err(error) => {
                eprintln!("sliderino: undo failed: {error}");
                return;
            }
        };
        self.selection = selection.filter(|id| self.presentation.element(*id).is_some());
        if let Some((slide, _)) = self.selection.and_then(|id| self.presentation.locate(id)) {
            self.current_slide = slide;
        }
        if self.presentation.slide(self.current_slide).is_none() {
            let last = self.presentation.slides.len() - 1;
            self.current_slide = self.presentation.slides[slide_index.unwrap_or(0).min(last)].id;
        }
        if let Some(edit) = &mut self.text_edit {
            let content = self
                .presentation
                .element(edit.id)
                .and_then(Element::as_text)
                .map(|text| text.content.as_str());
            match content {
                Some(content) if self.selection == Some(edit.id) => {
                    let clamp = |index: usize| floor_boundary(content, index.min(content.len()));
                    edit.caret = clamp(edit.caret);
                    edit.anchor = clamp(edit.anchor);
                    edit.marked = None;
                }
                _ => self.text_edit = None,
            }
        }
    }

    /// Shows another slide, leaving any text being edited.
    pub fn select_slide(&mut self, id: SlideId) {
        self.end_text_edit();
        self.selection = None;
        self.current_slide = id;
    }

    /// Adds an empty slide after the current one and shows it.
    pub fn add_slide(&mut self) {
        self.end_text_edit();
        let id = self.presentation.new_slide_id();
        let index = self
            .presentation
            .index_of(self.current_slide)
            .map_or(usize::MAX, |ix| ix + 1);
        let slide = Slide::new(id);
        if self.commit("Add slide", Operation::AddSlide { index, slide }, None) {
            self.current_slide = id;
        }
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
            index: usize::MAX,
            element: Element {
                id,
                frame,
                kind: ElementKind::Text(TextElement {
                    content: String::new(),
                    style,
                    sizing,
                }),
            },
        });
        self.end_text_edit();
        if self.commit("Create text", Operation::Batch(operations), Some(id)) {
            self.begin_text_edit(id, 0, 0);
        }
    }

    /// Enters text editing with the given selection.
    pub fn begin_text_edit(&mut self, id: ElementId, anchor: usize, caret: usize) {
        if self.text_edit.as_ref().is_some_and(|edit| edit.id != id) {
            self.end_text_edit();
        }
        self.history.close_burst();
        self.selection = Some(id);
        self.text_edit = Some(TextEdit {
            id,
            caret,
            anchor,
            marked: None,
            goal_x: None,
        });
    }

    /// Leaves text editing. A box left empty is deleted.
    pub fn end_text_edit(&mut self) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        self.history.close_burst();
        let empty = self
            .presentation
            .element(edit.id)
            .and_then(Element::as_text)
            .is_some_and(|text| text.content.is_empty());
        if empty {
            self.commit(
                "Delete text",
                Operation::RemoveElement { id: edit.id },
                None,
            );
        }
    }

    /// Content of the text being edited.
    pub fn edit_content(&self) -> Option<&str> {
        let edit = self.text_edit.as_ref()?;
        let text = self.presentation.element(edit.id)?.as_text()?;
        Some(&text.content)
    }

    /// Replaces `range` of the edited text, as typing: the change joins the
    /// current typing burst. The caret lands after the new text.
    pub fn type_text(&mut self, range: Range<usize>, text: &str) {
        let Some(edit) = &self.text_edit else {
            return;
        };
        let id = edit.id;
        let operation = Operation::ReplaceText {
            id,
            range: range.clone(),
            text: text.into(),
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
        let Some(layout) = self.layout_of(edit.id) else {
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
        let Some(layout) = self.layout_of(edit.id) else {
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
        // Keys typed into the inspector's fields belong to them.
        let focused = self.focus.is_focused(window);
        if focused && self.text_edit.is_some() {
            if self.on_text_key(keystroke, cx) {
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if focused && self.on_editor_key(keystroke) {
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

    /// Undo, redo and the keys that act on the selected element.
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
        let Some(id) = self.selection else {
            return false;
        };
        let plain = !keystroke.modifiers.modified();
        match keystroke.key.as_str() {
            "backspace" | "delete" if plain => {
                self.commit("Delete", Operation::RemoveElement { id }, None);
            }
            "enter" if plain => {
                let len = self
                    .presentation
                    .element(id)
                    .and_then(Element::as_text)
                    .map(|text| text.content.len());
                match len {
                    Some(len) => self.begin_text_edit(id, 0, len),
                    None => return false,
                }
            }
            "escape" => self.selection = None,
            _ => return false,
        }
        true
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

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inspector(window, cx);
        let scene = self.canvas_scene(cx);
        v_flex()
            .id("editor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .size_full()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .child(top_bar(self))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(slides_panel(self, cx))
                    .child(canvas(self, scene, cx))
                    .child(properties_panel(self, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
