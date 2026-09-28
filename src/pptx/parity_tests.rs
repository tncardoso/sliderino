//! Exports the debug scenes and checks that each deck is a consistent
//! package whose shapes match the elements (`parity.rs`).

use std::path::{Path, PathBuf};

use super::inspect::Deck;
use super::{Options, export, parity};
use crate::document::Presentation;

/// Scenes with videos, checked when the GStreamer plugins are installed.
const VIDEO_SCENES: &[&str] = &["pptx/media.json"];

/// The scenes to check, from `debug/scenes`.
const SCENES: &[&str] = &[
    "shapes.json",
    "text.json",
    "rotation.json",
    "custom-font.json",
    "system-font.json",
    "images.json",
    "tables.json",
    "pptx/groups.json",
    "pptx/media.json",
    "shaders.json",
];

fn scene_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("debug/scenes")
        .join(name)
}

pub(crate) fn load(name: &str) -> Presentation {
    let mut presentation = Presentation::new();
    let ops = crate::script::load(&scene_path(name)).unwrap();
    crate::script::apply(&mut presentation, ops).unwrap();
    presentation
}

fn check(name: &str) -> Vec<String> {
    let presentation = load(name);
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let mut problems = deck.check();
    problems.extend(parity::compare(&presentation, &deck).unwrap());
    problems
        .into_iter()
        .map(|problem| format!("{name}: {problem}"))
        .collect()
}

#[test]
fn every_scene_exports_with_parity() {
    let videos = crate::videos::tests::plugins_or_skip();
    let problems: Vec<String> = SCENES
        .iter()
        .filter(|name| videos || !VIDEO_SCENES.contains(name))
        .flat_map(|name| check(name))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn an_empty_presentation_is_a_consistent_deck() {
    let export = export(&Presentation::new(), &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    assert_eq!(deck.check(), Vec::<String>::new());
    assert_eq!(deck.slides().unwrap(), vec!["ppt/slides/slide1.xml"]);
}

#[test]
fn hidden_elements_are_left_out() {
    let presentation = load("pptx/groups.json");
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let slides = deck.summary().unwrap();
    let names: Vec<String> = slides[0]
        .shapes
        .iter()
        .flat_map(|shape| shape.walk())
        .map(|shape| shape.name.clone())
        .collect();
    assert!(names.contains(&"Rectangle 8".to_string()));
    assert!(!names.contains(&"Rectangle 9".to_string()));
}

#[test]
fn a_moved_child_is_a_difference() {
    let mut presentation = load("pptx/groups.json");
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let crate::document::ElementKind::Group(group) = &mut presentation.slides[0].elements[0].kind
    else {
        panic!("not a group");
    };
    group.children[0].frame.x += 1.;
    let problems = parity::compare(&presentation, &deck).unwrap();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("element 2: frame"), "{problems:?}");
}

#[test]
fn a_changed_line_is_a_difference() {
    let mut presentation = load("text.json");
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let crate::document::ElementKind::Text(text) = &mut presentation.slides[0].elements[1].kind
    else {
        panic!("not a text");
    };
    text.content = "Auto width grows with its texts".to_string();
    let problems = parity::compare(&presentation, &deck).unwrap();
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("element 2: lines")),
        "{problems:?}"
    );
}

#[test]
fn an_image_used_twice_is_one_part() {
    let presentation = load("images.json");
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let media: Vec<&String> = deck
        .names()
        .iter()
        .filter(|name| name.starts_with("ppt/media/"))
        .collect();
    assert_eq!(media.len(), 2, "{media:?}");
}

#[test]
fn a_changed_cell_is_a_difference() {
    let mut presentation = load("tables.json");
    let export = export(&presentation, &Options::default()).unwrap();
    let deck = Deck::read(&export.bytes).unwrap();
    let crate::document::ElementKind::Table(table) = &mut presentation.slides[0].elements[1].kind
    else {
        panic!("not a table");
    };
    table.rows[1][0].text.color = Some(crate::document::Rgb(0xFF0000));
    table.rows[0][0].fill = Some(crate::document::Fill::None);
    let problems = parity::compare(&presentation, &deck).unwrap();
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("cell 1,0: text")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("cell 0,0: fill")),
        "{problems:?}"
    );
}

#[test]
fn a_turned_table_is_a_group_of_its_parts() {
    let presentation = load("tables.json");
    let export = export(&presentation, &Options::default()).unwrap();
    assert!(
        export
            .warnings
            .iter()
            .any(|warning| warning.message.contains("cannot turn a table")),
    );
    let deck = Deck::read(&export.bytes).unwrap();
    let slides = deck.summary().unwrap();
    let turned = slides[0].shapes.last().unwrap();
    assert_eq!(turned.kind, super::inspect::ShapeKind::Group);
    assert_eq!(turned.name, "Table 5");
}

/// The scenes compared with LibreOffice, and the largest fraction of their
/// pixels that may differ: what was measured, with a small margin. The
/// scenes with videos differ most: LibreOffice draws a video without the
/// crop, outline, shape and rotation of its picture.
const VISUAL: &[(&str, f32)] = &[
    ("shapes.json", 0.0077),
    ("text.json", 0.0180),
    ("rotation.json", 0.0096),
    ("custom-font.json", 0.0077),
    ("system-font.json", 0.0085),
    ("images.json", 0.0060),
    ("tables.json", 0.0116),
    ("shaders.json", 0.1031),
    ("pptx/groups.json", 0.0027),
    ("pptx/media.json", 0.1291),
];

/// Renders each scene on the CPU and its deck with LibreOffice, and checks
/// that they differ no more than before. Slow: run it with
/// `cargo test -- --ignored pptx`.
#[test]
#[ignore = "needs LibreOffice; slow"]
fn pptx_decks_look_like_the_slides_in_libreoffice() {
    if !super::visual::available() {
        eprintln!("LibreOffice or pdftoppm is missing: skipping");
        return;
    }
    let videos = crate::videos::tests::plugins_or_skip();
    let work = std::env::temp_dir().join(format!("sliderino-visual-{}", std::process::id()));
    let mut failures = Vec::new();
    for (name, limit) in VISUAL {
        if !videos && VIDEO_SCENES.contains(name) {
            continue;
        }
        let presentation = load(name);
        let slide = presentation.slides[0].id;
        let reference = crate::render::render_slide(&presentation, slide, 1., false).unwrap();
        let deck = export(&presentation, &Options::default()).unwrap();
        let folder = work.join(name.replace('/', "-"));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("deck.pptx");
        super::save(&deck, &path).unwrap();
        let pages = super::visual::render(&path, &folder.join("work"), 1.).unwrap();
        let difference = super::visual::difference(&reference, &pages[0]);
        if difference.differing > *limit {
            difference.image.save_png(folder.join("diff.png")).ok();
            failures.push(format!(
                "{name}: {:.4}% of the pixels differ, more than {:.4}% (see {})",
                difference.differing * 100.,
                limit * 100.,
                folder.join("diff.png").display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    std::fs::remove_dir_all(work).ok();
}

/// Validates the deck of every scene against the Open XML schema. Slow:
/// run it with `cargo test -- --ignored pptx`.
#[test]
#[ignore = "needs dotnet; slow"]
fn pptx_decks_follow_the_open_xml_schema() {
    if !super::schema::available() {
        eprintln!("dotnet is missing: skipping");
        return;
    }
    let videos = crate::videos::tests::plugins_or_skip();
    let work = std::env::temp_dir().join(format!("sliderino-schema-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let mut problems = Vec::new();
    for name in SCENES {
        if !videos && VIDEO_SCENES.contains(name) {
            continue;
        }
        let deck = export(&load(name), &Options::default()).unwrap();
        let path = work.join(format!("{}.pptx", name.replace('/', "-")));
        super::save(&deck, &path).unwrap();
        let errors = super::schema::validate(&path).unwrap();
        problems.extend(errors.into_iter().map(|error| format!("{name}: {error}")));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    std::fs::remove_dir_all(work).ok();
}
