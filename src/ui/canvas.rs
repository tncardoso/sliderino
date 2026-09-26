//! Center area: the slide on the canvas ground, with zoom and pan, and the
//! tool palette.
//!
//! The slide is still empty; its content arrives with the renderer.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{Selectable as _, h_flex};
use gpui_kit::{
    AnyElement, App, BoxShadow, Context, CursorStyle, DispatchPhase, Edges, InteractiveElement as _,
    IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, ScrollDelta,
    ScrollWheelEvent, Styled, TestSupportExt as _, Window, canvas as paint_canvas, div, hsla, point,
    px,
};

use crate::camera::Camera;
use crate::editor::{EditorView, Tool};
use crate::shortcuts::WheelAction;
use crate::theme;

/// Space kept around a fitted slide; the bottom clears the tool palette.
const FIT_INSETS: Edges<gpui_kit::Pixels> = Edges {
    top: px(48.),
    right: px(48.),
    bottom: px(104.),
    left: px(48.),
};

/// Pixel scroll deltas (trackpads) count as one wheel line per this distance.
const PIXELS_PER_LINE: f32 = 50.;

pub fn canvas(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let cursor = match (editor.pan_drag.is_some(), editor.effective_tool()) {
        (true, _) => CursorStyle::ClosedHand,
        (false, Tool::Hand) => CursorStyle::OpenHand,
        _ => CursorStyle::Arrow,
    };

    div()
        .id("canvas")
        .test_support()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .bg(theme::canvas())
        .cursor(cursor)
        .on_any_mouse_down(cx.listener(EditorView::on_canvas_mouse_down))
        .on_scroll_wheel(cx.listener(EditorView::on_canvas_scroll))
        .child(viewport_tracker(cx))
        .children(editor.camera.map(|camera| slide(editor, camera)))
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

/// Invisible layer covering the canvas. It records the canvas bounds for the
/// camera and listens to the pointer at window level, so a pan keeps going
/// when the pointer leaves the canvas and ends on a release anywhere.
fn viewport_tracker(cx: &mut Context<EditorView>) -> impl IntoElement {
    let prepaint_view = cx.entity().downgrade();
    let paint_view = prepaint_view.clone();
    paint_canvas(
        move |bounds, _, cx: &mut App| {
            prepaint_view
                .update(cx, |this, cx| {
                    this.viewport = bounds;
                    if this.camera.is_none() {
                        this.zoom_to_fit();
                        cx.notify();
                    }
                })
                .ok();
        },
        move |_, _, window, cx| {
            let panning = paint_view
                .read_with(cx, |this, _| this.pan_drag.is_some())
                .unwrap_or(false);
            if panning {
                window.set_window_cursor_style(CursorStyle::ClosedHand);
            }
            let view = paint_view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    view.update(cx, |this, cx| this.on_pan_move(event, cx)).ok();
                }
            });
            let view = paint_view.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    view.update(cx, |this, cx| this.on_pan_end(event, cx)).ok();
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// The slide, positioned from a zero-size anchor at the canvas center so the
/// layout needs only the camera, not the measured bounds.
fn slide(editor: &EditorView, camera: Camera) -> impl IntoElement {
    let size = camera.slide_size(editor.presentation.size);
    div()
        .absolute()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div().relative().size_0().child(
                div()
                    .id("slide")
                    .test_support()
                    .absolute()
                    .left(camera.pan.x - size.width / 2.)
                    .top(camera.pan.y - size.height / 2.)
                    .w(size.width)
                    .h(size.height)
                    .bg(theme::background())
                    .shadow(vec![shadow(1., 2., 0., 0.06), shadow(8., 24., 0., 0.06)]),
            ),
        )
}

impl EditorView {
    /// Fits the slide into the last measured canvas bounds.
    pub fn zoom_to_fit(&mut self) {
        self.camera = Some(Camera::fit(
            self.viewport.size,
            self.presentation.size,
            FIT_INSETS,
        ));
    }

    fn on_canvas_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.pan_drag.is_some() {
            return;
        }
        let pans = self.shortcuts.pan_button.matches(event.button)
            || (event.button == gpui_kit::MouseButton::Left
                && self.effective_tool() == Tool::Hand);
        if pans {
            self.pan_drag = Some((event.button, event.position));
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_pan_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((button, last)) = self.pan_drag else {
            return;
        };
        if let Some(camera) = &mut self.camera {
            camera.pan_by(event.position - last);
        }
        self.pan_drag = Some((button, event.position));
        cx.notify();
    }

    fn on_pan_end(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.pan_drag.is_some_and(|(button, _)| button == event.button) {
            self.pan_drag = None;
            cx.notify();
        }
    }

    fn on_canvas_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcuts.wheel != WheelAction::Zoom {
            return;
        }
        let Some(camera) = &mut self.camera else {
            return;
        };
        // Positive y is a scroll up, which zooms in.
        let lines = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / PIXELS_PER_LINE,
        };
        if lines == 0. {
            return;
        }
        let anchor = event.position - self.viewport.center();
        camera.zoom_at(anchor, self.shortcuts.wheel_zoom_step.powf(lines));
        cx.stop_propagation();
        cx.notify();
    }
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
    let active = editor.effective_tool();
    let mut items: Vec<AnyElement> = Vec::new();
    for group in Tool::GROUPS {
        if !items.is_empty() {
            items.push(divider().into_any_element());
        }
        for tool in group {
            items.push(tool_button(*tool, active == *tool, cx).into_any_element());
        }
    }

    // Presses on the palette must not start a pan on the canvas below.
    h_flex()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
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

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke,
        MouseButton, Pixels, Point, ScrollDelta, TestAppContext, WindowHandle, point, px, size,
    };

    use crate::editor::{EditorView, Tool};

    fn open(cx: &mut TestAppContext) -> WindowHandle<EditorView> {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(cx);
        });
        let handle = cx.open_window(size(px(1440.), px(900.)), EditorView::new);
        cx.update_window(handle.into(), |editor, window, cx| {
            // The app focuses the editor when it opens the window.
            let focus = editor.downcast::<EditorView>().unwrap().read(cx).focus.clone();
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();
        handle
    }

    fn with_window<R>(
        cx: &mut TestAppContext,
        handle: WindowHandle<EditorView>,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        cx.update_window(handle.into(), |_, window, cx| f(window, cx))
            .unwrap()
    }

    fn slide(window: &gpui_kit::Window) -> Bounds<Pixels> {
        window.find("slide").bounds()
    }

    fn canvas_center(window: &gpui_kit::Window) -> Point<Pixels> {
        window.find("canvas").bounds().center()
    }

    fn close(a: Pixels, b: Pixels) -> bool {
        (f32::from(a) - f32::from(b)).abs() < 0.5
    }

    fn assert_moved(before: Bounds<Pixels>, after: Bounds<Pixels>, delta: Point<Pixels>) {
        assert!(
            close(after.origin.x - before.origin.x, delta.x)
                && close(after.origin.y - before.origin.y, delta.y)
                && after.size == before.size,
            "expected {before:?} moved by {delta:?}, got {after:?}"
        );
    }

    fn drag(
        window: &mut gpui_kit::Window,
        button: MouseButton,
        from: Point<Pixels>,
        to: Point<Pixels>,
        cx: &mut gpui_kit::App,
    ) {
        use gpui_kit::{MouseDownEvent, MouseMoveEvent, MouseUpEvent};
        let events = [
            MouseMoveEvent {
                position: from,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            MouseDownEvent {
                button,
                position: from,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            MouseMoveEvent {
                position: to,
                pressed_button: Some(button),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            MouseUpEvent {
                button,
                position: to,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
        ];
        for event in events {
            window.dispatch_event(event, cx);
            window.render_frame(cx);
        }
    }

    fn key(window: &mut gpui_kit::Window, source: &str, down: bool, cx: &mut gpui_kit::App) {
        let keystroke = Keystroke::parse(source).unwrap();
        let event = if down {
            KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input()
        } else {
            KeyUpEvent { keystroke }.to_platform_input()
        };
        window.dispatch_event(event, cx);
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn opens_with_the_slide_fitted_and_centered(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, _| {
            let canvas = window.find("canvas").bounds();
            let slide = slide(window);
            let ratio = f32::from(slide.size.width) / f32::from(slide.size.height);
            assert!((ratio - 16. / 9.).abs() < 0.01, "{slide:?}");
            assert!(close(slide.center().x, canvas.center().x));
            assert!(slide.origin.x >= canvas.origin.x + px(48.) - px(0.5));
            assert!(slide.right() <= canvas.right() - px(48.) + px(0.5));
            assert!(slide.bottom() <= canvas.bottom() - px(104.) + px(0.5));
        });
        let label = handle
            .read_with(cx, |editor, _| editor.camera.unwrap().label())
            .unwrap();
        assert!(label.ends_with('%') && label != "100%", "{label}");
    }

    #[gpui_kit::test]
    fn wheel_zooms_around_the_pointer(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let pointer = before.origin + point(before.size.width / 4., before.size.height / 3.);
            let offset = pointer - before.origin;
            let fraction = |v: Pixels| f32::from(v) / f32::from(before.size.width);
            let (fx, fy) = (fraction(offset.x), fraction(offset.y));
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: pointer,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: pointer,
                    delta: ScrollDelta::Lines(point(0., 1.)),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let after = slide(window);
            let ratio = f32::from(after.size.width) / f32::from(before.size.width);
            assert!((ratio - 1.1).abs() < 0.001, "zoomed by {ratio}");
            let anchored = after.origin + point(after.size.width * fx, after.size.width * fy);
            assert!(close(anchored.x, pointer.x) && close(anchored.y, pointer.y));

            for _ in 0..100 {
                window.dispatch_event(
                    gpui_kit::ScrollWheelEvent {
                        position: pointer,
                        delta: ScrollDelta::Lines(point(0., -1.)),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            window.render_frame(cx);
            assert!(close(slide(window).size.width, px(160.)), "stops at 10%");
        });
    }

    #[gpui_kit::test]
    fn middle_button_pans_with_any_tool(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let from = canvas_center(window);
            let delta = point(px(120.), px(-45.));
            drag(window, MouseButton::Middle, from, from + delta, cx);
            assert_moved(before, slide(window), delta);
        });
    }

    #[gpui_kit::test]
    fn left_drag_pans_only_with_the_hand_tool(cx: &mut TestAppContext) {
        let handle = open(cx);
        let delta = point(px(-80.), px(60.));
        with_window(cx, handle, |window, cx| {
            let before = slide(window);
            let from = canvas_center(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), point(px(0.), px(0.)));

            window.click("tool-hand", cx);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), delta);
        });
        let tool = handle.read_with(cx, |editor, _| editor.active_tool).unwrap();
        assert_eq!(tool, Tool::Hand);
    }

    #[gpui_kit::test]
    fn holding_space_over_the_canvas_enables_the_hand(cx: &mut TestAppContext) {
        let handle = open(cx);
        let delta = point(px(50.), px(30.));
        with_window(cx, handle, |window, cx| {
            let from = canvas_center(window);
            window.hover("canvas", cx);
            key(window, "space", true, cx);
            let before = slide(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), delta);

            key(window, "space", false, cx);
            let before = slide(window);
            drag(window, MouseButton::Left, from, from + delta, cx);
            assert_moved(before, slide(window), point(px(0.), px(0.)));
        });
        let tool = handle.read_with(cx, |editor, _| editor.active_tool).unwrap();
        assert_eq!(tool, Tool::Move);
    }

    #[gpui_kit::test]
    fn space_outside_the_canvas_does_nothing(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            // Over the slides panel, left of the canvas.
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: point(px(100.), px(300.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            key(window, "space", true, cx);
        });
        let held = handle.read_with(cx, |editor, _| editor.hand_key_held).unwrap();
        assert!(!held);
    }

    #[gpui_kit::test]
    fn ctrl_0_shows_100_percent_and_ctrl_1_fits(cx: &mut TestAppContext) {
        let handle = open(cx);
        with_window(cx, handle, |window, cx| {
            let fitted = slide(window);
            window.hover("canvas", cx);
            key(window, "ctrl-0", true, cx);
            let full = slide(window);
            assert!(close(full.size.width, px(1600.)) && close(full.size.height, px(900.)));
            key(window, "ctrl-1", true, cx);
            assert_eq!(slide(window), fitted);
        });
    }
}
