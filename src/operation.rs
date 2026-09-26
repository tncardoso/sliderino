//! The one type that describes every change to a presentation.
//!
//! [`Presentation::apply`](crate::document::Presentation::apply) applies an
//! operation and returns its inverse. The editor records those inverses as its
//! undo history; external agents will send the same operations through the CLI
//! and MCP, so people and agents share one history.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::document::{
    Arrowhead, Element, ElementId, ElementKind, Fill, FontData, FontFace, Frame, HAlign, ImageData,
    ImageId, LineHeight, Rgb, Slide, SlideId, Stroke, TextCase, TextSizing, TextStyle, VAlign,
    VideoData, VideoId,
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
    /// Changes the name, visibility, lock or opacity of an element. The
    /// only operation a locked element accepts.
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
    /// Changes only the style fields of a shape the patch sets.
    SetShapeStyle {
        id: ElementId,
        patch: ShapeStylePatch,
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
    /// Embeds an image in the presentation.
    AddImage {
        id: ImageId,
        data: ImageData,
    },
    /// Removes an embedded image no fill uses.
    RemoveImage {
        id: ImageId,
    },
    /// Embeds a video in the presentation.
    AddVideo {
        id: VideoId,
        data: VideoData,
    },
    /// Removes an embedded video no fill uses.
    RemoveVideo {
        id: VideoId,
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
    /// 0.0 to 1.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
}

/// Reads a present field, `null` included, as `Some`.
fn some_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

impl LayerPatch {
    /// The undo step name of the change.
    pub fn label(&self) -> &'static str {
        match (&self.name, self.hidden, self.locked, self.opacity) {
            (Some(_), None, None, None) => "Rename",
            (None, Some(true), None, None) => "Hide",
            (None, Some(false), None, None) => "Show",
            (None, None, Some(true), None) => "Lock",
            (None, None, Some(false), None) => "Unlock",
            (None, None, None, Some(_)) => "Opacity",
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
            opacity: swap(self.opacity, &mut element.opacity),
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
            && self.paragraph_spacing.is_none_or(|v| v >= 0.);
        if valid {
            Ok(())
        } else {
            Err(ApplyError::InvalidStyle)
        }
    }
}

/// Style fields of a shape to change; `None` leaves the field alone. A
/// field the shape does not have is an error: a line has no fill and only
/// a rectangle has a corner radius. `stroke: Some(None)` removes the stroke
/// of a rectangle or an ellipse. In JSON, omitted fields are `None` and
/// `"stroke": null` removes the stroke.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShapeStylePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(
        deserialize_with = "some_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub stroke: Option<Option<Stroke>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Arrowhead>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Arrowhead>,
}

impl ShapeStylePatch {
    /// The undo step name of the change.
    pub fn label(&self) -> &'static str {
        let fields = [
            (self.fill.is_some(), "Fill"),
            (self.stroke.is_some(), "Stroke"),
            (self.corner_radius.is_some(), "Corner radius"),
            (self.start.is_some(), "Line start"),
            (self.end.is_some(), "Line end"),
        ];
        let mut set = fields.iter().filter(|(set, _)| *set);
        match (set.next(), set.next()) {
            (Some((_, label)), None) => label,
            _ => "Shape style",
        }
    }

    pub fn validate(&self) -> Result<(), ApplyError> {
        if let Some(fill) = &self.fill {
            fill.validate()?;
        }
        if let Some(Some(stroke)) = &self.stroke {
            stroke.validate()?;
        }
        if self
            .corner_radius
            .is_some_and(|radius| !(radius.is_finite() && radius >= 0.))
        {
            return Err(ApplyError::InvalidStyle);
        }
        Ok(())
    }

    /// Writes the set fields into the shape `id` and returns a patch holding
    /// the values they replaced. Changes nothing when a field does not
    /// apply to the shape.
    pub fn apply_to(
        self,
        id: ElementId,
        kind: &mut ElementKind,
    ) -> Result<ShapeStylePatch, ApplyError> {
        let not_applicable = |field| Err(ApplyError::NotApplicable { id, field });
        let (fill, stroke, radius, ends) = match kind {
            ElementKind::Rectangle(shape) => (
                Some(&mut shape.fill),
                StrokeSlot::Optional(&mut shape.stroke),
                Some(&mut shape.corner_radius),
                None,
            ),
            ElementKind::Ellipse(shape) => (
                Some(&mut shape.fill),
                StrokeSlot::Optional(&mut shape.stroke),
                None,
                None,
            ),
            ElementKind::Line(line) => (
                None,
                StrokeSlot::Required(&mut line.stroke),
                None,
                Some((&mut line.start, &mut line.end)),
            ),
            ElementKind::Text(_) | ElementKind::Group(_) => {
                return Err(ApplyError::NotShape(id));
            }
        };
        if self.fill.is_some() && fill.is_none() {
            return not_applicable("fill");
        }
        if self.corner_radius.is_some() && radius.is_none() {
            return not_applicable("corner radius");
        }
        if (self.start.is_some() || self.end.is_some()) && ends.is_none() {
            return not_applicable("arrowheads");
        }
        if matches!(self.stroke, Some(None)) && matches!(stroke, StrokeSlot::Required(_)) {
            return Err(ApplyError::StrokeRequired(id));
        }
        fn swap<T>(new: Option<T>, field: Option<&mut T>) -> Option<T> {
            new.zip(field)
                .map(|(value, field)| std::mem::replace(field, value))
        }
        let (start, end) = ends.map_or((None, None), |(start, end)| (Some(start), Some(end)));
        Ok(ShapeStylePatch {
            fill: swap(self.fill, fill),
            stroke: self.stroke.map(|new| match stroke {
                StrokeSlot::Optional(field) => std::mem::replace(field, new),
                StrokeSlot::Required(field) => {
                    Some(std::mem::replace(field, new.expect("checked above")))
                }
            }),
            corner_radius: swap(self.corner_radius, radius),
            start: swap(self.start, start),
            end: swap(self.end, end),
        })
    }
}

/// The stroke field of a shape: optional on rectangles and ellipses,
/// always present on lines.
enum StrokeSlot<'a> {
    Optional(&'a mut Option<Stroke>),
    Required(&'a mut Stroke),
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
    /// The element is not a rectangle, an ellipse or a line.
    NotShape(ElementId),
    /// The shape has no such style field, such as the fill of a line.
    NotApplicable {
        id: ElementId,
        field: &'static str,
    },
    /// A line cannot lose its stroke.
    StrokeRequired(ElementId),
    /// The opacity is not between 0 and 1.
    InvalidOpacity,
    /// No embedded image has this id.
    MissingImage(ImageId),
    DuplicateImage(ImageId),
    /// A fill uses the image.
    ImageInUse(ImageId),
    /// No embedded video has this id.
    MissingVideo(VideoId),
    DuplicateVideo(VideoId),
    /// A fill uses the video.
    VideoInUse(VideoId),
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
            ApplyError::InvalidStyle => write!(f, "invalid style"),
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
            ApplyError::NotShape(id) => write!(f, "element {} is not a shape", id.0),
            ApplyError::NotApplicable { id, field } => {
                write!(f, "element {} has no {field}", id.0)
            }
            ApplyError::StrokeRequired(id) => {
                write!(f, "element {} is a line: it keeps its stroke", id.0)
            }
            ApplyError::InvalidOpacity => write!(f, "opacity must be between 0 and 1"),
            ApplyError::MissingImage(id) => write!(f, "image {} is not embedded", id.0),
            ApplyError::DuplicateImage(id) => write!(f, "image {} is already embedded", id.0),
            ApplyError::ImageInUse(id) => write!(f, "image {} is in use", id.0),
            ApplyError::MissingVideo(id) => write!(f, "video {} is not embedded", id.0),
            ApplyError::DuplicateVideo(id) => write!(f, "video {} is already embedded", id.0),
            ApplyError::VideoInUse(id) => write!(f, "video {} is in use", id.0),
        }
    }
}
