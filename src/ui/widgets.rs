//! Small building blocks shared by the side panels.

use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{Div, FontWeight, IntoElement, ParentElement, SharedString, Styled, div, px};

use crate::theme;

/// A panel section: horizontal inset of 16px and a bottom border.
pub fn section() -> Div {
    v_flex()
        .gap(px(10.))
        .px(px(16.))
        .pt(px(14.))
        .pb(px(16.))
        .border_b_1()
        .border_color(theme::border())
}

/// The uppercase label that opens a section, such as "POSITION".
pub fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .line_height(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_muted())
        .child(label.into())
}

/// A read-only property field: grey box with a leading glyph and a value.
pub fn field(prefix: impl IntoElement, value: impl Into<SharedString>) -> Div {
    h_flex()
        .flex_1()
        .h(px(28.))
        .px(px(8.))
        .gap(px(8.))
        .rounded(px(6.))
        .bg(theme::field())
        .child(prefix)
        .child(div().text_color(theme::text()).child(value.into()))
}

/// Fixed-width letter used as a field prefix, such as "X" or "W".
pub fn field_letter(letter: &'static str) -> impl IntoElement {
    div()
        .w(px(10.))
        .flex_shrink_0()
        .text_color(theme::text_muted())
        .child(letter)
}

/// Small icon used as a field prefix, such as the rotation angle.
pub fn field_icon(icon: impl Into<Icon>) -> impl IntoElement {
    icon.into().size(px(12.)).text_color(theme::text_muted())
}
