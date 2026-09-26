//! The editor window: title bar over the slides panel, canvas and inspector.
//!
//! The slides panel and inspector still show mock data; the canvas reads the
//! presentation size and the viewport camera.

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::{
    Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseButton, ParentElement, Pixels, Point, Render, Styled, Subscription, Window, px,
};

use crate::camera::Camera;
use crate::document::Presentation;
use crate::shortcuts::Shortcuts;
use crate::theme;
use crate::ui::canvas::canvas;
use crate::ui::properties_panel::properties_panel;
use crate::ui::slides_panel::slides_panel;
use crate::ui::top_bar::top_bar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Tool picked in the palette; see [`EditorView::effective_tool`].
    pub active_tool: Tool,
    pub presentation: Presentation,
    pub shortcuts: Shortcuts,
    /// None until the canvas is first measured; then it is fitted.
    pub camera: Option<Camera>,
    /// Canvas bounds in window coordinates, from the last prepaint.
    pub viewport: Bounds<Pixels>,
    /// Button that started the pan in progress and the last pointer position.
    pub pan_drag: Option<(MouseButton, Point<Pixels>)>,
    /// The hand-hold key (space by default) is down over the canvas.
    pub hand_key_held: bool,
    /// Keyboard focus of the editor; key events reach the canvas through it.
    pub focus: FocusHandle,
    _activation: Subscription,
}

impl EditorView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A key up or mouse up lost while the window is inactive would leave
        // the hand tool or a pan stuck on.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.hand_key_held = false;
                this.pan_drag = None;
                cx.notify();
            }
        });
        Self {
            active_slide: 1,
            library_tab: 0,
            inspector_tab: 0,
            active_tool: Tool::Move,
            presentation: Presentation::new(),
            shortcuts: Shortcuts::default(),
            camera: None,
            viewport: Bounds::default(),
            pan_drag: None,
            hand_key_held: false,
            focus: cx.focus_handle(),
            _activation: activation,
        }
    }

    /// The tool that pointer input uses: the hand while the hand key is held
    /// or a pan is in progress, otherwise the palette tool.
    pub fn effective_tool(&self) -> Tool {
        if self.hand_key_held || self.pan_drag.is_some() {
            Tool::Hand
        } else {
            self.active_tool
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.viewport.contains(&window.mouse_position()) {
            return;
        }
        let keystroke = &event.keystroke;
        if self.shortcuts.hand_hold.matches(keystroke) {
            if !event.is_held && !self.hand_key_held {
                self.hand_key_held = true;
                cx.notify();
            }
        } else if self.shortcuts.zoom_to_fit.matches(keystroke) {
            self.zoom_to_fit();
            cx.notify();
        } else if self.shortcuts.zoom_to_100.matches(keystroke) {
            if let Some(camera) = &mut self.camera {
                camera.set_zoom(1.);
            }
            cx.notify();
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.hand_key_held && self.shortcuts.hand_hold.matches_key(&event.keystroke) {
            self.hand_key_held = false;
            cx.notify();
        }
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("editor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .size_full()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .child(top_bar(self))
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
