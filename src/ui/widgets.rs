//! Small building blocks shared by the side panels.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    BoxShadow, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Stateful, Styled, TestSupportExt as _, div, hsla, point, px,
};

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

/// An editable property field: the grey box of [`field`] around a text input.
pub fn input_field(prefix: impl IntoElement, input: &Entity<InputState>) -> Div {
    h_flex()
        .flex_1()
        .min_w_0()
        .h(px(28.))
        .pl(px(8.))
        .gap(px(2.))
        .rounded(px(6.))
        .bg(theme::field())
        .child(prefix)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).appearance(false).xsmall()),
        )
}

/// A row of mutually exclusive icon buttons on a grey track, such as the
/// text alignment. Children come from [`segment`].
pub fn segmented() -> Div {
    h_flex()
        .h(px(28.))
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(6.))
        .bg(theme::field())
}

/// One button of a [`segmented`] row; the selected one sits on a white chip.
/// Disabled buttons are faded and ignore clicks.
pub fn segment(
    id: &'static str,
    icon: IconName,
    selected: bool,
    disabled: bool,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(selected, |this| {
            this.bg(theme::background()).shadow(vec![BoxShadow {
                color: hsla(0., 0., 0., 0.08),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }])
        })
        .when(!disabled, |this| this.cursor_pointer())
        .child(Icon::new(icon).size(px(14.)).text_color(if disabled {
            theme::text_faint()
        } else if selected {
            theme::ink()
        } else {
            theme::text_muted()
        }))
}
