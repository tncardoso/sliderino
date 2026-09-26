//! Scenes for the debug tooling: a JSON list of operations applied to a new
//! presentation, in the format of [`crate::api::ops`].
//!
//! Each op of a scene is one history step. Fonts are not automatic: a scene
//! adds each face it uses with `add_font`. Ids are chosen by the scene or
//! given by the presentation; the first slide of a new presentation is `1`.

use std::path::Path;

use crate::api::ops::{self, OpError, Options, Step};
use crate::document::Presentation;

pub use crate::api::ops::Op as ScriptOp;

#[derive(Debug)]
pub enum ScriptError {
    Read(std::io::Error),
    Parse(serde_json::Error),
    Op(OpError),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Read(error) => write!(f, "cannot read the scene: {error}"),
            ScriptError::Parse(error) => write!(f, "invalid scene: {error}"),
            ScriptError::Op(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ScriptError {}

pub fn load(path: &Path) -> Result<Vec<ScriptOp>, ScriptError> {
    let text = std::fs::read_to_string(path).map_err(ScriptError::Read)?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Vec<ScriptOp>, ScriptError> {
    ops::parse(text).map_err(ScriptError::Parse)
}

/// Applies the ops in order, one step each. Stops at the first failure,
/// reporting the op's index (from 0); the ops before it stay applied.
/// Warnings, such as a font whose license forbids embedding, go to stderr.
pub fn apply(
    presentation: &mut Presentation,
    ops: Vec<ScriptOp>,
) -> Result<Vec<Step>, ScriptError> {
    let applied = ops::apply(presentation, ops, Options::default()).map_err(ScriptError::Op)?;
    for warning in &applied.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(applied.steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ops::OpErrorKind;
    use crate::document::{ApplyError, ElementId, FontFace, HAlign};

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
            matches!(
                &error,
                ScriptError::Op(OpError {
                    index: 0,
                    kind: OpErrorKind::MissingFont(_)
                })
            ),
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
                &error,
                ScriptError::Op(OpError {
                    index: 1,
                    kind: OpErrorKind::Apply(ApplyError::MissingFont(_))
                })
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
