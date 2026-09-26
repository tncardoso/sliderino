//! Scenes for the debug tooling: a JSON list of operations applied to a new
//! presentation.
//!
//! Each entry mirrors an [`Operation`], tagged by `"op"` in snake_case:
//!
//! ```json
//! [
//!   {"op": "add_font", "face": {"family": "Inter", "weight": 600}},
//!   {"op": "add_element", "slide": 1, "element": {
//!     "id": 1, "frame": {"x": 120, "y": 80, "width": 640},
//!     "text": {"content": "Hello", "sizing": "auto_height",
//!              "style": {"font": {"family": "Inter", "weight": 600}, "size": 64}}}},
//!   {"op": "set_text_style", "id": 1, "patch": {"underline": true}}
//! ]
//! ```
//!
//! `add_font` only names the face; its bytes come from the fonts bundled with
//! the app or installed on the system. Ids are chosen by the scene; the first
//! slide of a new presentation is `1`.

use std::ops::Range;
use std::path::Path;

use serde::Deserialize;

use crate::document::{
    ApplyError, Element, ElementId, FontFace, Frame, Operation, Presentation, Slide, SlideId,
    TextSizing, TextStylePatch,
};
use crate::fonts;

fn end() -> usize {
    usize::MAX
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScriptOp {
    AddSlide {
        #[serde(default = "end")]
        index: usize,
        slide: Slide,
    },
    RemoveSlide {
        id: SlideId,
    },
    MoveSlide {
        id: SlideId,
        index: usize,
    },
    AddElement {
        slide: SlideId,
        /// Paint order position; on top when omitted.
        #[serde(default = "end")]
        index: usize,
        element: Element,
    },
    RemoveElement {
        id: ElementId,
    },
    SetFrame {
        id: ElementId,
        frame: Frame,
    },
    SetTextSizing {
        id: ElementId,
        sizing: TextSizing,
    },
    SetTextStyle {
        id: ElementId,
        patch: TextStylePatch,
    },
    ReplaceText {
        id: ElementId,
        range: Range<usize>,
        text: String,
    },
    AddFont {
        face: FontFace,
    },
    RemoveFont {
        face: FontFace,
    },
    Batch {
        ops: Vec<ScriptOp>,
    },
}

#[derive(Debug)]
pub enum ScriptError {
    Read(std::io::Error),
    Parse(serde_json::Error),
    /// The face is neither bundled nor installed.
    MissingFont {
        index: usize,
        face: FontFace,
    },
    Apply {
        index: usize,
        error: ApplyError,
    },
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = |face: &FontFace| format!("{} {}", face.family, face.style_name());
        match self {
            ScriptError::Read(error) => write!(f, "cannot read the scene: {error}"),
            ScriptError::Parse(error) => write!(f, "invalid scene: {error}"),
            ScriptError::MissingFont { index, face } => {
                write!(f, "op #{index}: font {} is not available", name(face))
            }
            ScriptError::Apply { index, error } => write!(f, "op #{index}: {error}"),
        }
    }
}

impl std::error::Error for ScriptError {}

pub fn load(path: &Path) -> Result<Vec<ScriptOp>, ScriptError> {
    let text = std::fs::read_to_string(path).map_err(ScriptError::Read)?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Vec<ScriptOp>, ScriptError> {
    serde_json::from_str(text).map_err(ScriptError::Parse)
}

impl ScriptOp {
    /// The operation to apply. `index` locates the op in error messages.
    /// A face whose license forbids embedding is embedded anyway, with a
    /// warning: scenes are for debugging.
    pub fn into_operation(self, index: usize) -> Result<Operation, ScriptError> {
        Ok(match self {
            ScriptOp::AddSlide { index, slide } => Operation::AddSlide { index, slide },
            ScriptOp::RemoveSlide { id } => Operation::RemoveSlide { id },
            ScriptOp::MoveSlide { id, index } => Operation::MoveSlide { id, index },
            ScriptOp::AddElement {
                slide,
                index,
                element,
            } => Operation::AddElement {
                slide,
                index,
                element,
            },
            ScriptOp::RemoveElement { id } => Operation::RemoveElement { id },
            ScriptOp::SetFrame { id, frame } => Operation::SetFrame { id, frame },
            ScriptOp::SetTextSizing { id, sizing } => Operation::SetTextSizing { id, sizing },
            ScriptOp::SetTextStyle { id, patch } => Operation::SetTextStyle { id, patch },
            ScriptOp::ReplaceText { id, range, text } => Operation::ReplaceText { id, range, text },
            ScriptOp::AddFont { face } => {
                let Some(data) = fonts::data(&face) else {
                    return Err(ScriptError::MissingFont { index, face });
                };
                if fonts::catalog().is_restricted(&face) {
                    eprintln!(
                        "warning: op #{index}: the license of {} {} forbids embedding",
                        face.family,
                        face.style_name()
                    );
                }
                Operation::AddFont { face, data }
            }
            ScriptOp::RemoveFont { face } => Operation::RemoveFont { face },
            ScriptOp::Batch { ops } => Operation::Batch(
                ops.into_iter()
                    .map(|op| op.into_operation(index))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }

    /// The name the editor gives this change in its history.
    pub fn label(&self, presentation: &Presentation) -> &'static str {
        match self {
            ScriptOp::AddSlide { .. } => "Add slide",
            ScriptOp::RemoveSlide { .. } => "Delete slide",
            ScriptOp::MoveSlide { .. } => "Move slide",
            ScriptOp::AddElement { .. } => "Create text",
            ScriptOp::RemoveElement { .. } => "Delete",
            ScriptOp::SetFrame { id, frame } => {
                let resized = presentation.element(*id).is_some_and(|element| {
                    element.frame.width != frame.width || element.frame.height != frame.height
                });
                if resized { "Resize" } else { "Move" }
            }
            ScriptOp::SetTextSizing { .. } => "Resizing",
            ScriptOp::SetTextStyle { patch, .. } => patch.label(),
            ScriptOp::ReplaceText { .. } => "Edit text",
            ScriptOp::AddFont { .. } => "Add font",
            ScriptOp::RemoveFont { .. } => "Remove font",
            ScriptOp::Batch { .. } => "Batch",
        }
    }
}

/// An applied scene op: its history label and the operation that undoes it.
#[derive(Debug)]
pub struct Applied {
    pub label: &'static str,
    pub inverse: Operation,
}

/// Applies the ops in order. Stops at the first failure, reporting the op's
/// index (from 0); the ops before it stay applied.
pub fn apply(
    presentation: &mut Presentation,
    ops: Vec<ScriptOp>,
) -> Result<Vec<Applied>, ScriptError> {
    let mut applied = Vec::with_capacity(ops.len());
    for (index, op) in ops.into_iter().enumerate() {
        let label = op.label(presentation);
        let operation = op.into_operation(index)?;
        let inverse = presentation
            .apply(operation)
            .map_err(|error| ScriptError::Apply { index, error })?;
        applied.push(Applied { label, inverse });
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::HAlign;

    const EXAMPLE: &str = r#"[
      {"op": "add_font", "face": {"family": "Inter", "weight": 600, "italic": false}},
      {"op": "add_element", "slide": 1, "index": 0, "element": {
        "id": 1, "frame": {"x": 120, "y": 80, "width": 640},
        "text": {"content": "Activation grew faster than signups", "sizing": "auto_height",
                 "style": {"font": {"family": "Inter", "weight": 600, "italic": false},
                           "size": 64, "align": "left"}}}},
      {"op": "set_text_style", "id": 1, "patch": {"underline": true}}
    ]"#;

    #[test]
    fn applies_the_example_scene() {
        let mut presentation = Presentation::new();
        let applied = apply(&mut presentation, parse(EXAMPLE).unwrap()).unwrap();
        let labels: Vec<_> = applied.iter().map(|step| step.label).collect();
        assert_eq!(labels, ["Add font", "Create text", "Underline"]);

        let element = presentation.element(ElementId(1)).unwrap();
        let text = element.as_text().unwrap();
        assert_eq!(text.style.font, FontFace::new("Inter", 600, false));
        assert_eq!(text.style.align, HAlign::Left);
        assert!(text.style.underline);
        assert_eq!(
            text.style.letter_spacing, 0.,
            "omitted fields keep defaults"
        );
        assert_eq!(element.frame.width, 640.);
        assert!(
            element.frame.height > 64.,
            "auto height fits the wrapped text"
        );
    }

    #[test]
    fn a_missing_font_is_reported_with_its_op() {
        let scene = r#"[{"op": "add_font", "face": {"family": "No Such Font Family"}}]"#;
        let error = apply(&mut Presentation::new(), parse(scene).unwrap()).unwrap_err();
        assert!(
            matches!(error, ScriptError::MissingFont { index: 0, .. }),
            "{error}"
        );
    }

    #[test]
    fn apply_errors_carry_the_op_index() {
        // The default font was never added.
        let scene = r#"[
          {"op": "add_slide", "slide": {"id": 2}},
          {"op": "add_element", "slide": 2, "element": {"id": 1, "text": {"content": "Hi"}}}
        ]"#;
        let error = apply(&mut Presentation::new(), parse(scene).unwrap()).unwrap_err();
        assert!(
            matches!(
                error,
                ScriptError::Apply {
                    index: 1,
                    error: ApplyError::MissingFont(_)
                }
            ),
            "{error}"
        );
        assert!(error.to_string().starts_with("op #1: font"));
    }

    #[test]
    fn batches_nest_and_frames_label_move_or_resize() {
        let scene = r#"[
          {"op": "batch", "ops": [
            {"op": "add_font", "face": {"family": "Inter"}},
            {"op": "add_element", "slide": 1, "element": {
              "id": 7, "frame": {"width": 100, "height": 50},
              "text": {"content": "Box", "sizing": "fixed"}}}
          ]},
          {"op": "set_frame", "id": 7, "frame": {"x": 40, "width": 100, "height": 50}},
          {"op": "set_frame", "id": 7, "frame": {"x": 40, "width": 300, "height": 50}}
        ]"#;
        let mut presentation = Presentation::new();
        let applied = apply(&mut presentation, parse(scene).unwrap()).unwrap();
        let labels: Vec<_> = applied.iter().map(|step| step.label).collect();
        assert_eq!(labels, ["Batch", "Move", "Resize"]);
        assert_eq!(
            presentation.element(ElementId(7)).unwrap().frame.width,
            300.
        );
    }
}
