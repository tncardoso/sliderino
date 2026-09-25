//! Center area: an empty 16:9 slide on the canvas ground and the tool palette.
//!
//! The slide is a placeholder until the renderer exists.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{Selectable as _, h_flex};
use gpui_kit::{
    AnyElement, BoxShadow, Context, IntoElement, ParentElement, Styled, div, hsla, point, px,
};

use crate::editor::{EditorView, Tool};
use crate::theme;

pub fn canvas(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .px(px(48.))
        .pb(px(56.))
        .bg(theme::canvas())
        .child(
            // 800×450 at 100% zoom; shrinks to fit a narrow window.
            div()
                .w_full()
                .max_w(px(800.))
                .aspect_ratio(16. / 9.)
                .bg(theme::background())
                .shadow(vec![shadow(1., 2., 0., 0.06), shadow(8., 24., 0., 0.06)]),
        )
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
    let mut items: Vec<AnyElement> = Vec::new();
    for group in Tool::GROUPS {
        if !items.is_empty() {
            items.push(divider().into_any_element());
        }
        for tool in group {
            items.push(tool_button(*tool, editor.active_tool == *tool, cx).into_any_element());
        }
    }

    h_flex()
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
