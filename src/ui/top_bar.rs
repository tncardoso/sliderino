//! Window title bar: document breadcrumb, agent status and the main actions.

use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Styled,
    TestSupportExt as _, div, px,
};

use crate::api::protocol::ClientKind;
use crate::editor::EditorView;
use crate::mock::DOCUMENT;
use crate::theme;

pub fn top_bar(editor: &EditorView, cx: &Context<EditorView>) -> impl IntoElement {
    TitleBar::new()
        .h(px(44.))
        .pl(px(16.))
        .bg(theme::background())
        .border_color(theme::border())
        .child(breadcrumb())
        .child(actions(editor, cx))
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

/// The agents connected through the API. Clicking opens the list of
/// clients and the "Follow agent" switch.
fn agent_status(editor: &EditorView, cx: &Context<EditorView>) -> impl IntoElement {
    let status = editor.agents.status(Instant::now());
    let connected = status.is_some();
    let (dot, text) = if connected {
        (theme::accent(), theme::accent())
    } else {
        (theme::text_faint(), theme::text_muted())
    };
    let trigger = Button::new("agent-status")
        .ghost()
        .xsmall()
        .h(px(28.))
        .px(px(10.))
        .rounded(px(14.))
        .when(connected, |button| button.bg(theme::accent_soft()))
        .child(
            h_flex()
                .gap(px(6.))
                .child(div().size(px(6.)).rounded_full().bg(dot))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child(status.unwrap_or_else(|| "No agent".into())),
                ),
        );
    let editor = cx.entity();
    Popover::new("agent-status-popover")
        .anchor(Anchor::TopRight)
        .trigger(trigger)
        .content(move |_, _, cx| {
            let agents = &editor.read(cx).agents;
            let clients = agents.clients.iter().map(|client| {
                let kind = match client.kind {
                    ClientKind::Mcp => "MCP",
                    ClientKind::Cli => "CLI",
                };
                div()
                    .text_color(theme::text())
                    .child(format!("{} · {kind}", client.name))
            });
            let editor = editor.clone();
            v_flex()
                .w(px(220.))
                .gap(px(8.))
                .text_size(px(12.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child("Agents"),
                )
                .when(agents.clients.is_empty(), |list| {
                    list.child(
                        div()
                            .text_color(theme::text_muted())
                            .child("No agent is connected. Agents connect with sliderino mcp."),
                    )
                })
                .children(clients)
                .child(
                    Switch::new("follow-agent")
                        .small()
                        .checked(agents.follow)
                        .label("Follow agent")
                        .tooltip("Show the slide and the element of each agent edit")
                        .on_change(move |follow, _, cx| {
                            editor.update(cx, |editor, cx| {
                                editor.agents.follow = *follow;
                                cx.notify();
                            });
                        }),
                )
        })
}

fn actions(editor: &EditorView, cx: &Context<EditorView>) -> impl IntoElement {
    let zoom = editor
        .camera
        .map(|camera| camera.label())
        .unwrap_or_default();
    let converting = editor.converting.map(|share| {
        div()
            .id("converting")
            .test_support()
            .px(px(8.))
            .text_color(theme::text_muted())
            .child(format!("Converting video… {}%", (share * 100.).round()))
    });
    h_flex()
        .gap(px(8.))
        .pr(px(12.))
        .children(converting)
        .child(agent_status(editor, cx))
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
                .px(px(12.))
                .on_click(cx.listener(|this, _, window, cx| this.present(window, cx))),
        )
}
