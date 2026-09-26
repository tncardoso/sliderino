//! Benchmarks of dragging text boxes in a test window. They are ignored by
//! default and need the `perf` feature:
//!
//! ```sh
//! cargo test --lib --features perf bench_ -- --ignored --nocapture
//! ```
//!
//! Each pointer move is dispatched and followed by a frame (render,
//! prepaint, paint), and the time of the pair is recorded. The test text
//! system does not rasterize glyphs on the GPU, so the numbers are the CPU
//! cost of the editor per event.

use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, WindowHandle,
};

use crate::document::{Element, ElementId, Operation, Presentation, Slide};
use crate::editor::EditorView;
use crate::perf;
use crate::script;
use crate::snap::Handle;
use crate::ui::test_support::{open_with, read, with_window};

const MOVES: usize = 200;

/// The example scene, copied onto three slides.
fn example_scene() -> Presentation {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/debug/scenes/text.json"
    ))
    .unwrap();
    let mut presentation = Presentation::new();
    script::apply(&mut presentation, script::parse(&text).unwrap()).unwrap();
    copy_slide(&mut presentation, 2);
    presentation
}

/// Adds `copies` slides holding copies of the first slide's elements.
fn copy_slide(presentation: &mut Presentation, copies: usize) {
    let elements = presentation.slides[0].elements.clone();
    for _ in 0..copies {
        let id = presentation.new_slide_id();
        let mut slide = Slide::new(id);
        for element in &elements {
            let id = presentation.new_element_id();
            slide.elements.push(Element {
                id,
                ..element.clone()
            });
        }
        presentation
            .apply(Operation::AddSlide {
                index: usize::MAX,
                slide,
            })
            .unwrap();
    }
}

/// Ten slides of six boxes with long paragraphs, in every sizing mode.
fn heavy_scene() -> Presentation {
    let paragraph = "Activation grew faster than signups this quarter because the new \
        onboarding flow shortened the path to the first slide. Teams that invited a \
        colleague in the first week kept working at twice the rate of solo authors, \
        and agents now render more slides than people do.";
    let sizings = ["auto_width", "auto_height", "fixed"];
    let mut ops = vec![r#"{"op": "add_font", "face": {"family": "Inter"}}"#.to_string()];
    for box_ix in 0..6 {
        let (col, row) = (box_ix % 2, box_ix / 2);
        let sizing = sizings[box_ix % 3];
        let content = if sizing == "auto_width" {
            "Revenue per active team".to_string()
        } else {
            format!("{paragraph}\n{paragraph}")
        };
        ops.push(format!(
            r#"{{"op": "add_element", "slide": 1, "element": {{
                "id": {id}, "frame": {{"x": {x}, "y": {y}, "width": 700, "height": 200}},
                "text": {{"content": {content:?}, "sizing": "{sizing}",
                          "style": {{"size": 20, "align": "justify"}}}}}}}}"#,
            id = box_ix + 1,
            x = 60 + col * 760,
            y = 40 + row * 280,
        ));
    }
    let json = format!("[{}]", ops.join(","));
    let mut presentation = Presentation::new();
    script::apply(&mut presentation, script::parse(&json).unwrap()).unwrap();
    copy_slide(&mut presentation, 9);
    presentation
}

fn at(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

fn mouse_move(position: Point<Pixels>, pressed: bool) -> gpui_kit::PlatformInput {
    MouseMoveEvent {
        position,
        pressed_button: pressed.then_some(MouseButton::Left),
        modifiers: Modifiers::default(),
    }
    .to_platform_input()
}

/// Presses at `from`, moves to `to` in [`MOVES`] steps and releases. Returns
/// the time of each step: the event and the frame after it.
fn timed_drag(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    from: Point<Pixels>,
    to: Point<Pixels>,
) -> Vec<Duration> {
    with_window(cx, handle, |window, cx| {
        window.dispatch_event(mouse_move(from, false), cx);
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: from,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        perf::enable();
        let mut times = Vec::with_capacity(MOVES);
        for step in 1..=MOVES {
            let t = step as f32 / MOVES as f32;
            let position = from + (to - from) * t;
            let start = Instant::now();
            window.dispatch_event(mouse_move(position, true), cx);
            window.render_frame(cx);
            times.push(start.elapsed());
        }
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position: to,
                modifiers: Modifiers::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        times
    })
}

fn print_stats(name: &str, times: &[Duration]) {
    let ms = |duration: Duration| duration.as_secs_f64() * 1e3;
    let mut sorted = times.to_vec();
    sorted.sort();
    let percentile = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    println!(
        "\n{name}: {} moves  first {:.2} ms  p50 {:.2} ms  p95 {:.2} ms  max {:.2} ms",
        times.len(),
        ms(times[0]),
        ms(percentile(0.5)),
        ms(percentile(0.95)),
        ms(*sorted.last().unwrap()),
    );
    print!("{}", perf::report());
}

/// Selects the element with a click, outside the measured drag.
fn select(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, id: ElementId) {
    let middle = read(cx, handle, |editor| {
        let frame = editor.presentation.element(id).unwrap().frame;
        editor
            .to_window(frame.x + frame.width / 2., frame.y + frame.height / 2.)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        crate::ui::test_support::click_at(window, middle, 1, cx);
    });
    assert_eq!(read(cx, handle, |editor| editor.selection), Some(id));
}

fn bench_move(cx: &mut TestAppContext, presentation: Presentation, name: &str) {
    let handle = open_with(cx, presentation);
    let id = ElementId(1);
    select(cx, handle, id);
    let frame = read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().frame
    });
    let (x, y) = (frame.x + frame.width / 2., frame.y + frame.height / 2.);
    let from = at(cx, handle, x, y);
    let to = at(cx, handle, x + 300., y + 200.);
    let times = timed_drag(cx, handle, from, to);
    print_stats(name, &times);
}

fn bench_resize(cx: &mut TestAppContext, presentation: Presentation, id: ElementId, name: &str) {
    let handle = open_with(cx, presentation);
    select(cx, handle, id);
    let frame = read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().frame
    });
    let (x, y) = Handle::Right.position(&frame);
    let from = at(cx, handle, x, y);
    let to = at(cx, handle, x - frame.width * 0.5, y);
    let times = timed_drag(cx, handle, from, to);
    print_stats(name, &times);
}

#[gpui_kit::test]
#[ignore = "benchmark: run with --features perf -- --ignored --nocapture"]
fn bench_drag_move(cx: &mut TestAppContext) {
    bench_move(cx, example_scene(), "move, example scene");
    bench_move(cx, heavy_scene(), "move, heavy scene");
}

#[gpui_kit::test]
#[ignore = "benchmark: run with --features perf -- --ignored --nocapture"]
fn bench_drag_resize(cx: &mut TestAppContext) {
    // Box 4 of the example scene and box 2 of the heavy one wrap by width.
    bench_resize(cx, example_scene(), ElementId(4), "resize, example scene");
    bench_resize(cx, heavy_scene(), ElementId(2), "resize, heavy scene");
}

/// A slide with one text box, built without the font catalog so that the
/// catalog stays cold.
fn cold_scene() -> Presentation {
    use crate::document::{ElementKind, FontFace, Frame, TextElement, TextSizing, TextStyle};
    let mut presentation = Presentation::new();
    let face = FontFace::new("Inter", 400, false);
    let data = crate::fonts::data(&face).unwrap();
    presentation
        .apply(Operation::AddFont { face, data })
        .unwrap();
    let slide = presentation.slides[0].id;
    presentation
        .apply(Operation::AddElement {
            slide,
            index: 0,
            element: Element {
                id: ElementId(1),
                frame: Frame {
                    x: 100.,
                    y: 100.,
                    width: 600.,
                    ..Frame::default()
                },
                kind: ElementKind::Text(TextElement {
                    content: "Activation grew faster than signups".into(),
                    style: TextStyle::default(),
                    sizing: TextSizing::AutoHeight,
                }),
            },
        })
        .unwrap();
    presentation
}

/// Run alone so that the font catalog is cold:
/// `cargo test --lib --features perf bench_first_selection -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore = "benchmark: run with --features perf -- --ignored --nocapture"]
fn bench_first_selection(cx: &mut TestAppContext) {
    let handle = open_with(cx, cold_scene());
    perf::enable();
    let start = Instant::now();
    select(cx, handle, ElementId(1));
    print_stats("first selection", &[start.elapsed()]);

    perf::enable();
    let start = Instant::now();
    select(cx, handle, ElementId(1));
    print_stats("second selection", &[start.elapsed()]);
}

/// Scanning the system fonts, which the editor does on a background thread.
#[test]
#[ignore = "benchmark: run with --features perf -- --ignored --nocapture"]
fn bench_catalog_load() {
    let start = Instant::now();
    let families = crate::fonts::catalog().families().len();
    println!(
        "\ncatalog load: {families} families in {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );
}
