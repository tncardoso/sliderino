//! The JSON form of [`Operation`], shared by debug scenes and external
//! agents.
//!
//! Each entry is tagged by `"op"` in snake_case:
//!
//! ```json
//! [
//!   {"op": "add_element", "slide": 1, "element": {
//!     "id": "$title", "frame": {"x": 120, "y": 80, "width": 640},
//!     "text": {"content": "Hello", "sizing": "auto_height",
//!              "style": {"font": {"family": "Inter", "weight": 600}, "size": 64}}}},
//!   {"op": "set_text_style", "id": "$title", "patch": {"underline": true}}
//! ]
//! ```
//!
//! Ids of new slides and elements are optional: the presentation gives the
//! next free one. A `"$name"` reference names the new id so later ops of the
//! same list can use it; [`Applied::refs`] maps each name to its id.
//!
//! `add_font` only names the face; its bytes come from the fonts bundled with
//! the app or installed on the system. With [`Options::auto_fonts`], ops that
//! use a face the presentation does not embed add it first.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;

use crate::document::{
    ApplyError, Element, ElementId, ElementKind, FontFace, Frame, GroupElement, LayerPatch,
    Operation, Presentation, Slide, SlideId, TextElement, TextSizing, TextStylePatch,
};
use crate::fonts;

fn end() -> usize {
    usize::MAX
}

/// An existing id, or a `"$name"` reference to an id made earlier in the
/// same list of ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdRef {
    Id(u64),
    Ref(String),
}

impl<'de> Deserialize<'de> for IdRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Id(u64),
            Name(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Id(id) => Ok(IdRef::Id(id)),
            Raw::Name(name) if name.len() > 1 && name.starts_with('$') => Ok(IdRef::Ref(name)),
            Raw::Name(name) => Err(serde::de::Error::custom(format!(
                "expected an id or a \"$name\" reference, got {name:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSlide {
    #[serde(default)]
    pub id: Option<IdRef>,
    #[serde(default)]
    pub elements: Vec<NewElement>,
}

/// In JSON the kind is a key of the element, as in [`Element`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct NewElement {
    #[serde(default)]
    pub id: Option<IdRef>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub frame: Frame,
    #[serde(flatten)]
    pub kind: NewKind,
}

/// The kind of a new element: a text, or a group of new elements.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewKind {
    Text(TextElement),
    Group(NewGroup),
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewGroup {
    #[serde(default)]
    pub children: Vec<NewElement>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddSlide {
        /// Position in the presentation; last when omitted.
        #[serde(default = "end")]
        index: usize,
        #[serde(default)]
        slide: NewSlide,
    },
    RemoveSlide {
        id: IdRef,
    },
    MoveSlide {
        id: IdRef,
        index: usize,
    },
    AddElement {
        slide: IdRef,
        /// Group of the slide to add to; the slide itself when omitted.
        #[serde(default)]
        parent: Option<IdRef>,
        /// Paint order position; on top when omitted.
        #[serde(default = "end")]
        index: usize,
        element: NewElement,
    },
    RemoveElement {
        id: IdRef,
    },
    /// Moves an element into a group of the same slide, or to the slide
    /// itself when `parent` is omitted, at `index` (on top when omitted).
    MoveElement {
        id: IdRef,
        #[serde(default)]
        parent: Option<IdRef>,
        #[serde(default = "end")]
        index: usize,
    },
    /// Puts elements of one slide in a new group, in place of the topmost.
    Group {
        #[serde(default)]
        id: Option<IdRef>,
        children: Vec<IdRef>,
    },
    /// Puts the children of a group in its place and removes it.
    Ungroup {
        id: IdRef,
    },
    SetLayer {
        id: IdRef,
        patch: LayerPatch,
    },
    SetFrame {
        id: IdRef,
        frame: Frame,
    },
    SetTextSizing {
        id: IdRef,
        sizing: TextSizing,
    },
    SetTextStyle {
        id: IdRef,
        patch: TextStylePatch,
    },
    ReplaceText {
        id: IdRef,
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
        ops: Vec<Op>,
    },
}

impl Op {
    /// The name the editor gives this change in its history.
    pub fn label(&self) -> &'static str {
        match self {
            Op::AddSlide { .. } => "Add slide",
            Op::RemoveSlide { .. } => "Delete slide",
            Op::MoveSlide { .. } => "Move slide",
            Op::AddElement { element, .. } => match element.kind {
                NewKind::Text(_) => "Create text",
                NewKind::Group(_) => "Create group",
            },
            Op::RemoveElement { .. } => "Delete",
            Op::MoveElement { .. } => "Move layer",
            Op::Group { .. } => "Group",
            Op::Ungroup { .. } => "Ungroup",
            Op::SetLayer { patch, .. } => patch.label(),
            Op::SetFrame { .. } => "Move",
            Op::SetTextSizing { .. } => "Resizing",
            Op::SetTextStyle { patch, .. } => patch.label(),
            Op::ReplaceText { .. } => "Edit text",
            Op::AddFont { .. } => "Add font",
            Op::RemoveFont { .. } => "Remove font",
            Op::Batch { .. } => "Batch",
        }
    }
}

pub fn parse(text: &str) -> Result<Vec<Op>, serde_json::Error> {
    serde_json::from_str(text)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Embed the faces that ops use before the ops that need them, and
    /// ignore an `add_font` of a face already embedded.
    pub auto_fonts: bool,
    /// On failure, revert the ops applied before the failing one.
    pub all_or_nothing: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpErrorKind {
    /// The face is neither bundled nor installed.
    MissingFont(FontFace),
    /// No earlier op made this reference.
    UnknownRef(String),
    /// An earlier op already made this reference.
    DuplicateRef(String),
    /// The reference names a slide where an element is expected, or the
    /// reverse.
    WrongRefKind(String),
    Apply(ApplyError),
}

/// A failed op and its position in the list, from 0. Ops inside a batch
/// report the position of the batch.
#[derive(Clone, Debug, PartialEq)]
pub struct OpError {
    pub index: usize,
    pub kind: OpErrorKind,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op #{}: ", self.index)?;
        match &self.kind {
            OpErrorKind::MissingFont(face) => write!(
                f,
                "font {} {} is not available",
                face.family,
                face.style_name()
            ),
            OpErrorKind::UnknownRef(name) => write!(f, "unknown reference {name}"),
            OpErrorKind::DuplicateRef(name) => write!(f, "reference {name} is already used"),
            OpErrorKind::WrongRefKind(name) => {
                write!(f, "reference {name} names another kind of object")
            }
            OpErrorKind::Apply(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OpError {}

/// One applied op: its history label and the operation that undoes it.
#[derive(Debug)]
pub struct Step {
    pub label: &'static str,
    pub inverse: Operation,
}

#[derive(Debug, Default)]
pub struct Applied {
    /// One step for each op that changed the presentation, in order.
    pub steps: Vec<Step>,
    /// Id of each `"$name"` reference.
    pub refs: BTreeMap<String, u64>,
    /// Notes about the ops that did not stop them, such as a font whose
    /// license forbids embedding.
    pub warnings: Vec<String>,
    /// The last slide and element the ops changed, to show them.
    pub last_slide: Option<SlideId>,
    pub last_element: Option<ElementId>,
}

impl Applied {
    /// The steps as one operation that undoes them all.
    pub fn inverse(&self) -> Operation {
        Operation::Batch(
            self.steps
                .iter()
                .rev()
                .map(|step| step.inverse.clone())
                .collect(),
        )
    }
}

/// Applies the ops in order. On failure, the ops before the failing one
/// stay applied unless [`Options::all_or_nothing`] is set.
pub fn apply(
    presentation: &mut Presentation,
    ops: Vec<Op>,
    options: Options,
) -> Result<Applied, OpError> {
    let mut compiler = Compiler {
        options,
        refs: BTreeMap::new(),
        applied: Applied::default(),
    };
    for (index, op) in ops.into_iter().enumerate() {
        if let Err(kind) = compiler.apply(presentation, op) {
            if options.all_or_nothing {
                for step in compiler.applied.steps.into_iter().rev() {
                    presentation
                        .apply(step.inverse)
                        .expect("the inverse of an applied operation applies");
                }
            }
            return Err(OpError { index, kind });
        }
    }
    let mut applied = compiler.applied;
    applied.refs = compiler
        .refs
        .into_iter()
        .map(|(name, id)| (name, id.value()))
        .collect();
    Ok(applied)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Made {
    Slide(SlideId),
    Element(ElementId),
}

impl Made {
    fn value(self) -> u64 {
        match self {
            Made::Slide(id) => id.0,
            Made::Element(id) => id.0,
        }
    }
}

struct Compiler {
    options: Options,
    refs: BTreeMap<String, Made>,
    applied: Applied,
}

impl Compiler {
    fn apply(&mut self, presentation: &mut Presentation, op: Op) -> Result<(), OpErrorKind> {
        let label = match &op {
            Op::SetFrame { id, frame } => {
                let resized = self
                    .element(id)
                    .ok()
                    .and_then(|id| presentation.element(id));
                match resized {
                    Some(element)
                        if element.frame.width != frame.width
                            || element.frame.height != frame.height =>
                    {
                        "Resize"
                    }
                    _ => "Move",
                }
            }
            op => op.label(),
        };
        let mut operations = Vec::new();
        self.compile(presentation, op, &mut operations)?;
        let operation = match operations.len() {
            0 => return Ok(()),
            1 => operations.pop().expect("one operation"),
            _ => Operation::Batch(operations),
        };
        let inverse = presentation.apply(operation).map_err(OpErrorKind::Apply)?;
        self.applied.steps.push(Step { label, inverse });
        Ok(())
    }

    /// Converts `op` to operations, pushing an `AddFont` before it for each
    /// face it needs when fonts are automatic.
    fn compile(
        &mut self,
        presentation: &mut Presentation,
        op: Op,
        out: &mut Vec<Operation>,
    ) -> Result<(), OpErrorKind> {
        let operation = match op {
            Op::AddSlide { index, slide } => {
                let id = self.new_slide(presentation, slide.id)?;
                let mut elements = Vec::with_capacity(slide.elements.len());
                for element in slide.elements {
                    elements.push(self.new_element(presentation, element, out)?);
                }
                self.applied.last_slide = Some(id);
                Operation::AddSlide {
                    index,
                    slide: Slide { id, elements },
                }
            }
            Op::RemoveSlide { id } => Operation::RemoveSlide {
                id: self.slide(&id)?,
            },
            Op::MoveSlide { id, index } => {
                let id = self.slide(&id)?;
                self.applied.last_slide = Some(id);
                Operation::MoveSlide { id, index }
            }
            Op::AddElement {
                slide,
                parent,
                index,
                element,
            } => {
                let slide = self.slide(&slide)?;
                let parent = parent.map(|parent| self.element(&parent)).transpose()?;
                let element = self.new_element(presentation, element, out)?;
                self.applied.last_slide = Some(slide);
                self.applied.last_element = Some(element.id);
                Operation::AddElement {
                    slide,
                    parent,
                    index,
                    element,
                }
            }
            Op::RemoveElement { id } => Operation::RemoveElement {
                id: self.touch(presentation, &id)?,
            },
            Op::MoveElement { id, parent, index } => Operation::MoveElement {
                id: self.touch(presentation, &id)?,
                parent: parent.map(|parent| self.element(&parent)).transpose()?,
                index,
            },
            Op::Group { id, children } => {
                let children = children
                    .iter()
                    .map(|child| self.element(child))
                    .collect::<Result<Vec<_>, _>>()?;
                let group = self.new_element_id(presentation, id)?;
                let operations = presentation
                    .group_operations(group, &children)
                    .map_err(OpErrorKind::Apply)?;
                self.applied.last_element = Some(group);
                if let Some(location) = children.first().and_then(|id| presentation.locate(*id)) {
                    self.applied.last_slide = Some(location.slide);
                }
                Operation::Batch(operations)
            }
            Op::Ungroup { id } => {
                let id = self.touch(presentation, &id)?;
                Operation::Batch(
                    presentation
                        .ungroup_operations(id)
                        .map_err(OpErrorKind::Apply)?,
                )
            }
            Op::SetLayer { id, patch } => Operation::SetLayer {
                id: self.touch(presentation, &id)?,
                patch,
            },
            Op::SetFrame { id, frame } => Operation::SetFrame {
                id: self.touch(presentation, &id)?,
                frame,
            },
            Op::SetTextSizing { id, sizing } => Operation::SetTextSizing {
                id: self.touch(presentation, &id)?,
                sizing,
            },
            Op::SetTextStyle { id, patch } => {
                if let Some(face) = &patch.font {
                    self.ensure_font(presentation, face, out)?;
                }
                Operation::SetTextStyle {
                    id: self.touch(presentation, &id)?,
                    patch,
                }
            }
            Op::ReplaceText { id, range, text } => Operation::ReplaceText {
                id: self.touch(presentation, &id)?,
                range,
                text,
            },
            Op::AddFont { face } => {
                if self.options.auto_fonts && self.is_embedded(presentation, &face, out) {
                    return Ok(());
                }
                self.font(face)?
            }
            Op::RemoveFont { face } => Operation::RemoveFont { face },
            Op::Batch { ops } => {
                let mut inner = Vec::with_capacity(ops.len());
                for op in ops {
                    self.compile(presentation, op, &mut inner)?;
                }
                Operation::Batch(inner)
            }
        };
        out.push(operation);
        Ok(())
    }

    fn new_slide(
        &mut self,
        presentation: &mut Presentation,
        id: Option<IdRef>,
    ) -> Result<SlideId, OpErrorKind> {
        Ok(match id {
            Some(IdRef::Id(id)) => SlideId(id),
            Some(IdRef::Ref(name)) => {
                let id = presentation.new_slide_id();
                self.name(name, Made::Slide(id))?;
                id
            }
            None => presentation.new_slide_id(),
        })
    }

    fn new_element(
        &mut self,
        presentation: &mut Presentation,
        element: NewElement,
        out: &mut Vec<Operation>,
    ) -> Result<Element, OpErrorKind> {
        let id = self.new_element_id(presentation, element.id)?;
        let kind = match element.kind {
            NewKind::Text(text) => {
                self.ensure_font(presentation, &text.style.font, out)?;
                ElementKind::Text(text)
            }
            NewKind::Group(group) => {
                let mut children = Vec::with_capacity(group.children.len());
                for child in group.children {
                    children.push(self.new_element(presentation, child, out)?);
                }
                ElementKind::Group(GroupElement { children })
            }
        };
        self.applied.last_element = Some(id);
        Ok(Element {
            name: element.name,
            hidden: element.hidden,
            locked: element.locked,
            ..Element::new(id, element.frame, kind)
        })
    }

    /// The id of a new element: the one given, a new one named by a
    /// reference, or the next free one.
    fn new_element_id(
        &mut self,
        presentation: &mut Presentation,
        id: Option<IdRef>,
    ) -> Result<ElementId, OpErrorKind> {
        Ok(match id {
            Some(IdRef::Id(id)) => ElementId(id),
            Some(IdRef::Ref(name)) => {
                let id = presentation.new_element_id();
                self.name(name, Made::Element(id))?;
                id
            }
            None => presentation.new_element_id(),
        })
    }

    fn name(&mut self, name: String, made: Made) -> Result<(), OpErrorKind> {
        if self.refs.contains_key(&name) {
            return Err(OpErrorKind::DuplicateRef(name));
        }
        self.refs.insert(name, made);
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Made, OpErrorKind> {
        self.refs
            .get(name)
            .copied()
            .ok_or_else(|| OpErrorKind::UnknownRef(name.into()))
    }

    fn slide(&self, id: &IdRef) -> Result<SlideId, OpErrorKind> {
        match id {
            IdRef::Id(id) => Ok(SlideId(*id)),
            IdRef::Ref(name) => match self.lookup(name)? {
                Made::Slide(id) => Ok(id),
                Made::Element(_) => Err(OpErrorKind::WrongRefKind(name.clone())),
            },
        }
    }

    fn element(&self, id: &IdRef) -> Result<ElementId, OpErrorKind> {
        match id {
            IdRef::Id(id) => Ok(ElementId(*id)),
            IdRef::Ref(name) => match self.lookup(name)? {
                Made::Element(id) => Ok(id),
                Made::Slide(_) => Err(OpErrorKind::WrongRefKind(name.clone())),
            },
        }
    }

    /// Resolves an element the op changes and records it as the last one.
    fn touch(&mut self, presentation: &Presentation, id: &IdRef) -> Result<ElementId, OpErrorKind> {
        let id = self.element(id)?;
        self.applied.last_element = Some(id);
        if let Some(location) = presentation.locate(id) {
            self.applied.last_slide = Some(location.slide);
        }
        Ok(id)
    }

    /// Whether the face is embedded or added by an earlier operation of
    /// the op being compiled.
    fn is_embedded(&self, presentation: &Presentation, face: &FontFace, out: &[Operation]) -> bool {
        presentation.fonts.contains(face)
            || out
                .iter()
                .any(|operation| matches!(operation, Operation::AddFont { face: added, .. } if added == face))
    }

    fn ensure_font(
        &mut self,
        presentation: &Presentation,
        face: &FontFace,
        out: &mut Vec<Operation>,
    ) -> Result<(), OpErrorKind> {
        if self.options.auto_fonts && !self.is_embedded(presentation, face, out) {
            out.push(self.font(face.clone())?);
        }
        Ok(())
    }

    /// An `AddFont` with the face's bytes. A face whose license forbids
    /// embedding is embedded anyway, with a warning: the author is trusted
    /// to hold the license.
    fn font(&mut self, face: FontFace) -> Result<Operation, OpErrorKind> {
        let Some(data) = fonts::data(&face) else {
            return Err(OpErrorKind::MissingFont(face));
        };
        if fonts::catalog().is_restricted(&face) {
            self.applied.warnings.push(format!(
                "the license of {} {} forbids embedding",
                face.family,
                face.style_name()
            ));
        }
        Ok(Operation::AddFont { face, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: Options = Options {
        auto_fonts: true,
        all_or_nothing: true,
    };

    #[test]
    fn references_name_new_ids_for_later_ops() {
        let mut presentation = Presentation::new();
        let ops = parse(
            r#"[
              {"op": "add_slide", "slide": {"id": "$s"}},
              {"op": "add_element", "slide": "$s", "element": {
                "id": "$title", "text": {"content": "Hi"}}},
              {"op": "set_text_style", "id": "$title", "patch": {"size": 80}}
            ]"#,
        )
        .unwrap();
        let applied = apply(&mut presentation, ops, AGENT).unwrap();
        let title = ElementId(applied.refs["$title"]);
        let slide = SlideId(applied.refs["$s"]);
        assert_eq!(presentation.locate(title).unwrap().slide, slide);
        let text = presentation.element(title).unwrap().as_text().unwrap();
        assert_eq!(text.style.size, 80.);
        assert_eq!(applied.last_element, Some(title));
        assert_eq!(applied.last_slide, Some(slide));
    }

    #[test]
    fn omitted_ids_take_the_next_free_ones() {
        let mut presentation = Presentation::new();
        let ops = parse(
            r#"[
              {"op": "add_element", "slide": 1, "element": {"text": {"content": "a"}}},
              {"op": "add_element", "slide": 1, "element": {"text": {"content": "b"}}}
            ]"#,
        )
        .unwrap();
        apply(&mut presentation, ops, AGENT).unwrap();
        let ids: Vec<_> = presentation.slides[0]
            .elements
            .iter()
            .map(|element| element.id)
            .collect();
        assert_eq!(ids, [ElementId(1), ElementId(2)]);
    }

    #[test]
    fn automatic_fonts_embed_each_face_once() {
        let mut presentation = Presentation::new();
        let ops = parse(
            r#"[
              {"op": "add_font", "face": {"family": "Inter"}},
              {"op": "add_element", "slide": 1, "element": {"text": {"content": "a"}}},
              {"op": "add_element", "slide": 1, "element": {"text": {"content": "b",
                "style": {"font": {"family": "Inter", "weight": 600}}}}}
            ]"#,
        )
        .unwrap();
        apply(&mut presentation, ops, AGENT).unwrap();
        assert!(
            presentation
                .fonts
                .contains(&FontFace::new("Inter", 600, false))
        );
    }

    #[test]
    fn all_or_nothing_reverts_the_earlier_ops() {
        let mut presentation = Presentation::new();
        let before = presentation.clone();
        let ops = parse(
            r#"[
              {"op": "add_element", "slide": 1, "element": {"text": {"content": "a"}}},
              {"op": "set_text_style", "id": "$nope", "patch": {"size": 10}}
            ]"#,
        )
        .unwrap();
        let error = apply(&mut presentation, ops, AGENT).unwrap_err();
        assert_eq!(error.index, 1);
        assert_eq!(error.kind, OpErrorKind::UnknownRef("$nope".into()));
        assert_eq!(presentation.slides, before.slides);
        assert_eq!(presentation.fonts, before.fonts);
    }

    #[test]
    fn references_keep_their_kind() {
        let mut presentation = Presentation::new();
        let ops = parse(
            r#"[
              {"op": "add_slide", "slide": {"id": "$s"}},
              {"op": "remove_element", "id": "$s"}
            ]"#,
        )
        .unwrap();
        let error = apply(&mut presentation, ops, AGENT).unwrap_err();
        assert_eq!(error.kind, OpErrorKind::WrongRefKind("$s".into()));
    }

    #[test]
    fn a_name_without_dollar_is_not_a_reference() {
        let error = parse(r#"[{"op": "remove_element", "id": "title"}]"#).unwrap_err();
        assert!(error.to_string().contains("\"$name\""), "{error}");
    }
}
