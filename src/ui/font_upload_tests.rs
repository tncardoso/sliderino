//! End-to-end tests of uploaded fonts: the upload button, a drop, the
//! family picker, the Fonts section and texts in faces that GPUI cannot
//! load.

use std::sync::Arc;

use gpui_kit::TestAppContext;

use crate::document::tests::{add_text, with_inter};
use crate::document::{ElementId, FontData, FontFace, Frame, Operation, Presentation, TextSizing};
use crate::fonts::{self, RemoveFamilyError, UploadedFace};
use crate::ui::canvas::PaintItem;
use crate::ui::test_support::{open_with, read, with_editor};

/// The faces of a bundled Inter file, renamed to `family`.
fn upload(index: usize, family: &str) -> Vec<UploadedFace> {
    let bytes = crate::font_file::rename(&crate::assets::fonts()[index], 0, family).unwrap();
    fonts::read_file(bytes).unwrap()
}

fn two_texts() -> (Presentation, ElementId, ElementId) {
    let mut presentation = with_inter();
    let first = add_text(
        &mut presentation,
        "One",
        TextSizing::AutoWidth,
        Frame::default(),
    );
    let second = add_text(
        &mut presentation,
        "Two",
        TextSizing::AutoWidth,
        Frame {
            y: 200.,
            ..Frame::default()
        },
    );
    (presentation, first, second)
}

fn font_of(editor: &crate::editor::EditorView, id: ElementId) -> FontFace {
    editor
        .presentation
        .element(id)
        .unwrap()
        .as_text()
        .unwrap()
        .style
        .font
        .clone()
}

#[gpui_kit::test]
fn the_upload_button_gives_the_font_to_the_selected_texts_in_one_step(cx: &mut TestAppContext) {
    let (presentation, first, second) = two_texts();
    let handle = open_with(cx, presentation);
    let before = read(cx, handle, |editor| editor.presentation.clone());
    let mut faces = upload(2, "Upload Test");
    faces.extend(upload(0, "Upload Test"));
    with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first, second];
        editor.upload_fonts(faces, true)
    });
    read(cx, handle, |editor| {
        // Both texts are regular: the regular face is the closest.
        assert_eq!(
            font_of(editor, first),
            FontFace::new("Upload Test", 400, false)
        );
        assert_eq!(
            font_of(editor, second),
            FontFace::new("Upload Test", 400, false)
        );
        assert!(
            editor
                .presentation
                .fonts
                .contains(&FontFace::new("Upload Test", 600, false))
        );
        assert_eq!(editor.selection, [first, second]);
    });
    with_editor(cx, handle, |editor, _| editor.undo().unwrap().unwrap());
    read(cx, handle, |editor| assert_eq!(editor.presentation, before));
}

#[gpui_kit::test]
fn a_dropped_font_is_embedded_but_not_given_to_the_selection(cx: &mut TestAppContext) {
    let (presentation, first, _) = two_texts();
    let handle = open_with(cx, presentation);
    with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first];
        editor.upload_fonts(upload(0, "Dropped"), false)
    });
    read(cx, handle, |editor| {
        assert_eq!(font_of(editor, first), FontFace::new("Inter", 400, false));
        assert!(
            editor
                .presentation
                .fonts
                .contains(&FontFace::new("Dropped", 400, false))
        );
    });
}

#[gpui_kit::test]
fn an_uploaded_font_with_an_installed_name_is_renamed(cx: &mut TestAppContext) {
    let (presentation, first, _) = two_texts();
    let handle = open_with(cx, presentation);
    let warnings = with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first];
        editor.upload_fonts(upload(1, "Inter"), true)
    });
    assert_eq!(
        warnings,
        ["Inter is embedded as Inter (2): the name is in use"]
    );
    read(cx, handle, |editor| {
        assert_eq!(
            font_of(editor, first),
            FontFace::new("Inter (2)", 500, false)
        );
    });
}

#[gpui_kit::test]
fn the_family_picker_lists_the_embedded_families(cx: &mut TestAppContext) {
    fonts::catalog();
    let (presentation, first, _) = two_texts();
    let handle = open_with(cx, presentation);
    with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first];
        editor.upload_fonts(upload(0, "Picked"), false)
    });
    with_editor(cx, handle, |_, _| {});
    read(cx, handle, |editor| {
        assert_eq!(
            editor.inspector.families_of.as_deref(),
            Some(&["Inter".to_string(), "Picked".to_string()][..])
        );
    });
    with_editor(cx, handle, |editor, _| editor.set_family("Picked"));
    read(cx, handle, |editor| {
        assert_eq!(font_of(editor, first), FontFace::new("Picked", 400, false));
    });
}

#[gpui_kit::test]
fn removing_a_family_changes_its_texts_to_inter(cx: &mut TestAppContext) {
    let (presentation, first, second) = two_texts();
    let handle = open_with(cx, presentation);
    with_editor(cx, handle, |editor, _| {
        editor.selection = vec![first];
        editor.upload_fonts(upload(6, "Gone"), true);
        editor.selection.clear();
    });
    with_editor(cx, handle, |editor, _| {
        assert_eq!(
            editor.remove_font_family("Inter"),
            Err(RemoveFamilyError::FallbackInUse(1))
        );
        editor.remove_font_family("Gone").unwrap();
    });
    read(cx, handle, |editor| {
        assert_eq!(font_of(editor, first), FontFace::new("Inter", 600, true));
        assert_eq!(font_of(editor, second), FontFace::new("Inter", 400, false));
        assert!(
            !crate::ui::inspector::embedded_families(&editor.presentation)
                .contains(&"Gone".to_string())
        );
    });
}

#[gpui_kit::test]
fn text_in_a_face_that_gpui_cannot_load_is_drawn_from_its_outlines(cx: &mut TestAppContext) {
    // The bytes name another family, so GPUI finds no face for "Unloadable".
    let mut presentation = with_inter();
    let face = FontFace::new("Unloadable", 400, false);
    let bytes = crate::font_file::rename(&crate::assets::fonts()[3], 0, "Elsewhere").unwrap();
    presentation
        .apply(Operation::AddFont {
            face: face.clone(),
            data: FontData {
                bytes: Arc::from(bytes),
                index: 0,
            },
        })
        .unwrap();
    let id = add_text(
        &mut presentation,
        "Hi",
        TextSizing::AutoWidth,
        Frame::default(),
    );
    presentation
        .apply(Operation::SetTextStyle {
            id,
            patch: crate::document::TextStylePatch {
                font: Some(face),
                ..Default::default()
            },
        })
        .unwrap();
    let handle = open_with(cx, presentation);
    let text = with_editor(cx, handle, |editor, cx| {
        let slide = editor.current_slide;
        editor
            .paint_items(slide, false, cx)
            .into_iter()
            .find_map(|item| match item {
                PaintItem::Text(text) => Some(text),
                PaintItem::Shape(_) => None,
            })
            .unwrap()
    });
    assert!(text.font_id.is_none());
    assert!(text.font.is_some(), "the outlines draw the text");
}
