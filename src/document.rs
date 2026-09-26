//! The presentation document: what gets saved to disk and what agents edit.
//!
//! Types here hold plain data only (no GPUI types) so they can be serialized
//! later without dragging the UI along. Every change goes through
//! [`Presentation::apply`] with an [`Operation`]; see `operation.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::text_layout;

/// Size of every slide in the presentation, in slide units.
///
/// At 100% zoom one slide unit is one logical pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideSize {
    pub width: u32,
    pub height: u32,
}

impl Default for SlideSize {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
        }
    }
}

/// Stable identity of a slide inside its presentation.
///
/// Ids follow creation order. Reordering keeps them, and the id of a removed
/// slide is never handed out again, so an external agent holding an id never
/// ends up pointing at a different slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlideId(pub u64);

/// Stable identity of an element, unique across all slides of the
/// presentation. Same rules as [`SlideId`]: never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub id: SlideId,
    /// Elements in paint order: the last one is on top.
    #[serde(default)]
    pub elements: Vec<Element>,
}

impl Slide {
    pub fn new(id: SlideId) -> Self {
        Self {
            id,
            elements: Vec::new(),
        }
    }
}

/// In JSON the kind is a key of the element: `{"id": 1, "frame": {..},
/// "text": {..}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    #[serde(default)]
    pub frame: Frame,
    #[serde(flatten)]
    pub kind: ElementKind,
}

impl Element {
    pub fn as_text(&self) -> Option<&TextElement> {
        match &self.kind {
            ElementKind::Text(text) => Some(text),
        }
    }

    fn as_text_mut(&mut self) -> Option<&mut TextElement> {
        match &mut self.kind {
            ElementKind::Text(text) => Some(text),
        }
    }
}

/// Position and size of an element, in slide units from the slide's top-left
/// corner. Rotation is in degrees, clockwise, around the frame center.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: f32,
}

impl Frame {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Text(TextElement),
}

/// A text box. The frame only places the text: it has no fill or stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextElement {
    /// Plain text; `\n` separates paragraphs.
    pub content: String,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub sizing: TextSizing,
}

/// How the frame of a text box follows its content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSizing {
    /// No wrapping; width and height follow the text.
    #[default]
    AutoWidth,
    /// Wraps at the frame width; the height follows the text.
    AutoHeight,
    /// Wraps at the frame width; the height is the author's. Text that does
    /// not fit is drawn past the bottom and reported as overflow.
    Fixed,
}

/// A font face: the family name plus the weight and slant that pick one file
/// of the family. Documents only reference faces present in their
/// [`FontLibrary`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFace {
    pub family: String,
    /// CSS weight, 100 to 900.
    #[serde(default = "regular_weight")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
}

fn regular_weight() -> u16 {
    400
}

impl FontFace {
    pub fn new(family: &str, weight: u16, italic: bool) -> Self {
        Self {
            family: family.into(),
            weight,
            italic,
        }
    }

    /// Name of the face inside its family, such as "SemiBold Italic".
    pub fn style_name(&self) -> String {
        let weight = match self.weight {
            100 => "Thin".to_string(),
            200 => "ExtraLight".to_string(),
            300 => "Light".to_string(),
            400 => "Regular".to_string(),
            500 => "Medium".to_string(),
            600 => "SemiBold".to_string(),
            700 => "Bold".to_string(),
            800 => "ExtraBold".to_string(),
            900 => "Black".to_string(),
            other => other.to_string(),
        };
        match (self.italic, self.weight) {
            (true, 400) => "Italic".to_string(),
            (true, _) => format!("{weight} Italic"),
            (false, _) => weight,
        }
    }
}

/// An RGB color, 0xRRGGBB. In JSON, a hex string such as "1A1A1A" or
/// "#1A1A1A".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u32);

impl Rgb {
    pub fn hex(self) -> String {
        format!("{:06X}", self.0 & 0xFF_FFFF)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Rgb)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Rgb::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "expected a hex color like \"1A1A1A\", got {text:?}"
            ))
        })
    }
}

/// In JSON, `"auto"` or `{"percent": 120}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineHeight {
    /// The line spacing the face asks for: (ascent + descent + line gap) of
    /// the font. Exports write the resolved percentage.
    Auto,
    /// Percentage of the font size.
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HAlign {
    Left,
    Center,
    Right,
    /// Stretches every line but the last of each paragraph to the box width.
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// Letter case applied when drawing. Only transforms that PDF, PPTX and HTML
/// all represent natively are offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    Original,
    Upper,
}

/// In JSON every field is optional and defaults to [`TextStyle::default`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextStyle {
    pub font: FontFace,
    /// Font size in slide units.
    pub size: f32,
    pub line_height: LineHeight,
    /// Extra space after each character, as a percentage of the font size.
    pub letter_spacing: f32,
    pub align: HAlign,
    /// Placement of the text inside a [`TextSizing::Fixed`] box.
    pub vertical_align: VAlign,
    /// Extra space after each paragraph but the last, in slide units.
    pub paragraph_spacing: f32,
    pub underline: bool,
    pub strikethrough: bool,
    pub case: TextCase,
    pub color: Rgb,
    /// 0.0 to 1.0.
    pub opacity: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: FontFace::new("Inter", 400, false),
            size: 32.,
            line_height: LineHeight::Auto,
            letter_spacing: 0.,
            align: HAlign::Left,
            vertical_align: VAlign::Top,
            paragraph_spacing: 0.,
            underline: false,
            strikethrough: false,
            case: TextCase::Original,
            color: Rgb(0x111111),
            opacity: 1.,
        }
    }
}

/// Bytes of one font face embedded in the presentation.
#[derive(Clone, PartialEq)]
pub struct FontData {
    pub bytes: Arc<[u8]>,
    /// Face index inside a font collection; 0 for single-face files.
    pub index: u32,
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("bytes", &format_args!("{} bytes", self.bytes.len()))
            .field("index", &self.index)
            .finish()
    }
}

/// Faces embedded in the presentation, so it opens anywhere with its fonts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontLibrary {
    faces: BTreeMap<FontFace, FontData>,
}

impl FontLibrary {
    pub fn get(&self, face: &FontFace) -> Option<&FontData> {
        self.faces.get(face)
    }

    pub fn contains(&self, face: &FontFace) -> bool {
        self.faces.contains_key(face)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    pub size: SlideSize,
    /// Slides in presentation order.
    pub slides: Vec<Slide>,
    pub fonts: FontLibrary,
    /// Id given to the next created slide; only ever grows.
    next_slide_id: u64,
    /// Id given to the next created element; only ever grows.
    next_element_id: u64,
}

impl Default for Presentation {
    fn default() -> Self {
        Self::new()
    }
}

impl Presentation {
    /// A presentation with the default size and one empty slide.
    pub fn new() -> Self {
        let mut presentation = Self {
            size: SlideSize::default(),
            slides: Vec::new(),
            fonts: FontLibrary::default(),
            next_slide_id: 1,
            next_element_id: 1,
        };
        let id = presentation.new_slide_id();
        presentation.slides.push(Slide::new(id));
        presentation
    }

    /// Reserves a slide id for an [`Operation::AddSlide`].
    pub fn new_slide_id(&mut self) -> SlideId {
        let id = SlideId(self.next_slide_id);
        self.next_slide_id += 1;
        id
    }

    /// Reserves an element id for an [`Operation::AddElement`].
    pub fn new_element_id(&mut self) -> ElementId {
        let id = ElementId(self.next_element_id);
        self.next_element_id += 1;
        id
    }

    pub fn index_of(&self, id: SlideId) -> Option<usize> {
        self.slides.iter().position(|slide| slide.id == id)
    }

    pub fn slide(&self, id: SlideId) -> Option<&Slide> {
        self.slides.iter().find(|slide| slide.id == id)
    }

    pub fn element(&self, id: ElementId) -> Option<&Element> {
        self.slides
            .iter()
            .flat_map(|slide| &slide.elements)
            .find(|element| element.id == id)
    }

    /// The slide holding the element and the element's index in it.
    pub fn locate(&self, id: ElementId) -> Option<(SlideId, usize)> {
        self.slides.iter().find_map(|slide| {
            let index = slide.elements.iter().position(|element| element.id == id)?;
            Some((slide.id, index))
        })
    }

    fn element_mut(&mut self, id: ElementId) -> Result<&mut Element, ApplyError> {
        self.slides
            .iter_mut()
            .flat_map(|slide| &mut slide.elements)
            .find(|element| element.id == id)
            .ok_or(ApplyError::UnknownElement(id))
    }

    fn text_mut(&mut self, id: ElementId) -> Result<&mut TextElement, ApplyError> {
        self.element_mut(id)?
            .as_text_mut()
            .ok_or(ApplyError::NotText(id))
    }

    /// Lays out a text element with its embedded font.
    pub fn text_layout(&self, id: ElementId) -> Option<text_layout::TextLayout> {
        let element = self.element(id)?;
        let text = element.as_text()?;
        let font = self.fonts.get(&text.style.font)?;
        text_layout::layout(text, &element.frame, font).ok()
    }

    /// Updates the frame of an auto-sized text box to fit its content.
    fn fit(&mut self, id: ElementId) -> Result<(), ApplyError> {
        let element = self.element(id).ok_or(ApplyError::UnknownElement(id))?;
        let Some(text) = element.as_text() else {
            return Ok(());
        };
        if text.sizing == TextSizing::Fixed {
            return Ok(());
        }
        let font = self
            .fonts
            .get(&text.style.font)
            .ok_or_else(|| ApplyError::MissingFont(text.style.font.clone()))?;
        let size = text_layout::measure(text, element.frame.width, font)
            .map_err(|_| ApplyError::BadFont(text.style.font.clone()))?;
        let sizing = text.sizing;
        let frame = &mut self.element_mut(id)?.frame;
        if sizing == TextSizing::AutoWidth {
            frame.width = size.0;
        }
        frame.height = size.1;
        Ok(())
    }

    /// Applies one edit and returns the operation that undoes it.
    ///
    /// On error the presentation is left unchanged, including for a
    /// [`Operation::Batch`] that fails halfway.
    pub fn apply(&mut self, operation: Operation) -> Result<Operation, ApplyError> {
        match operation {
            Operation::AddSlide { index, slide } => {
                if self.index_of(slide.id).is_some() {
                    return Err(ApplyError::DuplicateSlide(slide.id));
                }
                if let Some(element) = slide
                    .elements
                    .iter()
                    .find(|element| self.element(element.id).is_some())
                {
                    return Err(ApplyError::DuplicateElement(element.id));
                }
                self.next_slide_id = self.next_slide_id.max(slide.id.0 + 1);
                for element in &slide.elements {
                    self.next_element_id = self.next_element_id.max(element.id.0 + 1);
                }
                let id = slide.id;
                let index = index.min(self.slides.len());
                self.slides.insert(index, slide);
                Ok(Operation::RemoveSlide { id })
            }
            Operation::RemoveSlide { id } => {
                let index = self.index_of(id).ok_or(ApplyError::UnknownSlide(id))?;
                if self.slides.len() == 1 {
                    return Err(ApplyError::LastSlide);
                }
                let slide = self.slides.remove(index);
                Ok(Operation::AddSlide { index, slide })
            }
            Operation::MoveSlide { id, index } => {
                let from = self.index_of(id).ok_or(ApplyError::UnknownSlide(id))?;
                let slide = self.slides.remove(from);
                let index = index.min(self.slides.len());
                self.slides.insert(index, slide);
                Ok(Operation::MoveSlide { id, index: from })
            }
            Operation::AddElement {
                slide,
                index,
                element,
            } => {
                let slide_index = self
                    .index_of(slide)
                    .ok_or(ApplyError::UnknownSlide(slide))?;
                if self.element(element.id).is_some() {
                    return Err(ApplyError::DuplicateElement(element.id));
                }
                if let Some(text) = element.as_text() {
                    self.check_font(&text.style.font)?;
                }
                check_frame(&element.frame)?;
                let id = element.id;
                self.next_element_id = self.next_element_id.max(id.0 + 1);
                let elements = &mut self.slides[slide_index].elements;
                let index = index.min(elements.len());
                elements.insert(index, element);
                self.fit(id)?;
                Ok(Operation::RemoveElement { id })
            }
            Operation::RemoveElement { id } => {
                let (slide, index) = self.locate(id).ok_or(ApplyError::UnknownElement(id))?;
                let slide_index = self.index_of(slide).expect("located slide exists");
                let element = self.slides[slide_index].elements.remove(index);
                Ok(Operation::AddElement {
                    slide,
                    index,
                    element,
                })
            }
            Operation::SetFrame { id, frame } => {
                check_frame(&frame)?;
                let element = self.element_mut(id)?;
                let old = std::mem::replace(&mut element.frame, frame);
                self.fit(id)?;
                Ok(Operation::SetFrame { id, frame: old })
            }
            Operation::SetTextSizing { id, sizing } => {
                let frame = self
                    .element(id)
                    .ok_or(ApplyError::UnknownElement(id))?
                    .frame;
                let text = self.text_mut(id)?;
                let old = std::mem::replace(&mut text.sizing, sizing);
                self.fit(id)?;
                let inverse = Operation::SetTextSizing { id, sizing: old };
                // Fitting may have replaced dimensions the old mode kept, such
                // as the width of a wrapping box.
                if self
                    .element(id)
                    .is_some_and(|element| element.frame != frame)
                {
                    Ok(Operation::Batch(vec![
                        inverse,
                        Operation::SetFrame { id, frame },
                    ]))
                } else {
                    Ok(inverse)
                }
            }
            Operation::SetTextStyle { id, patch } => {
                if let Some(font) = &patch.font {
                    self.check_font(font)?;
                }
                patch.validate()?;
                let text = self.text_mut(id)?;
                let old = patch.apply_to(&mut text.style);
                self.fit(id)?;
                Ok(Operation::SetTextStyle { id, patch: old })
            }
            Operation::ReplaceText { id, range, text } => {
                let content = &mut self.text_mut(id)?.content;
                if range.start > range.end
                    || range.end > content.len()
                    || !content.is_char_boundary(range.start)
                    || !content.is_char_boundary(range.end)
                {
                    return Err(ApplyError::InvalidRange(range));
                }
                let old = content[range.clone()].to_string();
                content.replace_range(range.clone(), &text);
                let inverse = range.start..range.start + text.len();
                self.fit(id)?;
                Ok(Operation::ReplaceText {
                    id,
                    range: inverse,
                    text: old,
                })
            }
            Operation::AddFont { face, data } => {
                if self.fonts.contains(&face) {
                    return Err(ApplyError::DuplicateFont(face));
                }
                if text_layout::FontMetrics::read(&data).is_err() {
                    return Err(ApplyError::BadFont(face));
                }
                self.fonts.faces.insert(face.clone(), data);
                Ok(Operation::RemoveFont { face })
            }
            Operation::RemoveFont { face } => {
                let in_use = self
                    .slides
                    .iter()
                    .flat_map(|slide| &slide.elements)
                    .filter_map(Element::as_text)
                    .any(|text| text.style.font == face);
                if in_use {
                    return Err(ApplyError::FontInUse(face));
                }
                let data = self
                    .fonts
                    .faces
                    .remove(&face)
                    .ok_or_else(|| ApplyError::MissingFont(face.clone()))?;
                Ok(Operation::AddFont { face, data })
            }
            Operation::Batch(operations) => {
                let mut inverses = Vec::with_capacity(operations.len());
                for operation in operations {
                    match self.apply(operation) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(error) => {
                            for inverse in inverses.into_iter().rev() {
                                self.apply(inverse)
                                    .expect("the inverse of an applied operation applies");
                            }
                            return Err(error);
                        }
                    }
                }
                inverses.reverse();
                Ok(Operation::Batch(inverses))
            }
        }
    }

    fn check_font(&self, face: &FontFace) -> Result<(), ApplyError> {
        if self.fonts.contains(face) {
            Ok(())
        } else {
            Err(ApplyError::MissingFont(face.clone()))
        }
    }
}

fn check_frame(frame: &Frame) -> Result<(), ApplyError> {
    let values = [frame.x, frame.y, frame.width, frame.height, frame.rotation];
    if values.iter().all(|value| value.is_finite()) && frame.width >= 0. && frame.height >= 0. {
        Ok(())
    } else {
        Err(ApplyError::InvalidFrame)
    }
}

pub use crate::operation::{ApplyError, Operation, TextStylePatch};

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn inter_regular() -> FontData {
        FontData {
            bytes: Arc::from(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
            index: 0,
        }
    }

    pub fn with_inter() -> Presentation {
        let mut presentation = Presentation::new();
        presentation
            .apply(Operation::AddFont {
                face: FontFace::new("Inter", 400, false),
                data: inter_regular(),
            })
            .unwrap();
        presentation
    }

    pub fn text(content: &str, sizing: TextSizing) -> ElementKind {
        ElementKind::Text(TextElement {
            content: content.into(),
            style: TextStyle::default(),
            sizing,
        })
    }

    /// Adds a text element to the first slide and returns its id.
    pub fn add_text(
        presentation: &mut Presentation,
        content: &str,
        sizing: TextSizing,
        frame: Frame,
    ) -> ElementId {
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        presentation
            .apply(Operation::AddElement {
                slide,
                index: usize::MAX,
                element: Element {
                    id,
                    frame,
                    kind: text(content, sizing),
                },
            })
            .unwrap();
        id
    }

    fn ids(presentation: &Presentation) -> Vec<u64> {
        presentation.slides.iter().map(|slide| slide.id.0).collect()
    }

    fn add_slide(presentation: &mut Presentation) -> SlideId {
        let id = presentation.new_slide_id();
        presentation
            .apply(Operation::AddSlide {
                index: usize::MAX,
                slide: Slide::new(id),
            })
            .unwrap();
        id
    }

    /// Applies `operation`, undoes, redoes and undoes it again, checking each
    /// state; the presentation ends as it started.
    /// Id counters are left out: undoing a creation does not free its id.
    fn round_trip(presentation: &mut Presentation, operation: Operation) {
        let content = |p: &Presentation| (p.slides.clone(), p.fonts.clone());
        let before = content(presentation);
        let inverse = presentation.apply(operation.clone()).unwrap();
        let after = content(presentation);
        assert_ne!(after, before, "{operation:?} changes something");
        let redo = presentation.apply(inverse).unwrap();
        assert_eq!(content(presentation), before, "undo of {operation:?}");
        let undo = presentation.apply(redo).unwrap();
        assert_eq!(content(presentation), after, "redo of {operation:?}");
        presentation.apply(undo).unwrap();
    }

    #[test]
    fn new_presentation_is_1600_by_900_with_one_slide() {
        let presentation = Presentation::new();
        assert_eq!(
            presentation.size,
            SlideSize {
                width: 1600,
                height: 900
            }
        );
        assert_eq!(ids(&presentation), [1]);
    }

    #[test]
    fn ids_follow_creation_order_and_survive_reordering() {
        let mut presentation = Presentation::new();
        let second = add_slide(&mut presentation);
        add_slide(&mut presentation);
        presentation
            .apply(Operation::MoveSlide {
                id: second,
                index: 0,
            })
            .unwrap();
        assert_eq!(ids(&presentation), [2, 1, 3]);
        presentation
            .apply(Operation::MoveSlide {
                id: second,
                index: 99,
            })
            .unwrap();
        assert_eq!(ids(&presentation), [1, 3, 2]);
    }

    #[test]
    fn removed_ids_are_never_reused() {
        let mut presentation = Presentation::new();
        let second = add_slide(&mut presentation);
        let third = add_slide(&mut presentation);
        presentation
            .apply(Operation::RemoveSlide { id: third })
            .unwrap();
        presentation
            .apply(Operation::RemoveSlide { id: second })
            .unwrap();
        assert_eq!(add_slide(&mut presentation), SlideId(4));
        assert_eq!(
            presentation.apply(Operation::RemoveSlide { id: third }),
            Err(ApplyError::UnknownSlide(third))
        );
    }

    #[test]
    fn the_last_slide_cannot_be_removed() {
        let mut presentation = Presentation::new();
        let id = presentation.slides[0].id;
        assert_eq!(
            presentation.apply(Operation::RemoveSlide { id }),
            Err(ApplyError::LastSlide)
        );
    }

    #[test]
    fn every_operation_round_trips() {
        let mut presentation = with_inter();
        let frame = Frame {
            x: 100.,
            y: 80.,
            width: 300.,
            height: 50.,
            rotation: 0.,
        };
        let id = add_text(
            &mut presentation,
            "Hello world",
            TextSizing::AutoHeight,
            frame,
        );
        let slide = add_slide(&mut presentation);

        round_trip(
            &mut presentation,
            Operation::AddSlide {
                index: 0,
                slide: Slide::new(SlideId(40)),
            },
        );
        round_trip(&mut presentation, Operation::RemoveSlide { id: slide });
        round_trip(
            &mut presentation,
            Operation::MoveSlide {
                id: slide,
                index: 0,
            },
        );
        round_trip(&mut presentation, Operation::RemoveElement { id });
        round_trip(
            &mut presentation,
            Operation::SetFrame {
                id,
                frame: Frame {
                    x: 5.,
                    width: 120.,
                    ..frame
                },
            },
        );
        round_trip(
            &mut presentation,
            Operation::SetTextSizing {
                id,
                sizing: TextSizing::AutoWidth,
            },
        );
        round_trip(
            &mut presentation,
            Operation::SetTextStyle {
                id,
                patch: TextStylePatch {
                    size: Some(64.),
                    underline: Some(true),
                    ..Default::default()
                },
            },
        );
        round_trip(
            &mut presentation,
            Operation::ReplaceText {
                id,
                range: 0..5,
                text: "Olá".into(),
            },
        );
        round_trip(
            &mut presentation,
            Operation::AddFont {
                face: FontFace::new("Inter", 400, true),
                data: inter_regular(),
            },
        );
        round_trip(
            &mut presentation,
            Operation::Batch(vec![
                Operation::ReplaceText {
                    id,
                    range: 11..11,
                    text: "!".into(),
                },
                Operation::SetTextSizing {
                    id,
                    sizing: TextSizing::Fixed,
                },
            ]),
        );
    }

    #[test]
    fn a_failing_batch_changes_nothing() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "Hi",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let before = presentation.clone();
        let result = presentation.apply(Operation::Batch(vec![
            Operation::ReplaceText {
                id,
                range: 0..0,
                text: "Oh, ".into(),
            },
            Operation::ReplaceText {
                id,
                range: 0..99,
                text: String::new(),
            },
        ]));
        assert_eq!(result, Err(ApplyError::InvalidRange(0..99)));
        assert_eq!(presentation, before);
    }

    #[test]
    fn text_can_only_use_embedded_fonts() {
        let mut presentation = Presentation::new();
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        let element = Element {
            id,
            frame: Frame::default(),
            kind: text("Hi", TextSizing::AutoWidth),
        };
        let add = Operation::AddElement {
            slide,
            index: 0,
            element,
        };
        assert!(matches!(
            presentation.apply(add.clone()),
            Err(ApplyError::MissingFont(_))
        ));
        let face = FontFace::new("Inter", 400, false);
        presentation
            .apply(Operation::Batch(vec![
                Operation::AddFont {
                    face: face.clone(),
                    data: inter_regular(),
                },
                add,
            ]))
            .unwrap();
        assert_eq!(
            presentation.apply(Operation::RemoveFont { face: face.clone() }),
            Err(ApplyError::FontInUse(face))
        );
    }

    #[test]
    fn element_ids_are_never_reused() {
        let mut presentation = with_inter();
        let first = add_text(
            &mut presentation,
            "a",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        presentation
            .apply(Operation::RemoveElement { id: first })
            .unwrap();
        let second = add_text(
            &mut presentation,
            "b",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        assert_ne!(first, second);
    }

    #[test]
    fn auto_sizes_are_derived_when_applying() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "Hello",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let short = presentation.element(id).unwrap().frame;
        assert!(short.width > 0. && short.height > 0.);

        presentation
            .apply(Operation::ReplaceText {
                id,
                range: 5..5,
                text: " world".into(),
            })
            .unwrap();
        let long = presentation.element(id).unwrap().frame;
        assert!(long.width > short.width);
        assert_eq!(long.height, short.height);

        // Wrapping at a narrow width makes the box taller, not wider.
        presentation
            .apply(Operation::Batch(vec![
                Operation::SetTextSizing {
                    id,
                    sizing: TextSizing::AutoHeight,
                },
                Operation::SetFrame {
                    id,
                    frame: Frame {
                        width: short.width,
                        ..long
                    },
                },
            ]))
            .unwrap();
        let wrapped = presentation.element(id).unwrap().frame;
        assert_eq!(wrapped.width, short.width);
        assert!(wrapped.height > long.height);

        // A fixed box keeps the height it is given.
        presentation
            .apply(Operation::Batch(vec![
                Operation::SetTextSizing {
                    id,
                    sizing: TextSizing::Fixed,
                },
                Operation::SetFrame {
                    id,
                    frame: Frame {
                        height: 10.,
                        ..wrapped
                    },
                },
            ]))
            .unwrap();
        assert_eq!(presentation.element(id).unwrap().frame.height, 10.);
    }

    #[test]
    fn style_patches_undo_only_the_fields_they_touch() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "Hi",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let undo_size = presentation
            .apply(Operation::SetTextStyle {
                id,
                patch: TextStylePatch {
                    size: Some(48.),
                    ..Default::default()
                },
            })
            .unwrap();
        presentation
            .apply(Operation::SetTextStyle {
                id,
                patch: TextStylePatch {
                    color: Some(Rgb(0xFF0000)),
                    ..Default::default()
                },
            })
            .unwrap();
        presentation.apply(undo_size).unwrap();
        let style = &presentation.element(id).unwrap().as_text().unwrap().style;
        assert_eq!(style.size, 32.);
        assert_eq!(style.color, Rgb(0xFF0000), "the later color change stays");
    }

    #[test]
    fn face_style_names() {
        assert_eq!(FontFace::new("Inter", 400, false).style_name(), "Regular");
        assert_eq!(FontFace::new("Inter", 400, true).style_name(), "Italic");
        assert_eq!(
            FontFace::new("Inter", 600, true).style_name(),
            "SemiBold Italic"
        );
        assert_eq!(FontFace::new("Inter", 450, false).style_name(), "450");
    }

    #[test]
    fn colors_parse_and_print_as_hex() {
        assert_eq!(Rgb::parse("#1a1a1a"), Some(Rgb(0x1A1A1A)));
        assert_eq!(Rgb::parse("F4F4F2"), Some(Rgb(0xF4F4F2)));
        assert_eq!(Rgb::parse("F4F4"), None);
        assert_eq!(Rgb(0x00FF00).hex(), "00FF00");
    }

    #[test]
    fn elements_read_from_json_with_style_defaults() {
        let element: Element = serde_json::from_str(
            r##"{"id": 3, "frame": {"x": 10, "width": 200},
                "text": {"content": "Hi", "sizing": "auto_height",
                         "style": {"size": 48, "color": "#FF0000",
                                   "line_height": {"percent": 120}}}}"##,
        )
        .unwrap();
        assert_eq!(element.id, ElementId(3));
        assert_eq!(element.frame.width, 200.);
        let text = element.as_text().unwrap();
        assert_eq!(text.sizing, TextSizing::AutoHeight);
        assert_eq!(text.style.size, 48.);
        assert_eq!(text.style.color, Rgb(0xFF0000));
        assert_eq!(text.style.line_height, LineHeight::Percent(120.));
        assert_eq!(text.style.font, FontFace::new("Inter", 400, false));

        let typo = serde_json::from_str::<TextStyle>(r#"{"sise": 12}"#);
        assert!(typo.is_err(), "unknown style fields are rejected");
    }
}
