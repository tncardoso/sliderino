//! Platform text input for the text box being edited on the canvas: typed
//! characters, input method composition and the caret position the input
//! method needs. The platform speaks UTF-16 offsets; the document uses UTF-8
//! byte offsets.

use std::ops::Range;

use gpui_kit::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window, px};

use crate::editor::EditorView;

fn to_utf16(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].encode_utf16().count()
}

fn to_utf8(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (ix, ch) in text.char_indices() {
        if units >= index {
            return ix;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    to_utf16(text, range.start)..to_utf16(text, range.end)
}

fn range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    to_utf8(text, range.start)..to_utf8(text, range.end)
}

impl EditorView {
    /// The range a platform edit replaces: the given one, else the text being
    /// composed, else the selection. Typing over selected table cells starts
    /// editing the first one, its text selected.
    fn input_range(&mut self, range_utf16: Option<Range<usize>>) -> Option<Range<usize>> {
        if !self.type_over_cell() {
            return None;
        }
        let content = self.edit_content()?;
        let edit = self.text_edit.as_ref()?;
        Some(
            range_utf16
                .map(|range| range_to_utf8(content, &range))
                .or_else(|| edit.marked.clone())
                .unwrap_or_else(|| edit.selection()),
        )
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let content = self.edit_content()?;
        let range = range_to_utf8(content, &range_utf16);
        adjusted_range.replace(range_to_utf16(content, &range));
        Some(content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let content = self.edit_content()?;
        let edit = self.text_edit.as_ref()?;
        Some(UTF16Selection {
            range: range_to_utf16(content, &edit.selection()),
            reversed: edit.caret < edit.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let content = self.edit_content()?;
        let marked = self.text_edit.as_ref()?.marked.as_ref()?;
        Some(range_to_utf16(content, marked))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.text_edit {
            edit.marked = None;
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.input_range(range_utf16) else {
            return;
        };
        self.type_text(range, text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.input_range(range_utf16) else {
            return;
        };
        let start = range.start;
        self.type_text(range, new_text);
        if let Some(edit) = &mut self.text_edit {
            edit.marked = (!new_text.is_empty()).then(|| start..start + new_text.len());
            if let Some(selected) = new_selected_range_utf16 {
                let selected = range_to_utf8(new_text, &selected);
                edit.anchor = start + selected.start;
                edit.caret = start + selected.end;
            }
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = range_to_utf8(self.edit_content()?, &range_utf16);
        let (layout, frame) = self.edit_box()?;
        let caret = layout.caret(range.start);
        let zoom = self.camera?.zoom;
        let (x, y) = frame.to_slide(caret.left, caret.top);
        let origin = self.to_window(x, y)?;
        Some(Bounds {
            origin,
            size: gpui_kit::size(px(1.), px((caret.bottom - caret.top) * zoom)),
        })
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let (layout, frame) = self.edit_box()?;
        let at = self.to_slide(position)?;
        let (x, y) = frame.to_local(at.x, at.y);
        let index = layout.index_at(x, y);
        Some(to_utf16(self.edit_content()?, index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_utf16_and_utf8_offsets() {
        let text = "a€𝄞b";
        assert_eq!(to_utf16(text, 4), 2, "€ is one UTF-16 unit");
        assert_eq!(to_utf16(text, 8), 4, "𝄞 is two UTF-16 units");
        assert_eq!(to_utf8(text, 2), 4);
        assert_eq!(to_utf8(text, 4), 8);
        assert_eq!(to_utf8(text, 99), text.len());
    }
}
