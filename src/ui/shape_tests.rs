//! End-to-end tests of shapes on the canvas: hit testing, paint order,
//! marquee, moving, drawing with the shape tools and editing line ends.

use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle};

use crate::document::tests::{add_shape, add_text, with_inter};
use crate::document::{
    ElementId, ElementKind, EllipseElement, Fill, Frame, LineElement, Presentation,
    RectangleElement, Stroke, TextSizing,
};
use crate::editor::EditorView;
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
