//! The one type that describes every change to a presentation.
//!
//! [`Presentation::apply`](crate::document::Presentation::apply) applies an
//! operation and returns its inverse. The editor records those inverses as its
//! undo history; external agents will send the same operations through the CLI
//! and MCP, so people and agents share one history.

use std::ops::Range;

use crate::document::{
    Element, ElementId, FontData, FontFace, HAlign, LineHeight, Rgb, Slide, SlideId, TextCase,
    TextSizing, TextStyle, VAlign,
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
    /// Inserts an element at `index` of the slide's paint order (clamped).
    AddElement {
        slide: SlideId,
        index: usize,
        element: Element,
    },
    RemoveElement {
        id: ElementId,
    },
    /// Moves or resizes an element. Auto-sized text keeps the dimensions its
    /// content dictates.
    SetFrame {
        id: ElementId,
        frame: crate::document::Frame,
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

/// Fields of a [`TextStyle`] to change; `None` leaves the field alone.
#[derive(Clone, Debug, Default, PartialEq)]
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
        }
    }
}
