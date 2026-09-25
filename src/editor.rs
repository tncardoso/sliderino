//! The editor window: title bar over the slides panel, canvas and inspector.
//!
//! Only UI state lives here; there is no document yet.

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, px};

use crate::theme;
use crate::ui::canvas::canvas;
use crate::ui::properties_panel::properties_panel;
use crate::ui::slides_panel::slides_panel;
use crate::ui::top_bar::top_bar;

#[derive(Clone, Copy, PartialEq, Eq)]
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

pub struct EditorView {
    pub active_slide: usize,
    /// Tab of the left panel: 0 = Slides, 1 = Components.
    pub library_tab: usize,
    /// Tab of the right panel: 0 = Design, 1 = Notes, 2 = History.
    pub inspector_tab: usize,
    pub active_tool: Tool,
}

impl EditorView {
    pub fn new() -> Self {
        Self {
            active_slide: 1,
            library_tab: 0,
            inspector_tab: 0,
            active_tool: Tool::Move,
        }
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .child(top_bar())
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(slides_panel(self, cx))
                    .child(canvas(self, cx))
                    .child(properties_panel(self, cx)),
            )
    }
}
