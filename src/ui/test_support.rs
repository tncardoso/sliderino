//! Helpers for UI tests: open the editor in a test window and drive the
//! pointer and keyboard.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    WindowHandle, px, size,
};

use crate::editor::EditorView;
use crate::history::History;

pub fn open(cx: &mut TestAppContext) -> WindowHandle<EditorView> {
    open_with(cx, crate::document::Presentation::new())
}

/// Opens the editor on an existing presentation.
pub fn open_with(
    cx: &mut TestAppContext,
    presentation: crate::document::Presentation,
) -> WindowHandle<EditorView> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(cx);
        cx.text_system()
            .add_fonts(crate::assets::fonts())
            .expect("embedded Inter fonts load");
    });
    let handle = cx.open_window(size(px(1440.), px(900.)), move |window, cx| {
        EditorView::with_document(presentation, History::default(), window, cx)
    });
    cx.update_window(handle.into(), |editor, window, cx| {
        // The app focuses the editor when it opens the window.
        let focus = editor
            .downcast::<EditorView>()
            .unwrap()
            .read(cx)
            .focus
            .clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    handle
}

pub fn with_window<R>(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

/// Runs `f` on the editor and renders a frame afterwards.
pub fn with_editor<R>(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    f: impl FnOnce(&mut EditorView, &mut gpui_kit::Context<EditorView>) -> R,
) -> R {
    let result = handle.update(cx, |editor, _, cx| f(editor, cx)).unwrap();
    with_window(cx, handle, |window, cx| window.render_frame(cx));
    result
}

pub fn read<R>(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    f: impl FnOnce(&EditorView) -> R,
) -> R {
    handle.read_with(cx, |editor, _| f(editor)).unwrap()
}

pub fn slide(window: &gpui_kit::Window) -> Bounds<Pixels> {
    window.find("slide").bounds()
}

pub fn canvas_center(window: &gpui_kit::Window) -> Point<Pixels> {
    window.find("canvas").bounds().center()
}

pub fn close(a: Pixels, b: Pixels) -> bool {
    (f32::from(a) - f32::from(b)).abs() < 0.5
}

pub fn assert_moved(before: Bounds<Pixels>, after: Bounds<Pixels>, delta: Point<Pixels>) {
    assert!(
        close(after.origin.x - before.origin.x, delta.x)
            && close(after.origin.y - before.origin.y, delta.y)
            && after.size == before.size,
        "expected {before:?} moved by {delta:?}, got {after:?}"
    );
}

/// Presses at `from`, moves to `to` and releases there.
pub fn drag(
    window: &mut gpui_kit::Window,
    button: MouseButton,
    from: Point<Pixels>,
    to: Point<Pixels>,
    cx: &mut gpui_kit::App,
) {
    drag_with(window, button, from, to, Modifiers::default(), cx);
}

pub fn drag_with(
    window: &mut gpui_kit::Window,
    button: MouseButton,
    from: Point<Pixels>,
    to: Point<Pixels>,
    modifiers: Modifiers,
    cx: &mut gpui_kit::App,
) {
    let events = [
        MouseMoveEvent {
            position: from,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        MouseDownEvent {
            button,
            position: from,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        MouseMoveEvent {
            position: to,
            pressed_button: Some(button),
            modifiers,
        }
        .to_platform_input(),
        MouseUpEvent {
            button,
            position: to,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
    ];
    for event in events {
        window.dispatch_event(event, cx);
        window.render_frame(cx);
    }
}

/// A left click at a window position; `count` 2 is a double click.
pub fn click_at(
    window: &mut gpui_kit::Window,
    position: Point<Pixels>,
    count: usize,
    cx: &mut gpui_kit::App,
) {
    click_with(window, position, count, Modifiers::default(), cx);
}

/// A left click with modifiers held.
pub fn click_with(
    window: &mut gpui_kit::Window,
    position: Point<Pixels>,
    count: usize,
    modifiers: Modifiers,
    cx: &mut gpui_kit::App,
) {
    let events = [
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: count,
            first_mouse: false,
        }
        .to_platform_input(),
        MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: count,
        }
        .to_platform_input(),
    ];
    for event in events {
        window.dispatch_event(event, cx);
        window.render_frame(cx);
    }
}

pub fn key(window: &mut gpui_kit::Window, source: &str, down: bool, cx: &mut gpui_kit::App) {
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
