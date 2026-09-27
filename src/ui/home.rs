//! The Home screen: the first screen of `sliderino`, where the person starts
//! a new presentation or opens a file. From the "Sliderino — Home" artboard
//! in Paper.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    Context, EventEmitter, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement, Render, Styled, TestSupportExt as _, Window, div, px, rgb,
};

use crate::shortcuts::Shortcuts;
use crate::theme;
use crate::ui::brand;
use crate::ui::top_bar::{agent_status, close_window};

/// What the Home screen asks of the window around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeEvent {
    New,
    Open,
}

pub struct HomeView {
    pub focus: FocusHandle,
    shortcuts: Shortcuts,
}

impl EventEmitter<HomeEvent> for HomeView {}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            shortcuts: Shortcuts::default(),
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts.new.matches(&event.keystroke) {
            cx.emit(HomeEvent::New);
        } else if self.shortcuts.open.matches(&event.keystroke) {
            cx.emit(HomeEvent::Open);
        } else {
            return;
        }
        cx.stop_propagation();
    }
}

fn title_bar(cx: &Context<HomeView>) -> impl IntoElement {
    TitleBar::new()
        .h(px(44.))
        .pl(px(16.))
        .bg(theme::background())
        .border_color(theme::border())
        .on_close_window(close_window)
        .child(
            h_flex().gap(px(10.)).child(brand::app_icon()).child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Sliderino"),
            ),
        )
        .child(div().pr(px(12.)).child(agent_status(None, cx)))
}

fn hero() -> impl IntoElement {
    v_flex()
        .gap(px(20.))
        .child(
            h_flex().gap(px(20.)).child(brand::mark(76.)).child(
                div()
                    .text_size(px(80.))
                    .line_height(px(80.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(theme::text())
                    .child("Sliderino"),
            ),
        )
        .child(
            div()
                .text_size(px(18.))
                .line_height(px(26.))
                .text_color(theme::text_muted())
                .child("The smallest slide grammar that works with agents."),
        )
}

fn actions(cx: &Context<HomeView>) -> impl IntoElement {
    h_flex()
        .gap(px(10.))
        .child(
            Button::new("new-presentation")
                .primary()
                .h(px(40.))
                .px(px(16.))
                .rounded(px(8.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(Icon::new(IconName::Plus).xsmall())
                        .child("New presentation")
                        .child(
                            div()
                                .pl(px(4.))
                                .text_size(px(12.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(0x9A9A9A))
                                .child("Ctrl N"),
                        ),
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::New))),
        )
        .child(
            Button::new("open-file")
                .outline()
                .h(px(40.))
                .px(px(16.))
                .rounded(px(8.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(Icon::new(IconName::FolderOpen).xsmall())
                        .child("Open file"),
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::Open))),
        )
}

impl Render for HomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("home-screen")
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .child(title_bar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .items_start()
                    .gap(px(36.))
                    .py(px(64.))
                    .px(px(96.))
                    .child(hero())
                    .child(actions(cx)),
            )
    }
}
