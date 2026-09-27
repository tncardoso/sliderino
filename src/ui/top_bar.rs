//! Title bar of the editor: Home and the document name, the agent status
//! and the main actions.

use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div, px,
};

use crate::api::protocol::ClientKind;
use crate::api::server::Agents;
use crate::editor::{EditorEvent, EditorView};
use crate::theme;
use crate::ui::brand;
use crate::ui::workspace::CloseWindow;

pub fn top_bar(editor: &EditorView, cx: &Context<EditorView>) -> impl IntoElement {
    TitleBar::new()
        .h(px(44.))
        .pl(px(16.))
        .bg(theme::background())
        .border_color(theme::border())
        .on_close_window(close_window)
        .child(breadcrumb(editor, cx))
        .child(actions(editor, cx))
}

/// The close button of the title bars: the workspace asks about unsaved
/// changes first.
pub fn close_window(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(CloseWindow), cx);
}

/// Home, then the name of the presentation. A dot tells that it has
/// unsaved changes.
fn breadcrumb(editor: &EditorView, cx: &Context<EditorView>) -> impl IntoElement {
    h_flex().gap(px(12.)).child(brand::app_icon()).child(
        h_flex()
            .gap(px(6.))
            .text_size(px(13.))
            .child(
                div()
                    .id("home")
                    .test_support()
                    .cursor_pointer()
                    .text_color(theme::text_muted())
                    .hover(|style| style.text_color(theme::text()))
                    .child("Home")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::GoHome))),
            )
            .child(div().text_color(theme::text_faint()).child("/"))
            .child(
                div()
                    .id("document-title")
                    .test_support()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(editor.title()),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(format!(".{}", crate::file::EXTENSION)),
            )
            .when(editor.is_dirty(), |row| {
                row.child(
                    div()
                        .id("unsaved")
                        .test_support()
                        .text_color(theme::text_muted())
                        .child("•"),
                )
            }),
    )
}

/// The agents connected through the API. Clicking opens the list of
/// clients and, in the editor, the "Follow agent" switch.
pub fn agent_status(editor: Option<Entity<EditorView>>, cx: &App) -> impl IntoElement {
    let status = Agents::get(cx).and_then(|agents| agents.status(Instant::now()));
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
    Popover::new("agent-status-popover")
        .anchor(Anchor::TopRight)
        .trigger(trigger)
        .content(move |_, _, cx| {
            let clients: Vec<_> = Agents::get(cx)
                .map(|agents| agents.clients.clone())
                .unwrap_or_default();
            let rows = clients.iter().map(|client| {
                let kind = match client.kind {
                    ClientKind::Mcp => "MCP",
                    ClientKind::Cli => "CLI",
                };
                div()
                    .text_color(theme::text())
                    .child(format!("{} · {kind}", client.name))
            });
            let follow = editor.clone().map(|editor| {
                Switch::new("follow-agent")
                    .small()
                    .checked(editor.read(cx).follow_agents)
                    .label("Follow agent")
                    .tooltip("Show the slide and the element of each agent edit")
                    .on_change(move |follow, _, cx| {
                        editor.update(cx, |editor, cx| {
                            editor.follow_agents = *follow;
                            cx.notify();
                        });
                    })
            });
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
                .when(clients.is_empty(), |list| {
                    list.child(
                        div()
                            .text_color(theme::text_muted())
                            .child("No agent is connected. Agents connect with sliderino mcp."),
                    )
                })
                .children(rows)
                .children(follow)
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
        .child(agent_status(Some(cx.entity()), cx))
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
