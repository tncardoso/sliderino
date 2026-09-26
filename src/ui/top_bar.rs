//! Window title bar: document breadcrumb, agent status and the main actions.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, TitleBar, h_flex};
use gpui_kit::{
    FontWeight, InteractiveElement as _, IntoElement, ParentElement, Styled, TestSupportExt as _,
    div, px,
};

use crate::editor::EditorView;
use crate::mock::DOCUMENT;
use crate::theme;

pub fn top_bar(editor: &EditorView) -> impl IntoElement {
    TitleBar::new()
        .h(px(44.))
        .pl(px(16.))
        .bg(theme::background())
        .border_color(theme::border())
        .child(breadcrumb())
        .child(actions(editor))
}

fn logo() -> impl IntoElement {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(6.))
        .bg(theme::ink())
        .child(
            div()
                .w(px(10.))
                .h(px(7.))
                .rounded(px(1.5))
                .bg(theme::background()),
        )
}

fn breadcrumb() -> impl IntoElement {
    h_flex().gap(px(12.)).child(logo()).child(
        h_flex()
            .gap(px(6.))
            .text_size(px(13.))
            .child(div().text_color(theme::text_muted()).child(DOCUMENT.folder))
            .child(div().text_color(theme::text_faint()).child("/"))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(DOCUMENT.title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(DOCUMENT.extension),
            ),
    )
}

fn agent_status() -> impl IntoElement {
    h_flex()
        .h(px(28.))
        .px(px(10.))
        .gap(px(6.))
        .rounded(px(14.))
        .bg(theme::accent_soft())
        .child(div().size(px(6.)).rounded_full().bg(theme::accent()))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::accent())
                .child(DOCUMENT.agent_status),
        )
}

fn actions(editor: &EditorView) -> impl IntoElement {
    let zoom = editor
        .camera
        .map(|camera| camera.label())
        .unwrap_or_default();
    h_flex()
        .gap(px(8.))
        .pr(px(12.))
        .child(agent_status())
        .child(
            div()
                .id("zoom-level")
                .test_support()
                .px(px(8.))
                .text_color(theme::text_muted())
                .child(zoom),
        )
        .child(
            Button::new("export")
                .outline()
                .xsmall()
                .label("Export")
                .h(px(28.))
                .px(px(12.)),
        )
        .child(
            Button::new("present")
                .primary()
                .xsmall()
                .icon(IconName::Play)
                .label("Present")
                .h(px(28.))
                .px(px(12.)),
        )
}
