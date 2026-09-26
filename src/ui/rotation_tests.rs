//! End-to-end tests of rotation on the canvas and in the inspector: the
//! rotation zone outside the corners, angle snapping, rotated groups, hit
//! testing, resizing and text editing of rotated boxes.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle, point, px};

use crate::document::tests::{add_text, with_inter};
use crate::document::{ElementId, Frame, Operation, Presentation, TextSizing};
use crate::editor::EditorView;
use crate::snap::Handle;
use crate::ui::inspector::Field;
use crate::ui::test_support::{click_at, drag_with, open_with, read, with_editor, with_window};

fn text_at(
    presentation: &mut Presentation,
    content: &str,
    x: f32,
    y: f32,
    rotation: f32,
) -> ElementId {
    add_text(
        presentation,
        content,
        TextSizing::AutoWidth,
        Frame {
            x,
            y,
            rotation,
            ..Frame::default()
        },
    )
}

fn frame_of(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, id: ElementId) -> Frame {
    read(cx, handle, |editor| editor.frame_of(id).unwrap())
}

fn window_at(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    x: f32,
    y: f32,
) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

/// Selects `id` and drags from just outside the bottom-right corner of its
/// box, turning the pointer by `degrees` around the box center.
fn rotate_by(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
    degrees: f32,
    modifiers: Modifiers,
) {
    with_editor(cx, handle, |editor, _| editor.selection = vec![id]);
    let (frame, zoom) = read(cx, handle, |editor| {
        (editor.selection_box().unwrap(), editor.camera.unwrap().zoom)
    });
    let (cx_, cy_) = frame.center();
    let corner = frame.corners()[2];
    // A few screen pixels out along the diagonal, past the handle.
    let (dx, dy) = (corner.0 - cx_, corner.1 - cy_);
    let length = dx.hypot(dy);
    let out = 10. / zoom;
    let start = (
        cx_ + dx / length * (length + out),
        cy_ + dy / length * (length + out),
    );
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (sx, sy) = (start.0 - cx_, start.1 - cy_);
    let end = (cx_ + sx * cos - sy * sin, cy_ + sx * sin + sy * cos);
    let from = window_at(cx, handle, start.0, start.1);
    let to = window_at(cx, handle, end.0, end.1);
    with_window(cx, handle, |window, cx| {
        drag_with(window, MouseButton::Left, from, to, modifiers, cx)
    });
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

#[gpui_kit::test]
fn dragging_outside_a_corner_rotates_the_text(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = text_at(&mut presentation, "Hello", 500., 300., 0.);
    let handle = open_with(cx, presentation);
    let before = frame_of(cx, handle, id);
    rotate_by(cx, handle, id, 60., Modifiers::default());
    let after = frame_of(cx, handle, id);
    assert!((after.rotation - 60.).abs() < 1., "{after:?}");
    assert!(
        close(after.center().0, before.center().0) && close(after.center().1, before.center().1)
    );
    assert_eq!(
        read(cx, handle, |editor| editor
            .history
            .done()
            .last()
            .map(String::from)),
        Some("Rotate".into())
    );
}

#[gpui_kit::test]
fn shift_steps_by_15_degrees_and_quarter_turns_pull_the_angle(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = text_at(&mut presentation, "Hello", 500., 300., 0.);
    let handle = open_with(cx, presentation);
    rotate_by(cx, handle, id, 37., shift());
    assert_eq!(frame_of(cx, handle, id).rotation, 30.);
    rotate_by(cx, handle, id, 58.8, Modifiers::default());
    assert_eq!(
        frame_of(cx, handle, id).rotation,
        90.,
        "pulled to the quarter turn"
    );
    rotate_by(cx, handle, id, -10., Modifiers::default());
    assert!((frame_of(cx, handle, id).rotation - 80.).abs() < 1.);
}

#[gpui_kit::test]
fn rotating_a_group_turns_its_children_and_undoes(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let first = text_at(&mut presentation, "One", 300., 200., 0.);
    let second = text_at(&mut presentation, "Two", 600., 400., 0.);
    let group = presentation.new_element_id();
    let operations = presentation
        .group_operations(group, &[first, second])
        .unwrap();
    presentation.apply(Operation::Batch(operations)).unwrap();
    let handle = open_with(cx, presentation);
    let before = read(cx, handle, |editor| editor.presentation.slides.clone());
    rotate_by(cx, handle, group, 90., Modifiers::default());
    assert_eq!(frame_of(cx, handle, group).rotation, 90.);
    assert_eq!(frame_of(cx, handle, first).rotation, 90.);
    assert_eq!(frame_of(cx, handle, second).rotation, 90.);
    // The selection box of the group turns with it.
    let shown = read(cx, handle, |editor| editor.selection_box().unwrap());
    assert_eq!(shown.rotation, 90.);
    with_editor(cx, handle, |editor, _| editor.undo());
    let after = read(cx, handle, |editor| editor.presentation.slides.clone());
    assert_eq!(after, before);
}

#[gpui_kit::test]
fn a_click_hits_the_turned_shape_of_a_text(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = text_at(&mut presentation, "A long line of text", 400., 400., 90.);
    let handle = open_with(cx, presentation);
    let frame = frame_of(cx, handle, id);
    let (x, y) = frame.center();
    // Inside the unrotated frame, far to the right: empty once turned.
    let outside = window_at(cx, handle, x + frame.width / 2. - 4., y);
    with_window(cx, handle, |window, cx| click_at(window, outside, 1, cx));
    assert_eq!(read(cx, handle, |editor| editor.single_selection()), None);
    // Below the center, along the turned line.
    let inside = window_at(cx, handle, x, y + frame.width / 2. - 4.);
    with_window(cx, handle, |window, cx| click_at(window, inside, 1, cx));
    assert_eq!(
        read(cx, handle, |editor| editor.single_selection()),
        Some(id)
    );
}

#[gpui_kit::test]
fn a_marquee_selects_by_the_turned_shape(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = text_at(&mut presentation, "A long line of text", 400., 400., 45.);
    let handle = open_with(cx, presentation);
    let bounds = frame_of(cx, handle, id).bounds();
    // A small box in the empty top-left corner of the bounds.
    let from = window_at(cx, handle, bounds.x - 20., bounds.y - 20.);
    let to = window_at(cx, handle, bounds.x + 10., bounds.y + 10.);
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    assert!(read(cx, handle, |editor| editor.selection.is_empty()));
}

/// Types `text` into an inspector field and commits it.
fn type_field(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, field: Field, text: &str) {
    handle
        .update(cx, |editor, window, cx| {
            let input = editor.inspector.input(field).clone();
            input.update(cx, |state, cx| {
                state.set_value(text.to_string(), window, cx)
            });
            editor.commit_field(field, window, cx);
        })
        .unwrap();
    with_window(cx, handle, |window, cx| window.render_frame(cx));
}

fn field_value(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, field: Field) -> String {
    handle
        .update(cx, |editor, _, cx| {
            editor.inspector.input(field).read(cx).value().to_string()
        })
        .unwrap()
}

#[gpui_kit::test]
fn the_rotation_field_sets_the_final_angle(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let first = text_at(&mut presentation, "One", 300., 200., 10.);
    let second = text_at(&mut presentation, "Two", 600., 400., 20.);
    let handle = open_with(cx, presentation);

    with_editor(cx, handle, |editor, _| editor.selection = vec![first]);
    assert_eq!(field_value(cx, handle, Field::Rotation), "10°");
    let x = frame_of(cx, handle, first).x;
    type_field(cx, handle, Field::Rotation, "405°");
    assert_eq!(frame_of(cx, handle, first).rotation, 45.);
    assert_eq!(frame_of(cx, handle, first).x, x, "X is the unrotated frame");
    with_editor(cx, handle, |editor, _| editor.undo());
    assert_eq!(frame_of(cx, handle, first).rotation, 10.);

    with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first, second]
    });
    assert_eq!(field_value(cx, handle, Field::Rotation), "Mixed");
    let centers = [first, second].map(|id| frame_of(cx, handle, id).center());
    type_field(cx, handle, Field::Rotation, "30");
    for (id, center) in [first, second].into_iter().zip(centers) {
        let frame = frame_of(cx, handle, id);
        assert_eq!(frame.rotation, 30.);
        assert!(
            close(frame.center().0, center.0),
            "each turns around its own center"
        );
    }
    assert_eq!(field_value(cx, handle, Field::Rotation), "30°");
}

#[gpui_kit::test]
fn resizing_a_rotated_box_keeps_its_opposite_corner(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = add_text(
        &mut presentation,
        "Box",
        TextSizing::Fixed,
        Frame {
            x: 500.,
            y: 300.,
            width: 200.,
            height: 100.,
            rotation: 90.,
        },
    );
    let handle = open_with(cx, presentation);
    with_editor(cx, handle, |editor, _| editor.selection = vec![id]);
    let before = frame_of(cx, handle, id);
    let anchor = Handle::TopLeft.slide_position(&before);
    let (hx, hy) = Handle::BottomRight.slide_position(&before);
    let from = window_at(cx, handle, hx, hy);
    // Turned a quarter, the box widens down the slide.
    let to = from + point(px(0.), px(40.));
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    let after = frame_of(cx, handle, id);
    assert!(after.width > before.width + 10., "{after:?}");
    let kept = Handle::TopLeft.slide_position(&after);
    assert!(
        close(kept.0, anchor.0) && close(kept.1, anchor.1),
        "{kept:?} {anchor:?}"
    );
}

#[gpui_kit::test]
fn a_double_click_in_a_rotated_text_selects_the_word_under_it(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    let id = text_at(&mut presentation, "Hello world", 500., 300., 90.);
    let handle = open_with(cx, presentation);
    let frame = frame_of(cx, handle, id);
    // Near the end of the line, in the box's own axes.
    let (x, y) = frame.to_slide(frame.width * 0.75, frame.height / 2.);
    let at = window_at(cx, handle, x, y);
    with_window(cx, handle, |window, cx| click_at(window, at, 1, cx));
    assert_eq!(
        read(cx, handle, |editor| editor.single_selection()),
        Some(id)
    );
    with_window(cx, handle, |window, cx| click_at(window, at, 2, cx));
    let selected = read(cx, handle, |editor| {
        editor.text_edit.as_ref().map(|edit| edit.selection())
    });
    assert_eq!(selected, Some(6..11));
}
