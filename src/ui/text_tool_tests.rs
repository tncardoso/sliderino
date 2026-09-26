//! End-to-end tests of the text tool in a test window: creating, typing,
//! undoing, resizing, snapping and formatting through the inspector.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    Focusable as _, Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle,
};

use crate::document::{ElementId, Frame, HAlign, TextSizing};
use crate::editor::{EditorView, Tool};
use crate::snap::Handle;
use crate::ui::inspector::Field;
use crate::ui::test_support::{click_at, drag_with, key, open, read, with_editor, with_window};

/// Window position of a slide point.
fn at(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

fn zoom(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> f32 {
    read(cx, handle, |editor| editor.camera.unwrap().zoom)
}

fn only_element(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> ElementId {
    read(cx, handle, |editor| {
        let elements = &editor.current_slide().elements;
        assert_eq!(elements.len(), 1, "one element on the slide");
        elements[0].id
    })
}

fn text_of(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
) -> (String, Frame, TextSizing) {
    read(cx, handle, |editor| {
        let element = editor.presentation.element(id).unwrap();
        let text = element.as_text().unwrap();
        (text.content.clone(), element.frame, text.sizing)
    })
}

fn history(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<String> {
    read(cx, handle, |editor| {
        editor.history.done().map(String::from).collect()
    })
}

/// Clicks with the text tool at a slide point and types `text`.
fn create_by_click(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    x: f32,
    y: f32,
    text: &str,
) -> ElementId {
    let position = at(cx, handle, x, y);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        click_at(window, position, 1, cx);
        window.input(text, cx);
    });
    only_element(cx, handle)
}

#[gpui_kit::test]
fn clicking_with_the_text_tool_creates_an_auto_width_box(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_by_click(cx, handle, 200., 300., "Hello");
    let (content, frame, sizing) = text_of(cx, handle, id);
    assert_eq!(content, "Hello");
    assert_eq!(sizing, TextSizing::AutoWidth);
    assert!((frame.x - 200.).abs() < 0.01, "{frame:?}");
    assert!(frame.width > 50. && frame.height > 30., "{frame:?}");
    assert_eq!(history(cx, handle), ["Create text", "Edit text"]);
    let (editing, tool) = read(cx, handle, |editor| {
        (
            editor.text_edit.as_ref().map(|edit| edit.caret),
            editor.active_tool,
        )
    });
    assert_eq!(editing, Some(5));
    assert_eq!(tool, Tool::Move, "the palette goes back to Move");

    // The box grows with the text.
    with_window(cx, handle, |window, cx| window.input("World", cx));
    let (_, wider, _) = text_of(cx, handle, id);
    assert!(wider.width > frame.width);
}

#[gpui_kit::test]
fn dragging_with_the_text_tool_creates_a_fixed_box(cx: &mut TestAppContext) {
    let handle = open(cx);
    let from = at(cx, handle, 100., 100.);
    let to = at(cx, handle, 500., 300.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        );
        window.input("Hi", cx);
    });
    let id = only_element(cx, handle);
    let (content, frame, sizing) = text_of(cx, handle, id);
    assert_eq!(content, "Hi");
    assert_eq!(sizing, TextSizing::Fixed);
    assert!((frame.x - 100.).abs() < 1. && (frame.y - 100.).abs() < 1.);
    assert!(
        (frame.width - 400.).abs() < 1. && (frame.height - 200.).abs() < 1.,
        "{frame:?}"
    );
}

#[gpui_kit::test]
fn undo_reverts_a_typing_burst_then_the_creation(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_by_click(cx, handle, 200., 300., "Hello");
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    let (content, ..) = text_of(cx, handle, id);
    assert_eq!(content, "", "the whole burst is one step");
    let editing = read(cx, handle, |editor| editor.text_edit.is_some());
    assert!(editing, "undo keeps the text in edit mode");

    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    let (exists, editing) = read(cx, handle, |editor| {
        (
            editor.presentation.element(id).is_some(),
            editor.text_edit.is_some(),
        )
    });
    assert!(!exists && !editing);

    with_window(cx, handle, |window, cx| {
        key(window, "ctrl-shift-z", true, cx);
        key(window, "ctrl-y", true, cx);
    });
    let (content, ..) = text_of(cx, handle, id);
    assert_eq!(content, "Hello");
}

#[gpui_kit::test]
fn leaving_an_empty_box_deletes_it(cx: &mut TestAppContext) {
    let handle = open(cx);
    let position = at(cx, handle, 200., 300.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        click_at(window, position, 1, cx);
        key(window, "escape", true, cx);
    });
    let count = read(cx, handle, |editor| editor.current_slide().elements.len());
    assert_eq!(count, 0);
    assert_eq!(history(cx, handle), ["Create text", "Delete text"]);
}

#[gpui_kit::test]
fn editing_keys_move_the_caret_and_delete(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_by_click(cx, handle, 200., 300., "abc");
    with_window(cx, handle, |window, cx| {
        key(window, "left", true, cx);
        key(window, "backspace", true, cx);
        key(window, "enter", true, cx);
        key(window, "home", true, cx);
        key(window, "shift-end", true, cx);
    });
    let (content, ..) = text_of(cx, handle, id);
    assert_eq!(content, "a\nc");
    let selection = read(cx, handle, |editor| {
        editor.text_edit.as_ref().unwrap().selection()
    });
    assert_eq!(selection, 2..3, "the second line is selected");
}

#[gpui_kit::test]
fn dragging_a_side_handle_makes_auto_width_text_wrap(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_by_click(cx, handle, 200., 300., "Wrapping");
    // A space, so the box can wrap between the words.
    with_editor(cx, handle, |editor, _| {
        editor.type_text(8..8, " text");
        editor.end_text_edit();
    });
    let (_, frame, _) = text_of(cx, handle, id);
    let (hx, hy) = Handle::Right.position(&frame);
    let from = at(cx, handle, hx, hy);
    let to = at(cx, handle, hx - frame.width * 0.4, hy);
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        );
    });
    let (_, wrapped, sizing) = text_of(cx, handle, id);
    assert_eq!(sizing, TextSizing::AutoHeight);
    assert!(wrapped.width < frame.width);
    assert!(
        wrapped.height > frame.height * 1.5,
        "two lines: {wrapped:?}"
    );
    assert_eq!(
        history(cx, handle).last().map(String::as_str),
        Some("Resize")
    );

    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    let (_, restored, sizing) = text_of(cx, handle, id);
    assert_eq!((restored, sizing), (frame, TextSizing::AutoWidth));
}

#[gpui_kit::test]
fn shift_keeps_the_ratio_of_a_corner_resize(cx: &mut TestAppContext) {
    let handle = open(cx);
    let from = at(cx, handle, 100., 100.);
    let to = at(cx, handle, 500., 300.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        );
        window.input("Box", cx);
        key(window, "escape", true, cx);
    });
    let id = only_element(cx, handle);
    let (_, frame, _) = text_of(cx, handle, id);
    let corner = at(cx, handle, frame.x + frame.width, frame.y + frame.height);
    let target = at(
        cx,
        handle,
        frame.x + frame.width + 200.,
        frame.y + frame.height + 10.,
    );
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    with_window(cx, handle, |window, cx| {
        drag_with(window, MouseButton::Left, corner, target, shift, cx);
    });
    let (_, resized, sizing) = text_of(cx, handle, id);
    assert_eq!(sizing, TextSizing::Fixed);
    let ratio = resized.width / resized.height;
    assert!(
        (ratio - frame.width / frame.height).abs() < 0.01,
        "{resized:?}"
    );
    assert!((resized.width - (frame.width + 200.)).abs() < 1.);
}

#[gpui_kit::test]
fn moving_snaps_to_the_slide_center_unless_ctrl_is_held(cx: &mut TestAppContext) {
    let handle = open(cx);
    let from = at(cx, handle, 100., 100.);
    let to = at(cx, handle, 300., 200.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        );
        window.input("Box", cx);
        key(window, "escape", true, cx);
    });
    let id = only_element(cx, handle);
    let (_, frame, _) = text_of(cx, handle, id);
    // Grab the middle, away from the handles, and drop the box center 3
    // units right of the slide center.
    let (cx0, cy0) = (frame.x + frame.width / 2., frame.y + frame.height / 2.);
    let grab = at(cx, handle, cx0, cy0);
    let drop = at(cx, handle, 803., cy0 + 137.);
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            grab,
            drop,
            Modifiers::default(),
            cx,
        );
    });
    let (_, moved, _) = text_of(cx, handle, id);
    assert!(
        (moved.x + moved.width / 2. - 800.).abs() < 0.01,
        "snapped: {moved:?}"
    );
    assert_eq!(history(cx, handle).last().map(String::as_str), Some("Move"));

    // 5 screen pixels: a drag, yet within snapping distance of the center.
    let shift = 5. / zoom(cx, handle);
    let middle = (moved.x + moved.width / 2., moved.y + moved.height / 2.);
    let grab = at(cx, handle, middle.0, middle.1);
    let drop = at(cx, handle, middle.0 + shift, middle.1);
    let ctrl = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    with_window(cx, handle, |window, cx| {
        drag_with(window, MouseButton::Left, grab, drop, ctrl, cx);
    });
    let (_, free, _) = text_of(cx, handle, id);
    assert!((free.x - (moved.x + shift)).abs() < 0.01, "{free:?}");
}

#[gpui_kit::test]
fn inspector_fields_and_buttons_format_the_selected_text(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_by_click(cx, handle, 200., 300., "Title");
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));

    // Type a font size and confirm with Enter.
    let input = read(cx, handle, |editor| {
        editor.inspector.input(Field::Size).clone()
    });
    with_window(cx, handle, |window, cx| {
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        input.update(cx, |state, cx| state.set_value("64", window, cx));
        window.press("enter", cx);
    });
    let (size, frame) = read(cx, handle, |editor| {
        let element = editor.presentation.element(id).unwrap();
        (element.as_text().unwrap().style.size, element.frame)
    });
    assert_eq!(size, 64.);
    assert!(frame.height > 64., "the auto-sized box grew: {frame:?}");

    with_window(cx, handle, |window, cx| {
        window.click("text-align-center", cx);
        window.click("text-underline", cx);
        window.click("sizing-fixed", cx);
    });
    let (align, underline, sizing) = read(cx, handle, |editor| {
        let element = editor.presentation.element(id).unwrap();
        let text = element.as_text().unwrap();
        (text.style.align, text.style.underline, text.sizing)
    });
    assert_eq!(
        (align, underline, sizing),
        (HAlign::Center, true, TextSizing::Fixed)
    );
    assert_eq!(
        history(cx, handle)[2..],
        ["Font size", "Text alignment", "Underline", "Resizing"]
    );
}

#[gpui_kit::test]
fn the_plus_button_adds_a_slide_and_shows_it(cx: &mut TestAppContext) {
    let handle = open(cx);
    with_window(cx, handle, |window, cx| window.click("add-slide", cx));
    let (count, current) = read(cx, handle, |editor| {
        (
            editor.presentation.slides.len(),
            editor.presentation.index_of(editor.current_slide),
        )
    });
    assert_eq!((count, current), (2, Some(1)));
    with_window(cx, handle, |window, cx| {
        window.click(("slide", 0usize), cx);
    });
    let current = read(cx, handle, |editor| {
        editor.presentation.index_of(editor.current_slide)
    });
    assert_eq!(current, Some(0));
}

#[gpui_kit::test]
fn the_history_tab_lists_the_steps(cx: &mut TestAppContext) {
    let handle = open(cx);
    create_by_click(cx, handle, 200., 300., "Hi");
    with_editor(cx, handle, |editor, _| editor.inspector_tab = 2);
    with_window(cx, handle, |window, _| {
        assert!(window.try_find(("history-step", 2usize)).is_some());
        assert!(window.try_find(("history-step", 3usize)).is_none());
    });
}

#[gpui_kit::test]
fn a_field_left_by_clicking_another_element_edits_the_first_one(cx: &mut TestAppContext) {
    let handle = open(cx);
    let first = create_by_click(cx, handle, 200., 200., "First");
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    let second_at = at(cx, handle, 200., 600.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-text", cx);
        click_at(window, second_at, 1, cx);
        window.input("Second", cx);
        key(window, "escape", true, cx);
    });
    let second = read(cx, handle, |editor| editor.selection.unwrap());
    let first_at = read(cx, handle, |editor| {
        let frame = editor.presentation.element(first).unwrap().frame;
        editor
            .to_window(frame.x + frame.width / 2., frame.y + frame.height / 2.)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| click_at(window, first_at, 1, cx));
    assert_eq!(read(cx, handle, |editor| editor.selection), Some(first));

    // Type a size, then click the other box without pressing Enter.
    let input = read(cx, handle, |editor| {
        editor.inspector.input(Field::Size).clone()
    });
    let second_middle = read(cx, handle, |editor| {
        let frame = editor.presentation.element(second).unwrap().frame;
        editor
            .to_window(frame.x + frame.width / 2., frame.y + frame.height / 2.)
            .unwrap()
    });
    // Each step is its own event cycle, as with a real keyboard and mouse.
    with_window(cx, handle, |window, cx| {
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        input.update(cx, |state, cx| state.set_value("", window, cx));
    });
    with_window(cx, handle, |window, cx| window.input("50", cx));
    with_window(cx, handle, |window, cx| {
        click_at(window, second_middle, 1, cx)
    });
    let sizes = read(cx, handle, |editor| {
        let size = |id| {
            editor
                .presentation
                .element(id)
                .unwrap()
                .as_text()
                .unwrap()
                .style
                .size
        };
        (size(first), size(second), editor.selection)
    });
    assert_eq!(sizes, (50., 32., Some(second)));
}
