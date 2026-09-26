//! End-to-end tests of shapes on the canvas: hit testing, paint order,
//! marquee, moving, drawing with the shape tools and editing line ends.

use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle};

use crate::document::tests::{add_shape, add_text, with_inter};
use crate::document::{
    ElementId, ElementKind, EllipseElement, Fill, Frame, LineElement, Presentation,
    RectangleElement, Stroke, TextSizing,
};
use crate::editor::{EditorView, Tool};
use crate::ui::test_support::{click_at, drag_with, key, open_with, read, with_window};

fn frame(x: f32, y: f32, width: f32, height: f32) -> Frame {
    Frame {
        x,
        y,
        width,
        height,
        rotation: 0.,
    }
}

fn at(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

fn click(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) {
    let position = at(cx, handle, x, y);
    with_window(cx, handle, |window, cx| click_at(window, position, 1, cx));
}

fn selection(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<ElementId> {
    read(cx, handle, |editor| editor.selection.clone())
}

fn element_frame(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
) -> Frame {
    read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().frame
    })
}

fn ellipse() -> ElementKind {
    ElementKind::Ellipse(EllipseElement::default())
}

#[gpui_kit::test]
fn clicks_hit_a_filled_ellipse_but_not_its_box_corners(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 400., 400.), ellipse());
    let handle = open_with(cx, presentation);
    click(cx, handle, 130., 130.);
    assert!(selection(cx, handle).is_empty());
    click(cx, handle, 300., 300.);
    assert_eq!(selection(cx, handle), vec![id]);
}

#[gpui_kit::test]
fn an_empty_shape_is_hit_only_on_its_outline(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(
        &mut presentation,
        frame(100., 100., 400., 400.),
        ElementKind::Rectangle(RectangleElement {
            fill: Fill::None,
            stroke: Some(Stroke::default()),
            corner_radius: 0.,
        }),
    );
    let handle = open_with(cx, presentation);
    click(cx, handle, 300., 300.);
    assert!(selection(cx, handle).is_empty());
    click(cx, handle, 100., 300.);
    assert_eq!(selection(cx, handle), vec![id]);
}

#[gpui_kit::test]
fn a_thin_turned_line_is_hit_near_its_axis(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(
        &mut presentation,
        Frame::from_line((100., 100.), (500., 500.), 0.),
        ElementKind::Line(LineElement {
            stroke: Stroke {
                width: 1.,
                ..Stroke::default()
            },
            ..LineElement::default()
        }),
    );
    let handle = open_with(cx, presentation);
    click(cx, handle, 300., 302.);
    assert_eq!(selection(cx, handle), vec![id]);
    click(cx, handle, 300., 340.);
    assert!(selection(cx, handle).is_empty());
}

#[gpui_kit::test]
fn a_shape_over_a_text_takes_the_click(cx: &mut TestAppContext) {
    let mut presentation = with_inter();
    add_text(
        &mut presentation,
        "Under",
        TextSizing::AutoWidth,
        frame(100., 100., 0., 0.),
    );
    let over = add_shape(&mut presentation, frame(80., 80., 300., 200.), ellipse());
    let handle = open_with(cx, presentation);
    click(cx, handle, 230., 180.);
    assert_eq!(selection(cx, handle), vec![over]);
}

#[gpui_kit::test]
fn a_marquee_selects_a_line(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let line = add_shape(
        &mut presentation,
        Frame::from_line((300., 300.), (500., 300.), 0.),
        ElementKind::Line(LineElement::default()),
    );
    let handle = open_with(cx, presentation);
    let (from, to) = (at(cx, handle, 250., 250.), at(cx, handle, 400., 350.));
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
    assert_eq!(selection(cx, handle), vec![line]);
}

#[gpui_kit::test]
fn shapes_move_and_the_move_undoes(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    let (from, to) = (at(cx, handle, 200., 200.), at(cx, handle, 300., 250.));
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
    let moved = element_frame(cx, handle, id);
    assert!(
        (moved.x - 200.).abs() < 1. && (moved.y - 150.).abs() < 1.,
        "{moved:?}"
    );
    with_window(cx, handle, |window, cx| {
        key(window, "secondary-z", true, cx);
    });
    assert_eq!(element_frame(cx, handle, id), frame(100., 100., 200., 200.));
}

#[gpui_kit::test]
fn a_double_click_on_a_shape_keeps_it_selected(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    let position = at(cx, handle, 200., 200.);
    with_window(cx, handle, |window, cx| click_at(window, position, 2, cx));
    assert_eq!(selection(cx, handle), vec![id]);
    assert!(read(cx, handle, |editor| editor.text_edit.is_none()));
}

fn pick(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, tool: Tool) {
    crate::ui::test_support::with_editor(cx, handle, |editor, _| editor.active_tool = tool);
}

fn drag_on_slide(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    from: (f32, f32),
    to: (f32, f32),
    modifiers: Modifiers,
) {
    let (from, to) = (at(cx, handle, from.0, from.1), at(cx, handle, to.0, to.1));
    with_window(cx, handle, |window, cx| {
        drag_with(window, MouseButton::Left, from, to, modifiers, cx)
    });
}

fn only_element(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
) -> crate::document::Element {
    read(cx, handle, |editor| {
        let elements = &editor.current_slide().elements;
        assert_eq!(elements.len(), 1);
        elements[0].clone()
    })
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 2.
}

#[gpui_kit::test]
fn dragging_with_the_rectangle_tool_draws_one_undoable_rectangle(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    pick(cx, handle, Tool::Rectangle);
    drag_on_slide(cx, handle, (100., 100.), (400., 300.), Modifiers::default());
    let element = only_element(cx, handle);
    assert!(matches!(element.kind, ElementKind::Rectangle(_)));
    let frame = element.frame;
    assert!(near(frame.x, 100.) && near(frame.width, 300.) && near(frame.height, 200.));
    assert_eq!(selection(cx, handle), vec![element.id]);
    assert_eq!(read(cx, handle, |editor| editor.active_tool), Tool::Move);
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert!(read(cx, handle, |editor| editor
        .current_slide()
        .elements
        .is_empty()));
}

#[gpui_kit::test]
fn shift_draws_a_circle_and_alt_draws_from_the_center(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    pick(cx, handle, Tool::Ellipse);
    let modifiers = Modifiers {
        shift: true,
        alt: true,
        ..Modifiers::default()
    };
    drag_on_slide(cx, handle, (500., 400.), (600., 450.), modifiers);
    let frame = only_element(cx, handle).frame;
    assert!(
        near(frame.width, 200.) && near(frame.height, 200.),
        "{frame:?}"
    );
    assert!(near(frame.center().0, 500.) && near(frame.center().1, 400.));
}

#[gpui_kit::test]
fn a_click_with_a_shape_tool_makes_a_default_shape(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    pick(cx, handle, Tool::Line);
    click(cx, handle, 500., 400.);
    let element = only_element(cx, handle);
    assert!(element.is_line());
    let (start, end) = element.frame.line_ends();
    assert!(near(start.0, 450.) && near(end.0, 550.) && near(end.1, 400.));
}

#[gpui_kit::test]
fn shift_turns_a_drawn_line_to_45_degrees(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    pick(cx, handle, Tool::Line);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    drag_on_slide(cx, handle, (100., 100.), (400., 380.), shift);
    let frame = only_element(cx, handle).frame;
    assert_eq!(frame.rotation, 45.);
}

fn a_line(presentation: &mut Presentation) -> ElementId {
    add_shape(
        presentation,
        Frame::from_line((200., 200.), (600., 200.), 0.),
        ElementKind::Line(LineElement::default()),
    )
}

#[gpui_kit::test]
fn dragging_an_end_moves_it_and_keeps_the_other(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = a_line(&mut presentation);
    let handle = open_with(cx, presentation);
    click(cx, handle, 400., 200.);
    assert_eq!(selection(cx, handle), vec![id]);
    let snap_off = Modifiers::default();
    drag_on_slide(cx, handle, (600., 200.), (613., 517.), snap_off);
    let (start, end) = element_frame(cx, handle, id).line_ends();
    assert!(near(start.0, 200.) && near(start.1, 200.), "{start:?}");
    assert!(near(end.0, 613.) && near(end.1, 517.), "{end:?}");
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert_eq!(
        element_frame(cx, handle, id),
        Frame::from_line((200., 200.), (600., 200.), 0.)
    );
}

#[gpui_kit::test]
fn a_locked_line_has_no_end_handles(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = a_line(&mut presentation);
    presentation
        .apply(crate::document::Operation::SetLayer {
            id,
            patch: crate::document::LayerPatch {
                locked: Some(true),
                ..Default::default()
            },
        })
        .unwrap();
    let handle = open_with(cx, presentation);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| editor.selection = vec![id]);
    drag_on_slide(cx, handle, (600., 200.), (600., 500.), Modifiers::default());
    assert_eq!(
        element_frame(cx, handle, id),
        Frame::from_line((200., 200.), (600., 200.), 0.)
    );
}

#[gpui_kit::test]
fn shift_keeps_a_circle_round_while_it_resizes(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    click(cx, handle, 200., 200.);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    drag_on_slide(cx, handle, (300., 300.), (450., 350.), shift);
    let resized = element_frame(cx, handle, id);
    assert!(near(resized.width, resized.height), "{resized:?}");
    assert!(resized.width > 250., "{resized:?}");
}
