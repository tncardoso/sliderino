//! Left panel: slide thumbnails and, later, the component library.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, div, linear_color_stop, linear_gradient, px, relative,
    rgb,
};

use crate::editor::EditorView;
use crate::mock::{SLIDES, SlideKind};
use crate::theme;

const THUMB_WIDTH: f32 = 184.;
const THUMB_HEIGHT: f32 = 104.;

pub fn slides_panel(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    v_flex()
        .w(px(232.))
        .flex_shrink_0()
        .h_full()
        .bg(theme::background())
        .border_r_1()
        .border_color(theme::border())
        .child(
            TabBar::new("library-tabs")
                .underline()
                .xsmall()
                .h(px(40.))
                .pl(px(8.))
                .pr(px(8.))
                .selected_index(editor.library_tab)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.library_tab = *ix;
                    cx.notify();
                }))
                .child(Tab::new().label("Slides"))
                .child(Tab::new().label("Components"))
                .suffix(
                    Button::new("add-slide")
                        .ghost()
                        .small()
                        .icon(IconName::Plus),
                ),
        )
        .child(
            v_flex()
                .id("thumbnails")
                .flex_1()
                .gap(px(14.))
                .pt(px(12.))
                .pb(px(16.))
                .pl(px(12.))
                .pr(px(16.))
                .overflow_y_scrollbar()
                .children(SLIDES.iter().enumerate().map(|(ix, slide)| {
                    let active = ix == editor.active_slide;
                    h_flex()
                        .id(("slide", ix))
                        .items_start()
                        .gap(px(8.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.active_slide = ix;
                            cx.notify();
                        }))
                        .child(slide_number(ix + 1, active))
                        .child(thumbnail(slide, active))
                })),
        )
}

fn slide_number(number: usize, active: bool) -> impl IntoElement {
    div()
        .w(px(16.))
        .flex_shrink_0()
        .pt(px(2.))
        .flex()
        .justify_end()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(if active {
            theme::accent()
        } else {
            theme::text_muted()
        })
        .when(active, |this| this.font_weight(FontWeight::SEMIBOLD))
        .child(number.to_string())
}

/// The frame of a thumbnail; the active slide gets a 2px accent ring.
fn frame(active: bool) -> Div {
    let frame = div()
        .w(px(THUMB_WIDTH))
        .h(px(THUMB_HEIGHT))
        .flex_shrink_0()
        .rounded(px(4.))
        .overflow_hidden()
        .bg(theme::background());
    if active {
        frame.border_2().border_color(theme::accent())
    } else {
        frame.border_1().border_color(theme::border())
    }
}

fn thumbnail(slide: &SlideKind, active: bool) -> AnyElement {
    let frame = frame(active);
    match slide {
        SlideKind::Title { title, subtitle } => frame
            .bg(theme::ink())
            .flex()
            .flex_col()
            .justify_end()
            .gap(px(4.))
            .p(px(12.))
            .child(div().w(px(28.)).h(px(3.)).bg(theme::accent()))
            .child(thumb_heading(title, 13.).text_color(theme::background()))
            .child(
                div()
                    .text_size(px(6.))
                    .line_height(px(8.))
                    .text_color(rgb(0x9A9A9A))
                    .child(*subtitle),
            )
            .into_any_element(),
        SlideKind::Kpi { title, values } => {
            frame
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .child(thumb_heading(title, 8.).w(px(110.)))
                .child(h_flex().gap(px(6.)).children(values.iter().enumerate().map(
                    |(ix, value)| {
                        v_flex()
                            .flex_1()
                            .gap(px(2.))
                            .p(px(5.))
                            .rounded(px(2.))
                            .bg(theme::surface())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(14.))
                                    .font_weight(FontWeight::EXTRA_BOLD)
                                    .text_color(if ix == 0 {
                                        theme::accent()
                                    } else {
                                        theme::text()
                                    })
                                    .child(*value),
                            )
                            .child(placeholder_line(30.))
                    },
                )))
                .into_any_element()
        }
        SlideKind::TwoColumn { title } => frame
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .child(thumb_heading(title, 8.))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(10.))
                    .child(text_column(0.6))
                    .child(text_column(0.75)),
            )
            .into_any_element(),
        SlideKind::ImageRight { title } => frame
            .flex()
            .gap(px(10.))
            .p(px(12.))
            .child(
                v_flex()
                    .flex_1()
                    .gap(px(4.))
                    .child(thumb_heading(title, 8.))
                    .child(div().h(px(2.)).bg(theme::placeholder_line()))
                    .child(
                        div()
                            .h(px(2.))
                            .w(relative(0.7))
                            .bg(theme::placeholder_line()),
                    ),
            )
            .child(div().w(px(84.)).rounded(px(2.)).bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0xC9D5FF), 0.),
                linear_color_stop(theme::accent(), 1.),
            )))
            .into_any_element(),
        SlideKind::Closing { title } => frame
            .bg(theme::accent())
            .flex()
            .items_center()
            .justify_center()
            .child(thumb_heading(title, 12.).text_color(theme::background()))
            .into_any_element(),
    }
}

fn thumb_heading(text: &'static str, size: f32) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(size + 1.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(theme::text())
        .child(text)
}

fn placeholder_line(width: f32) -> Div {
    div()
        .w(px(width))
        .h(px(2.))
        .flex_shrink_0()
        .bg(rgb(0xC9C9C9))
}

/// A column of fake text: a dark heading rule and three grey lines.
fn text_column(last_line: f32) -> impl IntoElement {
    v_flex()
        .flex_1()
        .gap(px(3.))
        .child(div().w(px(40.)).h(px(2.)).bg(theme::ink()))
        .child(div().h(px(2.)).bg(theme::placeholder_line()))
        .child(div().h(px(2.)).bg(theme::placeholder_line()))
        .child(
            div()
                .h(px(2.))
                .w(relative(last_line))
                .bg(theme::placeholder_line()),
        )
}
