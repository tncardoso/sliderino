//! End-to-end tests of the Slides tab: selecting slides, the menu actions,
//! and dragging thumbnails to change the order.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, TestAppContext, WindowHandle};
use gpui_kit::{point, px};

use crate::document::{Operation, tests::with_inter};
use crate::document::{Presentation, Slide, SlideId};
use crate::editor::EditorView;
use crate::ui::test_support::{click_with, open_with, read, with_editor, with_window};

/// An editor on a presentation with `count` empty slides, ids 1 to `count`.
fn deck(cx: &mut TestAppContext, count: u64) -> WindowHandle<EditorView> {
    let mut presentation: Presentation = with_inter();
    for _ in 1..count {
        let id = presentation.new_slide_id();
        presentation
            .apply(Operation::AddSlide {
                index: usize::MAX,
                slide: Slide::new(id),
            })
            .unwrap();
    }
    open_with(cx, presentation)
}

fn order(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<u64> {
    read(cx, handle, |editor| {
        editor.presentation.slides.iter().map(|s| s.id.0).collect()
    })
}

/// The selected slides in presentation order, and the slide on the canvas.
fn selected(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> (Vec<u64>, u64) {
    read(cx, handle, |editor| {
        (
            editor.selected_slides().iter().map(|s| s.0).collect(),
            editor.current_slide.0,
        )
    })
}

/// Clicks the thumbnail at `index` (from 0) in the list.
fn click_thumb(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    index: usize,
    modifiers: Modifiers,
) {
    with_window(cx, handle, |window, cx| {
        let center = window.find(("slide", index)).bounds().center();
        click_with(window, center, 1, modifiers, cx);
    });
}

/// Drags the thumbnail at `from` to the top (`before`) or bottom edge of
/// the thumbnail at `to`.
fn drag_thumb(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    from: usize,
    to: usize,
    before: bool,
) {
    with_window(cx, handle, |window, cx| {
        let start = window.find(("slide", from)).bounds().center();
        let target = window.find(("slide", to)).bounds();
        let y = if before {
            target.top() + px(4.)
        } else {
            target.bottom() - px(4.)
        };
        window.drag(start, point(target.center().x, y), cx);
    });
}

#[gpui_kit::test]
fn ctrl_and_shift_clicks_select_several_slides(cx: &mut TestAppContext) {
    let handle = deck(cx, 5);
    click_thumb(cx, handle, 1, Modifiers::default());
    assert_eq!(selected(cx, handle), (vec![2], 2));
    click_thumb(cx, handle, 3, Modifiers::control());
    assert_eq!(selected(cx, handle), (vec![2, 4], 4));
    // Shift selects from the last Ctrl click.
    click_thumb(cx, handle, 4, Modifiers::shift());
    assert_eq!(selected(cx, handle), (vec![4, 5], 5));
    click_thumb(cx, handle, 0, Modifiers::control());
    assert_eq!(selected(cx, handle), (vec![1, 4, 5], 1));
    // Ctrl removes a slide; the canvas shows another selected one.
    click_thumb(cx, handle, 0, Modifiers::control());
    assert_eq!(selected(cx, handle).0, [4, 5]);
    // A plain click keeps only one.
    click_thumb(cx, handle, 2, Modifiers::default());
    assert_eq!(selected(cx, handle), (vec![3], 3));
}

#[gpui_kit::test]
fn a_right_click_outside_the_selection_selects_only_that_slide(cx: &mut TestAppContext) {
    // A real right click opens the menu, which gpui keeps past the end of the
    // test; the handler of the thumbnail calls this method.
    let handle = deck(cx, 3);
    click_thumb(cx, handle, 0, Modifiers::default());
    click_thumb(cx, handle, 1, Modifiers::control());
    // Inside the selection: the menu acts on both.
    with_editor(cx, handle, |editor, _| editor.right_click_slide(SlideId(1)));
    assert_eq!(selected(cx, handle).0, [1, 2]);
    with_editor(cx, handle, |editor, _| editor.right_click_slide(SlideId(3)));
    assert_eq!(selected(cx, handle), (vec![3], 3));
}

#[gpui_kit::test]
fn dragging_thumbnails_moves_the_selected_slides_together(cx: &mut TestAppContext) {
    let handle = deck(cx, 5);
    // One slide that is not selected: it moves alone and becomes selected.
    drag_thumb(cx, handle, 0, 2, false);
    assert_eq!(order(cx, handle), [2, 3, 1, 4, 5]);
    assert_eq!(selected(cx, handle), (vec![1], 1));

    click_thumb(cx, handle, 0, Modifiers::default());
    click_thumb(cx, handle, 3, Modifiers::control());
    // Slides 2 and 4 before slide 5.
    drag_thumb(cx, handle, 3, 4, true);
    assert_eq!(order(cx, handle), [3, 1, 2, 4, 5]);
    assert_eq!(selected(cx, handle).0, [2, 4]);

    with_editor(cx, handle, |editor, _| editor.undo());
    assert_eq!(order(cx, handle), [2, 3, 1, 4, 5]);
    assert_eq!(selected(cx, handle).0, [2, 4]);
}

#[gpui_kit::test]
fn delete_removes_the_selected_slides_and_undo_restores_them(cx: &mut TestAppContext) {
    let handle = deck(cx, 4);
    click_thumb(cx, handle, 1, Modifiers::default());
    click_thumb(cx, handle, 2, Modifiers::control());
    with_editor(cx, handle, |editor, _| editor.delete_slides());
    assert_eq!(order(cx, handle), [1, 4]);
    // The slide that takes the place of the first deleted one shows.
    assert_eq!(selected(cx, handle), (vec![4], 4));

    with_editor(cx, handle, |editor, _| editor.undo());
    assert_eq!(order(cx, handle), [1, 2, 3, 4]);
    assert_eq!(selected(cx, handle).0, [2, 3]);
    with_editor(cx, handle, |editor, _| editor.redo());
    assert_eq!(order(cx, handle), [1, 4]);
}

#[gpui_kit::test]
fn deleting_all_slides_leaves_one_empty_slide(cx: &mut TestAppContext) {
    let handle = deck(cx, 2);
    click_thumb(cx, handle, 1, Modifiers::shift());
    assert_eq!(selected(cx, handle).0, [1, 2]);
    with_editor(cx, handle, |editor, _| editor.delete_slides());
    let (slides, current) = read(cx, handle, |editor| {
        (editor.presentation.slides.clone(), editor.current_slide)
    });
    assert_eq!(slides, [Slide::new(current)]);
    assert!(current.0 > 2);
    with_editor(cx, handle, |editor, _| editor.undo());
    assert_eq!(order(cx, handle), [1, 2]);
}

#[gpui_kit::test]
fn duplicate_puts_the_copies_after_the_selection_and_selects_them(cx: &mut TestAppContext) {
    let handle = deck(cx, 3);
    click_thumb(cx, handle, 0, Modifiers::default());
    click_thumb(cx, handle, 1, Modifiers::shift());
    with_editor(cx, handle, |editor, _| editor.duplicate_slides());
    let copies: Vec<SlideId> = read(cx, handle, |editor| {
        editor.presentation.slides[2..4]
            .iter()
            .map(|s| s.id)
            .collect()
    });
    assert_eq!(order(cx, handle)[..2], [1, 2]);
    assert_eq!(order(cx, handle)[4], 3);
    assert_eq!(
        selected(cx, handle),
        (copies.iter().map(|s| s.0).collect(), copies[0].0)
    );
    // New slide goes after the last selected slide.
    with_editor(cx, handle, |editor, _| editor.add_slide());
    let (ids, current) = read(cx, handle, |editor| {
        (
            editor
                .presentation
                .slides
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            editor.current_slide,
        )
    });
    assert_eq!(ids[4], current);
    assert_eq!(selected(cx, handle).0, [current.0]);
}
