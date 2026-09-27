//! Exports the debug scenes and checks that each deck is a consistent
//! package whose shapes match the elements (`parity.rs`).

use std::path::{Path, PathBuf};

use super::inspect::Deck;
use super::{Options, export, parity};
use crate::document::Presentation;

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
    let problems: Vec<String> = SCENES.iter().flat_map(|name| check(name)).collect();
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
