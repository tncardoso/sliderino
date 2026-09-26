//! End-to-end tests of the Hierarchy tab: rows, selection, collapsing,
//! the eye and the lock, renaming and dragging layers.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, TestAppContext, WindowHandle};

use crate::document::tests::{add_text, with_inter};
use crate::document::{ElementId, Frame, Operation, TextSizing};
use crate::editor::EditorView;
use crate::ui::test_support::{click_with, key, open_with, read, with_editor, with_window};

struct Scene {
    handle: WindowHandle<EditorView>,
    group: ElementId,
    first: ElementId,
    second: ElementId,
    other: ElementId,
}

/// Two texts in a group under a third text, with the Hierarchy tab open.
fn scene(cx: &mut TestAppContext) -> Scene {
    let mut presentation = with_inter();
    let frame = |x: f32, y: f32| Frame {
        x,
        y,
        ..Frame::default()
    };
    let first = add_text(
        &mut presentation,
        "One",
        TextSizing::AutoWidth,
        frame(100., 100.),
    );
    let second = add_text(
        &mut presentation,
        "Two",
        TextSizing::AutoWidth,
        frame(400., 300.),
    );
    let other = add_text(
        &mut presentation,
        "Other",
        TextSizing::AutoWidth,
        frame(900., 600.),
    );
    let group = presentation.new_element_id();
    let operations = presentation
        .group_operations(group, &[first, second])
        .unwrap();
    presentation.apply(Operation::Batch(operations)).unwrap();
    let handle = open_with(cx, presentation);
    with_editor(cx, handle, |editor, _| editor.library_tab = 2);
    Scene {
        handle,
        group,
        first,
        second,
        other,
    }
}

fn row(id: ElementId) -> (&'static str, usize) {
    ("layer", id.0 as usize)
}

fn selection(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<ElementId> {
    read(cx, handle, |editor| editor.selection.clone())
}

fn click_row(cx: &mut TestAppContext, s: &Scene, id: ElementId, modifiers: Modifiers) {
    with_window(cx, s.handle, |window, cx| {
        let center = window.find(row(id)).bounds().center();
        click_with(window, center, 1, modifiers, cx);
    });
}

/// Top of each row, to check their order.
fn top(cx: &mut TestAppContext, s: &Scene, id: ElementId) -> f32 {
    with_window(cx, s.handle, |window, _| {
        f32::from(window.find(row(id)).bounds().top())
    })
}

#[gpui_kit::test]
fn rows_list_layers_topmost_first_and_select_them(cx: &mut TestAppContext) {
    let s = scene(cx);
    let order = [s.other, s.group, s.second, s.first];
    let tops: Vec<f32> = order.iter().map(|id| top(cx, &s, *id)).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");

    click_row(cx, &s, s.first, Modifiers::default());
    assert_eq!(selection(cx, s.handle), [s.first]);
    click_row(cx, &s, s.other, Modifiers::control());
    assert_eq!(selection(cx, s.handle), [s.first, s.other]);
    click_row(cx, &s, s.first, Modifiers::shift());
    assert_eq!(selection(cx, s.handle), order);
}

#[gpui_kit::test]
fn a_collapsed_group_hides_its_children(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_window(cx, s.handle, |window, cx| {
        window.click(("layer-chevron", s.group.0 as usize), cx)
    });
    let shown = with_window(cx, s.handle, |window, _| {
        window.try_find(row(s.first)).is_some()
    });
    assert!(!shown);
}

#[gpui_kit::test]
fn the_eye_and_the_lock_toggle_the_layer(cx: &mut TestAppContext) {
    let s = scene(cx);
    let flags = |cx: &mut TestAppContext| {
        read(cx, s.handle, |editor| {
            let element = editor.presentation.element(s.other).unwrap();
            (element.hidden, element.locked)
        })
    };
    with_window(cx, s.handle, |window, cx| {
        window.hover(row(s.other), cx);
        window.click(("layer-eye", s.other.0 as usize), cx);
    });
    assert_eq!(flags(cx), (true, false));
    with_window(cx, s.handle, |window, cx| {
        window.hover(row(s.other), cx);
        window.click(("layer-lock", s.other.0 as usize), cx);
    });
    assert_eq!(flags(cx), (true, true));
    with_editor(cx, s.handle, |editor, _| editor.undo());
    assert_eq!(flags(cx), (true, false));
}

#[gpui_kit::test]
fn hovering_a_row_outlines_its_layer(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_window(cx, s.handle, |window, cx| window.hover(row(s.second), cx));
    assert_eq!(
        read(cx, s.handle, |editor| editor.hovered_layer),
        Some(s.second)
    );
}

#[gpui_kit::test]
fn renaming_a_layer_is_one_undo_step(cx: &mut TestAppContext) {
    let s = scene(cx);
    s.handle
        .update(cx, |editor, window, cx| {
            editor.start_rename(s.group, window, cx)
        })
        .unwrap();
    with_window(cx, s.handle, |window, cx| {
        window.render_frame(cx);
        window.input("Header", cx);
        key(window, "enter", true, cx);
    });
    let name = |cx: &mut TestAppContext| {
        read(cx, s.handle, |editor| {
            editor.presentation.element(s.group).unwrap().name.clone()
        })
    };
    assert_eq!(name(cx).as_deref(), Some("Header"));
    assert!(read(cx, s.handle, |editor| editor.renaming.is_none()));
    with_editor(cx, s.handle, |editor, _| editor.undo());
    assert_eq!(name(cx), None);
}

#[gpui_kit::test]
fn dragging_a_row_onto_a_group_moves_the_layer_into_it(cx: &mut TestAppContext) {
    let s = scene(cx);
    with_window(cx, s.handle, |window, cx| {
        window.drag_to(row(s.other), row(s.group), cx)
    });
    let children: Vec<ElementId> = read(cx, s.handle, |editor| {
        editor
            .presentation
            .element(s.group)
            .unwrap()
            .children()
            .iter()
            .map(|child| child.id)
            .collect()
    });
    assert_eq!(children, [s.first, s.second, s.other]);
    assert_eq!(selection(cx, s.handle), [s.other]);
}
