//! The one type that describes every change to a presentation.
//!
//! [`Presentation::apply`](crate::document::Presentation::apply) applies an
//! operation and returns its inverse. The editor records those inverses as its
//! undo history; external agents will send the same operations through the CLI
//! and MCP, so people and agents share one history.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::document::{
    Element, ElementId, FontData, FontFace, Frame, HAlign, LineHeight, Rgb, Slide, SlideId,
    TextCase, TextSizing, TextStyle, VAlign,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// Inserts a slide, with its elements, at `index` (clamped to the end).
    AddSlide {
        index: usize,
        slide: Slide,
    },
    RemoveSlide {
        id: SlideId,
    },
    /// Moves a slide to `index` (clamped to the last position).
    #[allow(
        dead_code,
        reason = "reordering in the slides panel and the agent API come later"
    )]
    MoveSlide {
        id: SlideId,
        index: usize,
    },
    /// Inserts an element at `index` of the paint order of `parent`, a group
    /// of the slide, or of the slide itself when `parent` is `None`
    /// (clamped).
    AddElement {
        slide: SlideId,
        parent: Option<ElementId>,
        index: usize,
        element: Element,
    },
    RemoveElement {
        id: ElementId,
    },
    /// Moves an element to `index` of the children of `parent` (clamped), a
    /// group of the same slide, or of the slide itself when `parent` is
    /// `None`. Frames do not change.
    MoveElement {
        id: ElementId,
        parent: Option<ElementId>,
        index: usize,
    },
    /// Moves, resizes or turns an element; the rotation is the final angle.
    /// Auto-sized text keeps the dimensions its content dictates. A group
    /// moves its descendants; resized or turned, it scales and turns their
    /// positions and boxes but not their fonts.
    SetFrame {
        id: ElementId,
        frame: Frame,
    },
    /// Sets frames of group `id` and its descendants, checking only the lock
    /// of the group. It undoes the resize or the turn of a group; the group
    /// keeps only the rotation of its frame, then fits its children.
    SetGroupFrames {
        id: ElementId,
        frames: Vec<(ElementId, Frame)>,
    },
    /// Changes the name, visibility or lock of an element. The only
    /// operation a locked element accepts.
    SetLayer {
        id: ElementId,
        patch: LayerPatch,
    },
    SetTextSizing {
        id: ElementId,
        sizing: TextSizing,
    },
    /// Changes only the style fields the patch sets.
    SetTextStyle {
        id: ElementId,
        patch: TextStylePatch,
    },
    /// Replaces a byte range of the text content.
    ReplaceText {
        id: ElementId,
        range: Range<usize>,
        text: String,
    },
    /// Embeds a font face in the presentation.
    AddFont {
        face: FontFace,
        data: FontData,
    },
    /// Removes an embedded face no text uses.
    RemoveFont {
        face: FontFace,
    },
    /// Applies the operations in order, all or nothing. Its inverse is a
    /// batch of the inverses in reverse order.
    Batch(Vec<Operation>),
}

/// Layer fields of an [`Element`] to change; `None` leaves the field alone.
/// `name: Some(None)` clears the name. In JSON, omitted fields are `None` and
/// `"name": null` clears it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayerPatch {
    #[serde(
        deserialize_with = "some_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

/// Reads a present field, `null` included, as `Some`.
fn some_option<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl LayerPatch {
    /// The undo step name of the change.
    pub fn label(&self) -> &'static str {
        match (&self.name, self.hidden, self.locked) {
            (Some(_), None, None) => "Rename",
            (None, Some(true), None) => "Hide",
            (None, Some(false), None) => "Show",
            (None, None, Some(true)) => "Lock",
            (None, None, Some(false)) => "Unlock",
            _ => "Layer",
        }
    }

    /// Writes the set fields into `element` and returns a patch holding the
    /// values they replaced. An empty name clears the name.
    pub fn apply_to(self, element: &mut Element) -> LayerPatch {
        fn swap<T>(new: Option<T>, field: &mut T) -> Option<T> {
            new.map(|value| std::mem::replace(field, value))
        }
        let name = self
            .name
            .map(|name| name.filter(|name| !name.trim().is_empty()));
        LayerPatch {
            name: swap(name, &mut element.name),
            hidden: swap(self.hidden, &mut element.hidden),
            locked: swap(self.locked, &mut element.locked),
        }
    }
}

/// Fields of a [`TextStyle`] to change; `None` leaves the field alone. In
/// JSON, omitted fields are `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextStylePatch {
    pub font: Option<FontFace>,
    pub size: Option<f32>,
    pub line_height: Option<LineHeight>,
    pub letter_spacing: Option<f32>,
    pub align: Option<HAlign>,
    pub vertical_align: Option<VAlign>,
    pub paragraph_spacing: Option<f32>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub case: Option<TextCase>,
    pub color: Option<Rgb>,
    pub opacity: Option<f32>,
}

impl TextStylePatch {
    /// The undo step name of the change: the field's name when the patch
    /// sets one field, "Text style" when it sets several.
    pub fn label(&self) -> &'static str {
        let fields = [
            (self.font.is_some(), "Font"),
            (self.size.is_some(), "Font size"),
            (self.line_height.is_some(), "Line height"),
            (self.letter_spacing.is_some(), "Letter spacing"),
            (self.align.is_some(), "Text alignment"),
            (self.vertical_align.is_some(), "Vertical alignment"),
            (self.paragraph_spacing.is_some(), "Paragraph spacing"),
            (self.underline.is_some(), "Underline"),
            (self.strikethrough.is_some(), "Strikethrough"),
            (self.case.is_some(), "Letter case"),
            (self.color.is_some(), "Text color"),
            (self.opacity.is_some(), "Text opacity"),
        ];
        let mut set = fields.iter().filter(|(set, _)| *set);
        match (set.next(), set.next()) {
            (Some((_, label)), None) => label,
            _ => "Text style",
        }
    }

    /// Writes the set fields into `style` and returns a patch holding the
    /// values they replaced.
    pub fn apply_to(self, style: &mut TextStyle) -> TextStylePatch {
        fn swap<T>(new: Option<T>, field: &mut T) -> Option<T> {
            new.map(|value| std::mem::replace(field, value))
        }
        TextStylePatch {
            font: swap(self.font, &mut style.font),
            size: swap(self.size, &mut style.size),
            line_height: swap(self.line_height, &mut style.line_height),
            letter_spacing: swap(self.letter_spacing, &mut style.letter_spacing),
            align: swap(self.align, &mut style.align),
            vertical_align: swap(self.vertical_align, &mut style.vertical_align),
            paragraph_spacing: swap(self.paragraph_spacing, &mut style.paragraph_spacing),
            underline: swap(self.underline, &mut style.underline),
            strikethrough: swap(self.strikethrough, &mut style.strikethrough),
            case: swap(self.case, &mut style.case),
            color: swap(self.color, &mut style.color),
            opacity: swap(self.opacity, &mut style.opacity),
        }
    }

    pub fn validate(&self) -> Result<(), ApplyError> {
        let positive = |value: Option<f32>| value.is_none_or(|v| v.is_finite() && v > 0.);
        let finite = |value: Option<f32>| value.is_none_or(f32::is_finite);
        let line_height = match self.line_height {
            Some(LineHeight::Percent(percent)) => percent.is_finite() && percent > 0.,
            _ => true,
        };
        let valid = positive(self.size)
            && line_height
            && finite(self.letter_spacing)
            && finite(self.paragraph_spacing)
            && self.paragraph_spacing.is_none_or(|v| v >= 0.)
            && self.opacity.is_none_or(|v| (0. ..=1.).contains(&v));
        if valid {
            Ok(())
        } else {
            Err(ApplyError::InvalidStyle)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ApplyError {
    UnknownSlide(SlideId),
    UnknownElement(ElementId),
    DuplicateSlide(SlideId),
    DuplicateElement(ElementId),
    LastSlide,
    NotText(ElementId),
    InvalidRange(Range<usize>),
    InvalidFrame,
    InvalidStyle,
    /// The face is not embedded in the presentation.
    MissingFont(FontFace),
    DuplicateFont(FontFace),
    FontInUse(FontFace),
    /// The embedded bytes are not a readable font.
    BadFont(FontFace),
    /// The element or one of its ancestors is locked.
    Locked(ElementId),
    NotGroup(ElementId),
    /// The parent is on another slide, or inside the moved element.
    InvalidParent(ElementId),
    GroupAcrossSlides,
    EmptyGroup,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::UnknownSlide(id) => write!(f, "no slide {}", id.0),
            ApplyError::UnknownElement(id) => write!(f, "no element {}", id.0),
            ApplyError::DuplicateSlide(id) => write!(f, "slide {} already exists", id.0),
            ApplyError::DuplicateElement(id) => write!(f, "element {} already exists", id.0),
            ApplyError::LastSlide => write!(f, "a presentation keeps at least one slide"),
            ApplyError::NotText(id) => write!(f, "element {} is not text", id.0),
            ApplyError::InvalidRange(range) => write!(f, "invalid text range {range:?}"),
            ApplyError::InvalidFrame => write!(f, "invalid frame"),
            ApplyError::InvalidStyle => write!(f, "invalid text style"),
            ApplyError::MissingFont(face) => write!(f, "font {face:?} is not embedded"),
            ApplyError::DuplicateFont(face) => write!(f, "font {face:?} is already embedded"),
            ApplyError::FontInUse(face) => write!(f, "font {face:?} is in use"),
            ApplyError::BadFont(face) => write!(f, "font {face:?} cannot be read"),
            ApplyError::Locked(id) => write!(f, "element {} is locked", id.0),
            ApplyError::NotGroup(id) => write!(f, "element {} is not a group", id.0),
            ApplyError::InvalidParent(id) => {
                write!(f, "element {} cannot hold this element", id.0)
            }
            ApplyError::GroupAcrossSlides => {
                write!(f, "grouped elements must be on the same slide")
            }
            ApplyError::EmptyGroup => write!(f, "a new group needs at least one element"),
        }
    }
}
