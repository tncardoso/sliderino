//! End-to-end tests of shapes on the canvas and in the inspector: hit
//! testing, paint order, marquee, moving, drawing with the shape tools,
//! editing line ends and the style fields.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Modifiers, MouseButton, Pixels, Point, TestAppContext, WindowHandle};

use crate::document::tests::{add_shape, add_text, with_inter};
use crate::document::{
    Arrowhead, ElementId, ElementKind, EllipseElement, Fill, Frame, HeadKind, HeadSize,
    LineElement, Presentation, RectangleElement, Stroke, TextSizing,
};
use crate::editor::{EditorView, Tool};
use crate::ui::inspector::Field;
use crate::ui::shape_inspector::{FillType, ShapeField};
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
        key(window, "ctrl-z", true, cx);
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

fn type_shape_field(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    field: ShapeField,
    text: &str,
) {
    handle
        .update(cx, |editor, window, cx| {
            let input = editor.shape_inspector.input(field).clone();
            input.update(cx, |state, cx| {
                state.set_value(text.to_string(), window, cx)
            });
            editor.commit_shape_field(field, window, cx);
        })
        .unwrap();
    with_window(cx, handle, |window, cx| window.render_frame(cx));
}

fn shape_field(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    field: ShapeField,
) -> String {
    handle
        .update(cx, |editor, _, cx| {
            editor
                .shape_inspector
                .input(field)
                .read(cx)
                .value()
                .to_string()
        })
        .unwrap()
}

fn history(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<String> {
    read(cx, handle, |editor| {
        editor.history.done().map(String::from).collect()
    })
}

fn kind(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, id: ElementId) -> ElementKind {
    read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().kind.clone()
    })
}

fn stroked() -> ElementKind {
    ElementKind::Rectangle(RectangleElement {
        stroke: Some(Stroke::default()),
        ..RectangleElement::default()
    })
}

fn select(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, ids: Vec<ElementId>) {
    crate::ui::test_support::with_editor(cx, handle, |editor, _| editor.selection = ids);
}

#[gpui_kit::test]
fn the_stroke_width_field_is_one_undo_step(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), stroked());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    assert_eq!(shape_field(cx, handle, ShapeField::StrokeWidth), "4");
    type_shape_field(cx, handle, ShapeField::StrokeWidth, "12");
    assert_eq!(kind(cx, handle, id).stroke().unwrap().width, 12.);
    assert_eq!(history(cx, handle), vec!["Stroke width"]);
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert_eq!(kind(cx, handle, id).stroke().unwrap().width, 4.);
}

#[gpui_kit::test]
fn removing_the_fill_undoes_back_to_the_solid_fill(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::None)
    });
    assert_eq!(kind(cx, handle, id).fill(), Some(&Fill::None));
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert_eq!(kind(cx, handle, id).fill(), Some(&Fill::default()));
}

#[gpui_kit::test]
fn gradients_keep_two_to_ten_stops(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    let stops = |cx: &mut TestAppContext| {
        kind(cx, handle, id)
            .fill()
            .unwrap()
            .stops()
            .map_or(0, <[_]>::len)
    };
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Radial);
        editor.remove_gradient_stop(0);
    });
    assert_eq!(stops(cx), 2);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        for _ in 0..12 {
            editor.add_gradient_stop();
        }
    });
    assert_eq!(stops(cx), crate::style::MAX_STOPS);
    let fill = kind(cx, handle, id).fill().unwrap().clone();
    assert!(fill.validate().is_ok());
}

#[gpui_kit::test]
fn line_ends_change_kind_size_and_side(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = a_line(&mut presentation);
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_arrowhead(false, Some(HeadKind::Triangle), None);
        editor.set_arrowhead(false, None, Some(HeadSize::Large));
        editor.swap_arrowheads();
    });
    let ElementKind::Line(line) = kind(cx, handle, id) else {
        panic!("a line");
    };
    assert_eq!(
        line.start,
        Arrowhead::new(HeadKind::Triangle, HeadSize::Large)
    );
    assert!(line.end.is_none());
    assert_eq!(
        history(cx, handle),
        vec!["Line end", "Line end", "Swap line ends"]
    );
}

#[gpui_kit::test]
fn several_shapes_show_mixed_values_and_change_together(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let thin = add_shape(&mut presentation, frame(100., 100., 200., 200.), stroked());
    let line = a_line(&mut presentation);
    crate::document::tests::set_style(
        &mut presentation,
        thin,
        crate::document::ShapeStylePatch {
            stroke: Some(Some(Stroke {
                width: 1.,
                ..Stroke::default()
            })),
            ..Default::default()
        },
    );
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![thin, line]);
    assert_eq!(shape_field(cx, handle, ShapeField::StrokeWidth), "Mixed");
    type_shape_field(cx, handle, ShapeField::StrokeWidth, "7");
    assert_eq!(kind(cx, handle, thin).stroke().unwrap().width, 7.);
    assert_eq!(kind(cx, handle, line).stroke().unwrap().width, 7.);
    assert_eq!(history(cx, handle), vec!["Stroke width"]);
    assert_eq!(shape_field(cx, handle, ShapeField::StrokeWidth), "7");
    // Only the rectangle has a fill: the fill section is left out, and a
    // change of fill skips the line.
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_dash(crate::document::Dash::Dotted)
    });
    assert_eq!(
        kind(cx, handle, line).stroke().unwrap().dash,
        crate::document::Dash::Dotted
    );
}

#[gpui_kit::test]
fn the_opacity_field_sets_the_opacity_of_each_element(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let a = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let b = add_shape(&mut presentation, frame(400., 100., 200., 200.), stroked());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![a, b]);
    handle
        .update(cx, |editor, window, cx| {
            let input = editor.inspector.input(Field::Opacity).clone();
            input.update(cx, |state, cx| {
                state.set_value("40".to_string(), window, cx)
            });
            editor.commit_field(Field::Opacity, window, cx);
        })
        .unwrap();
    let opacity = |cx: &mut TestAppContext, id| {
        read(cx, handle, |editor| {
            editor.presentation.element(id).unwrap().opacity
        })
    };
    assert_eq!((opacity(cx, a), opacity(cx, b)), (0.4, 0.4));
    assert_eq!(history(cx, handle), vec!["Opacity"]);
}

#[gpui_kit::test]
fn an_inserted_image_is_one_undo_step_fitted_to_the_slide(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    let id = crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor
            .insert_image_bytes(crate::images::tests::png(4000, 1000).to_vec(), None)
            .unwrap()
    });
    let element = only_element(cx, handle);
    assert_eq!(element.id, id);
    // 80% of the 1600-unit slide, with the proportions of the image.
    assert!(near(element.frame.width, 1280.) && near(element.frame.height, 320.));
    assert!(near(element.frame.center().0, 800.) && near(element.frame.center().1, 450.));
    assert_eq!(history(cx, handle), vec!["Insert image"]);
    // The same bytes again reuse the embedded image.
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor
            .insert_image_bytes(
                crate::images::tests::png(4000, 1000).to_vec(),
                Some((100., 100.)),
            )
            .unwrap();
    });
    assert_eq!(
        read(cx, handle, |editor| editor
            .presentation
            .images
            .iter()
            .count()),
        1
    );
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert!(read(cx, handle, |editor| editor
        .presentation
        .images
        .iter()
        .count()
        == 0));
}

#[gpui_kit::test]
fn a_pasted_image_is_inserted(cx: &mut TestAppContext) {
    let handle = open_with(cx, Presentation::new());
    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(
            &gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                crate::images::tests::png(10, 10).to_vec(),
            ),
        ))
    });
    with_window(cx, handle, |window, cx| {
        key(window, "secondary-v", true, cx)
    });
    let element = only_element(cx, handle);
    assert!(matches!(element.kind.fill(), Some(Fill::Image(_))));
}

#[gpui_kit::test]
fn shapes_take_an_image_fill_and_its_fit(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor
            .fill_with_image_bytes(crate::images::tests::png(8, 8).to_vec())
            .unwrap();
        editor.set_image_fit(crate::document::ImageFit::Stretch);
    });
    let Some(Fill::Image(fill)) = kind(cx, handle, id).fill().cloned() else {
        panic!("an image fill");
    };
    assert_eq!(fill.fit, crate::document::ImageFit::Stretch);
    assert_eq!(history(cx, handle), vec!["Image fill", "Image fit"]);
    let error = crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.fill_with_image_bytes(b"not an image".to_vec())
    });
    assert!(error.is_err());
}

#[gpui_kit::test]
fn shapes_take_a_shader_fill_and_its_playback(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Shader);
        editor.set_fill_start(crate::document::Start::OnClick);
        editor.set_fill_loop(false);
        editor
            .fill_with_image_bytes(crate::images::tests::png(8, 8).to_vec())
            .unwrap();
    });
    let Some(Fill::Image(image)) = kind(cx, handle, id).fill().cloned() else {
        panic!("an image fill");
    };
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Shader);
        editor.set_shader_channel(Some(image.id), None);
    });
    let Some(Fill::Shader(shader)) = kind(cx, handle, id).fill().cloned() else {
        panic!("a shader fill");
    };
    assert_eq!(shader.source.as_ref(), crate::shaders::DEFAULT_SOURCE);
    assert_eq!(shader.channel0, Some(image.id));
    assert_eq!(
        history(cx, handle),
        vec![
            "Fill",
            "Start",
            "Loop",
            "Image fill",
            "Fill",
            "Shader channel"
        ]
    );
}

#[gpui_kit::test]
fn the_preview_stops_when_the_shape_is_no_longer_selected(cx: &mut TestAppContext) {
    let mut presentation = Presentation::new();
    let id = add_shape(&mut presentation, frame(100., 100., 200., 200.), ellipse());
    let handle = open_with(cx, presentation);
    select(cx, handle, vec![id]);
    crate::ui::test_support::with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Shader);
        editor.toggle_preview();
        assert!(editor.previewing());
    });
    select(cx, handle, vec![]);
    cx.run_until_parked();
    let live = read(cx, handle, |editor| editor.preview.borrow().is_live(id));
    assert!(!live);
}
