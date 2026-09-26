//! End-to-end tests of groups on the canvas: Figma-like selection, multiple
//! selection, marquee, moving and resizing groups, and the group keys.

use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle};

use crate::document::tests::{add_text, with_inter};
use crate::document::{ElementId, Frame, LayerPatch, Operation, TextSizing};
use crate::editor::EditorView;
use crate::ui::test_support::{
    click_at, click_with, drag_with, key, open_with, read, with_editor, with_window,
};

struct Scene {
    handle: WindowHandle<EditorView>,
    group: ElementId,
    first: ElementId,
    second: ElementId,
    /// A text outside the group.
    other: ElementId,
}

fn frame(x: f32, y: f32, width: f32) -> Frame {
    Frame {
        x,
        y,
        width,
        ..Frame::default()
    }
}

/// Two texts in a group and a third text alone.
fn scene(cx: &mut TestAppContext) -> Scene {
    let mut presentation = with_inter();
    let first = add_text(
        &mut presentation,
        "One",
        TextSizing::AutoHeight,
        frame(100., 100., 200.),
    );
    let second = add_text(
        &mut presentation,
        "Two",
        TextSizing::AutoWidth,
        frame(400., 300., 0.),
    );
    let other = add_text(
        &mut presentation,
        "Other",
        TextSizing::AutoWidth,
        frame(900., 600., 0.),
    );
    let group = presentation.new_element_id();
    let operations = presentation
        .group_operations(group, &[first, second])
        .unwrap();
    presentation.apply(Operation::Batch(operations)).unwrap();
    let handle = open_with(cx, presentation);
    Scene {
        handle,
        group,
        first,
        second,
        other,
    }
}

/// Window position of a point inside an element, near its top-left corner.
fn inside(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
) -> Point<Pixels> {
    read(cx, handle, |editor| {
        let frame = editor.presentation.element(id).unwrap().frame;
        editor.to_window(frame.x + 8., frame.y + 8.).unwrap()
    })
}

fn at(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

fn selection(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<ElementId> {
    read(cx, handle, |editor| editor.selection.clone())
}

fn frame_of(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, id: ElementId) -> Frame {
    read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().frame
    })
}

fn click(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    position: Point<Pixels>,
    count: usize,
    modifiers: Modifiers,
) {
    with_window(cx, handle, |window, cx| {
        click_with(window, position, count, modifiers, cx)
    });
}

fn press_key(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, source: &str) {
    with_window(cx, handle, |window, cx| key(window, source, true, cx));
}

#[gpui_kit::test]
fn a_click_on_a_child_selects_its_group_and_a_double_click_enters(cx: &mut TestAppContext) {
    let s = scene(cx);
    let position = inside(cx, s.handle, s.second);
    with_window(cx, s.handle, |window, cx| click_at(window, position, 1, cx));
    assert_eq!(selection(cx, s.handle), [s.group]);
    with_window(cx, s.handle, |window, cx| click_at(window, position, 2, cx));
    assert_eq!(selection(cx, s.handle), [s.second]);
    // Inside the group, a click selects a sibling directly.
    let position = inside(cx, s.handle, s.first);
    with_window(cx, s.handle, |window, cx| click_at(window, position, 1, cx));
    assert_eq!(selection(cx, s.handle), [s.first]);
    // Escape goes back up to the group.
    press_key(cx, s.handle, "escape");
    assert_eq!(selection(cx, s.handle), [s.group]);
}

#[gpui_kit::test]
fn ctrl_click_selects_the_leaf_and_shift_click_adds(cx: &mut TestAppContext) {
    let s = scene(cx);
    let second = inside(cx, s.handle, s.second);
    click(cx, s.handle, second, 1, Modifiers::control());
    assert_eq!(selection(cx, s.handle), [s.second]);
    let other = inside(cx, s.handle, s.other);
    click(cx, s.handle, other, 1, Modifiers::shift());
    assert_eq!(selection(cx, s.handle), [s.second, s.other]);
    click(cx, s.handle, other, 1, Modifiers::shift());
    assert_eq!(selection(cx, s.handle), [s.second]);
}

#[gpui_kit::test]
fn dragging_a_group_moves_its_children_as_one_step(cx: &mut TestAppContext) {
    let s = scene(cx);
    let before = (
        frame_of(cx, s.handle, s.first),
        frame_of(cx, s.handle, s.second),
    );
    let from = inside(cx, s.handle, s.second);
    let to = at(cx, s.handle, 408. + 50., 308. + 30.);
    with_window(cx, s.handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    let first = frame_of(cx, s.handle, s.first);
    let second = frame_of(cx, s.handle, s.second);
    let (dx, dy) = (first.x - before.0.x, first.y - before.0.y);
    assert!(dx > 40. && dy > 20., "the group moved: {first:?}");
    assert_eq!((second.x - before.1.x, second.y - before.1.y), (dx, dy));
    assert_eq!(selection(cx, s.handle), [s.group]);
    with_editor(cx, s.handle, |editor, _| editor.undo());
    assert_eq!(frame_of(cx, s.handle, s.first), before.0);
    assert_eq!(frame_of(cx, s.handle, s.second), before.1);
}

#[gpui_kit::test]
fn resizing_a_group_from_a_handle_scales_its_children(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_editor(cx, s.handle, |editor, _| editor.selection = vec![s.group]);
    let group = frame_of(cx, s.handle, s.group);
    let first = frame_of(cx, s.handle, s.first);
    let from = at(cx, s.handle, group.x + group.width, group.y + group.height);
    let to = at(
        cx,
        s.handle,
        group.x + group.width * 2.,
        group.y + group.height,
    );
    with_window(cx, s.handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::control(),
            cx,
        )
    });
    let resized = frame_of(cx, s.handle, s.first);
    assert!((resized.width - first.width * 2.).abs() < 1., "{resized:?}");
}

#[gpui_kit::test]
fn ctrl_g_groups_the_selection_and_ctrl_shift_g_ungroups(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_editor(cx, s.handle, |editor, _| {
        editor.selection = vec![s.group, s.other]
    });
    press_key(cx, s.handle, "ctrl-g");
    let outer = read(cx, s.handle, |editor| {
        let slide = editor.current_slide();
        assert_eq!(slide.elements.len(), 1);
        slide.elements[0].id
    });
    assert_eq!(selection(cx, s.handle), [outer]);
    press_key(cx, s.handle, "ctrl-shift-g");
    assert_eq!(selection(cx, s.handle), [s.group, s.other]);
    let top: Vec<ElementId> = read(cx, s.handle, |editor| {
        editor
            .current_slide()
            .elements
            .iter()
            .map(|e| e.id)
            .collect()
    });
    assert_eq!(top, [s.group, s.other]);
}

#[gpui_kit::test]
fn deleting_the_children_of_a_group_removes_the_group(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_editor(cx, s.handle, |editor, _| editor.selection = vec![s.group]);
    press_key(cx, s.handle, "enter");
    assert_eq!(selection(cx, s.handle), [s.first, s.second]);
    press_key(cx, s.handle, "delete");
    let group = read(cx, s.handle, |editor| {
        editor.presentation.element(s.group).is_some()
    });
    assert!(!group, "the emptied group is removed");
    with_editor(cx, s.handle, |editor, _| editor.undo());
    assert_eq!(selection(cx, s.handle), [s.first, s.second]);
    let children = read(cx, s.handle, |editor| {
        editor
            .presentation
            .element(s.group)
            .unwrap()
            .children()
            .len()
    });
    assert_eq!(
        children, 2,
        "one undo brings back the group and its children"
    );
}

#[gpui_kit::test]
fn a_marquee_selects_what_it_touches_at_the_top_level(cx: &mut TestAppContext) {
    let s = scene(cx);
    let from = at(cx, s.handle, 380., 280.);
    let to = at(cx, s.handle, 1000., 650.);
    with_window(cx, s.handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    assert_eq!(selection(cx, s.handle), [s.group, s.other]);
    with_window(cx, s.handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::control(),
            cx,
        )
    });
    assert_eq!(selection(cx, s.handle), [s.second, s.other]);
}

#[gpui_kit::test]
fn clicks_ignore_hidden_and_locked_elements(cx: &mut TestAppContext) {
    let s = scene(cx);
    let position = inside(cx, s.handle, s.other);
    for patch in [
        LayerPatch {
            hidden: Some(true),
            ..LayerPatch::default()
        },
        LayerPatch {
            hidden: Some(false),
            locked: Some(true),
            ..LayerPatch::default()
        },
    ] {
        with_editor(cx, s.handle, |editor, _| {
            editor.commit("Layer", Operation::SetLayer { id: s.other, patch }, vec![]);
        });
        with_window(cx, s.handle, |window, cx| click_at(window, position, 1, cx));
        assert_eq!(selection(cx, s.handle), []);
    }
}
