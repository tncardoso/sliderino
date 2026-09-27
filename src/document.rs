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

    /// Every element of the slide, depth first in paint order: a group comes
    /// before its children.
    pub fn walk(&self) -> Vec<Node<'_>> {
        let mut nodes = Vec::new();
        walk_into(&self.elements, None, &mut nodes);
        nodes
    }

    /// The text elements drawn on the slide, in paint order: hidden ones and
    /// the children of hidden groups are left out.
    pub fn visible_texts(&self) -> impl Iterator<Item = (&Element, &TextElement)> {
        self.walk()
            .into_iter()
            .filter(|node| !node.hidden)
            .filter_map(|node| Some((node.element, node.element.as_text()?)))
    }

    /// The elements drawn on the slide, groups left out, in paint order:
    /// hidden ones and the children of hidden groups are left out.
    pub fn visible_leaves(&self) -> impl Iterator<Item = Node<'_>> {
        self.walk()
            .into_iter()
            .filter(|node| !node.hidden && node.element.as_group().is_none())
    }
}

/// An element met by [`Slide::walk`].
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    pub element: &'a Element,
    pub parent: Option<ElementId>,
    /// 0 for the elements of the slide itself.
    pub depth: usize,
    /// The element or one of its ancestors is hidden.
    pub hidden: bool,
    /// The element or one of its ancestors is locked.
    pub locked: bool,
    /// The opacity of the element times the opacities of its ancestors.
    pub opacity: f32,
}

fn walk_into<'a>(elements: &'a [Element], parent: Option<&Node<'a>>, nodes: &mut Vec<Node<'a>>) {
    for element in elements {
        let node = Node {
            element,
            parent: parent.map(|parent| parent.element.id),
            depth: parent.map_or(0, |parent| parent.depth + 1),
            hidden: element.hidden || parent.is_some_and(|parent| parent.hidden),
            locked: element.locked || parent.is_some_and(|parent| parent.locked),
            opacity: element.opacity * parent.map_or(1., |parent| parent.opacity),
        };
        nodes.push(node);
        walk_into(element.children(), Some(&node), nodes);
    }
}

/// Where an element sits: its slide, its parent group (`None` for the slide
/// itself) and its index among its siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub slide: SlideId,
    pub parent: Option<ElementId>,
    pub index: usize,
}

fn find(elements: &[Element], id: ElementId) -> Option<&Element> {
    elements.iter().find_map(|element| {
        if element.id == id {
            Some(element)
        } else {
            find(element.children(), id)
        }
    })
}

fn find_mut(elements: &mut [Element], id: ElementId) -> Option<&mut Element> {
    for element in elements {
        if element.id == id {
            return Some(element);
        }
        if let Some(group) = element.as_group_mut()
            && let Some(found) = find_mut(&mut group.children, id)
        {
            return Some(found);
        }
    }
    None
}

/// Parent and index of `id` among `elements` and their descendants.
fn position(
    elements: &[Element],
    parent: Option<ElementId>,
    id: ElementId,
) -> Option<(Option<ElementId>, usize)> {
    elements.iter().enumerate().find_map(|(index, element)| {
        if element.id == id {
            Some((parent, index))
        } else {
            position(element.children(), Some(element.id), id)
        }
    })
}

/// Calls `f` on the element and each of its descendants.
fn each_in_tree<'a>(element: &'a Element, f: &mut impl FnMut(&'a Element)) {
    f(element);
    for child in element.children() {
        each_in_tree(child, f);
    }
}

/// Moves the frames of the element and its descendants by `dx`, `dy`.
fn translate(element: &mut Element, dx: f32, dy: f32) {
    element.frame.x += dx;
    element.frame.y += dy;
    if let Some(group) = element.as_group_mut() {
        for child in &mut group.children {
            translate(child, dx, dy);
        }
    }
}

/// Sets the frame of each group to the box around its children's frames,
/// in the axes of the group's rotation, children first. An empty group
/// keeps its frame.
fn refit_groups(elements: &mut [Element]) {
    for element in elements {
        let Some(group) = element.as_group_mut() else {
            continue;
        };
        refit_groups(&mut group.children);
        let rotation = element.frame.rotation;
        let ElementKind::Group(group) = &element.kind else {
            continue;
        };
        let children = &group.children;
        if let Some(bounds) = union_in(children.iter().map(|child| &child.frame), rotation) {
            element.frame = bounds;
        }
    }
}

/// The smallest unrotated frame holding every frame, rotated ones included;
/// `None` for none.
pub fn union<'a>(frames: impl IntoIterator<Item = &'a Frame>) -> Option<Frame> {
    union_in(frames, 0.)
}

/// The smallest frame with `rotation` that holds every frame; `None` for
/// none. The frame of a rotated group.
pub fn union_in<'a>(frames: impl IntoIterator<Item = &'a Frame>, rotation: f32) -> Option<Frame> {
    // Corners in the axes of `rotation`, turned around the slide origin.
    let mut span: Option<(f32, f32, f32, f32)> = None;
    for frame in frames {
        for (x, y) in frame.corners() {
            let (u, v) = rotate_vector(x, y, -rotation);
            span = Some(match span {
                None => (u, v, u, v),
                Some((left, top, right, bottom)) => {
                    (left.min(u), top.min(v), right.max(u), bottom.max(v))
                }
            });
        }
    }
    let (left, top, right, bottom) = span?;
    let (width, height) = (right - left, bottom - top);
    if rotation == 0. {
        return Some(Frame {
            x: left,
            y: top,
            width,
            height,
            rotation: 0.,
        });
    }
    let (cx, cy) = rotate_vector(left + width / 2., top + height / 2., rotation);
    Some(Frame {
        x: cx - width / 2.,
        y: cy - height / 2.,
        width,
        height,
        rotation,
    })
}

/// The frame turned by `degrees` around `pivot`: its center goes around the
/// pivot and its rotation grows by the same angle.
pub fn turn_frame(frame: &Frame, pivot: (f32, f32), degrees: f32) -> Frame {
    let (cx, cy) = frame.center();
    let (dx, dy) = rotate_vector(cx - pivot.0, cy - pivot.1, degrees);
    Frame {
        x: pivot.0 + dx - frame.width / 2.,
        y: pivot.1 + dy - frame.height / 2.,
        rotation: normalize_degrees(frame.rotation + degrees),
        ..*frame
    }
}

/// Turns the vector (`x`, `y`) by `degrees`, clockwise on the slide (y
/// points down).
pub fn rotate_vector(x: f32, y: f32, degrees: f32) -> (f32, f32) {
    if degrees == 0. {
        return (x, y);
    }
    let (sin, cos) = sin_cos(degrees);
    (x * cos - y * sin, x * sin + y * cos)
}

/// Sine and cosine of an angle in degrees, exact for multiples of 90°.
fn sin_cos(degrees: f32) -> (f32, f32) {
    match normalize_degrees(degrees) {
        0. => (0., 1.),
        90. => (1., 0.),
        180. => (0., -1.),
        -90. => (-1., 0.),
        other => other.to_radians().sin_cos(),
    }
}

/// Two lengths or angles that differ only by rounding.
fn nearly(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3
}

/// The same angle in (-180, 180].
pub fn normalize_degrees(degrees: f32) -> f32 {
    let turned = degrees.rem_euclid(360.);
    if turned > 180. {
        turned - 360.
    } else if turned == 0. {
        0.
    } else {
        turned
    }
}

/// How a child follows its group in [`map_frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMode {
    /// The box scales with the group.
    Box,
    /// The box keeps its size: text that sizes its own width.
    KeepSize,
    /// The two ends of a line follow the group.
    Line,
}

impl MapMode {
    pub fn of(element: &Element) -> MapMode {
        match &element.kind {
            ElementKind::Text(text) if text.sizing == TextSizing::AutoWidth => MapMode::KeepSize,
            ElementKind::Line(_) => MapMode::Line,
            _ => MapMode::Box,
        }
    }
}

/// The frame a child takes when its group goes from `from` to `to`, both
/// possibly rotated. The rotated top-left corner of the child follows the
/// group: it keeps its place in the group's axes, scaled with the group. The
/// child turns with the group, and its box scales with it unless the text
/// sizes its own width. Font sizes do not change.
///
/// A child turned by an angle that is not a multiple of 90° inside the group
/// cannot stretch along one group axis without a skew: its box scales along
/// the group axis nearest to each of its own, which is close enough. A line
/// has no such limit: both of its ends follow the group exactly.
pub fn map_frame(child: &Frame, from: &Frame, to: &Frame, mode: MapMode) -> Frame {
    let ratio = |to: f32, from: f32| if from > 0. { to / from } else { 1. };
    let sx = ratio(to.width, from.width);
    let sy = ratio(to.height, from.height);
    let map = |(x, y): (f32, f32)| {
        let (u, v) = from.to_local(x, y);
        to.to_slide(u * sx, v * sy)
    };
    if mode == MapMode::Line {
        let (start, end) = child.line_ends();
        let turned = normalize_degrees(child.rotation + to.rotation - from.rotation);
        return Frame::from_line(map(start), map(end), turned);
    }
    let (x, y) = map(child.to_slide(0., 0.));
    let (sin, cos) = sin_cos(child.rotation - from.rotation);
    let (fx, fy) = if sin.abs() > cos.abs() {
        (sy, sx)
    } else {
        (sx, sy)
    };
    let keeps_size = mode == MapMode::KeepSize;
    let frame = Frame {
        x: 0.,
        y: 0.,
        width: if keeps_size {
            child.width
        } else {
            child.width * fx
        },
        height: if keeps_size {
            child.height
        } else {
            child.height * fy
        },
        rotation: normalize_degrees(child.rotation + to.rotation - from.rotation),
    };
    frame.with_top_left_at(x, y)
}

/// In JSON the kind is a key of the element: `{"id": 1, "frame": {..},
/// "text": {..}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    /// Name shown in the hierarchy; `None` shows a name made from the
    /// content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Not drawn, exported or reported. Hides the children of a group.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Rejects every operation but [`Operation::SetLayer`] on the element and
    /// its descendants. An unlocked ancestor still moves it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// 0.0 to 1.0. A group multiplies the opacity of its children.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
    #[serde(default)]
    pub frame: Frame,
    #[serde(flatten)]
    pub kind: ElementKind,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn one() -> f32 {
    1.
}

fn is_one(value: &f32) -> bool {
    *value == 1.
}

fn is_zero(value: &f32) -> bool {
    *value == 0.
}

impl Element {
    /// A visible, unlocked element without a name.
    pub fn new(id: ElementId, frame: Frame, kind: ElementKind) -> Self {
        Self {
            id,
            name: None,
            hidden: false,
            locked: false,
            opacity: 1.,
            frame,
            kind,
        }
    }

    pub fn as_text(&self) -> Option<&TextElement> {
        match &self.kind {
            ElementKind::Text(text) => Some(text),
            _ => None,
        }
    }

    fn as_text_mut(&mut self) -> Option<&mut TextElement> {
        match &mut self.kind {
            ElementKind::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_group(&self) -> Option<&GroupElement> {
        match &self.kind {
            ElementKind::Group(group) => Some(group),
            _ => None,
        }
    }

    fn as_group_mut(&mut self) -> Option<&mut GroupElement> {
        match &mut self.kind {
            ElementKind::Group(group) => Some(group),
            _ => None,
        }
    }

    pub fn is_line(&self) -> bool {
        matches!(self.kind, ElementKind::Line(_))
    }

    /// The element's children; empty for an element that is not a group.
    pub fn children(&self) -> &[Element] {
        self.as_group().map_or(&[], |group| &group.children)
    }
}

/// Position and size of an element, in slide units from the slide's top-left
/// corner. Rotation is in degrees, clockwise, around the frame center, in
/// (-180, 180]: `x`, `y`, `width` and `height` describe the frame before it
/// turns.
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
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2., self.y + self.height / 2.)
    }

    /// The slide point of a point given from the top-left corner of the
    /// unrotated frame.
    pub fn to_slide(&self, x: f32, y: f32) -> (f32, f32) {
        if self.rotation == 0. {
            return (self.x + x, self.y + y);
        }
        let (cx, cy) = self.center();
        let (dx, dy) = rotate_vector(x - self.width / 2., y - self.height / 2., self.rotation);
        (cx + dx, cy + dy)
    }

    /// The inverse of [`Self::to_slide`].
    pub fn to_local(&self, x: f32, y: f32) -> (f32, f32) {
        if self.rotation == 0. {
            return (x - self.x, y - self.y);
        }
        let (cx, cy) = self.center();
        let (dx, dy) = rotate_vector(x - cx, y - cy, -self.rotation);
        (dx + self.width / 2., dy + self.height / 2.)
    }

    /// The corners on the slide: top-left, top-right, bottom-right and
    /// bottom-left of the unrotated frame.
    pub fn corners(&self) -> [(f32, f32); 4] {
        [
            self.to_slide(0., 0.),
            self.to_slide(self.width, 0.),
            self.to_slide(self.width, self.height),
            self.to_slide(0., self.height),
        ]
    }

    /// The smallest unrotated frame holding this one.
    pub fn bounds(&self) -> Frame {
        union(std::iter::once(self)).expect("one frame")
    }

    /// The same size and rotation, placed so that its rotated top-left
    /// corner sits at (`x`, `y`).
    pub fn with_top_left_at(self, x: f32, y: f32) -> Frame {
        if self.rotation == 0. {
            return Frame { x, y, ..self };
        }
        let (dx, dy) = rotate_vector(-self.width / 2., -self.height / 2., self.rotation);
        Frame {
            x: x - dx - self.width / 2.,
            y: y - dy - self.height / 2.,
            ..self
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (x, y) = self.to_local(x, y);
        x >= 0. && x <= self.width && y >= 0. && y <= self.height
    }

    /// The start and end of a line: the ends of the horizontal center axis
    /// of the frame, on the slide.
    pub fn line_ends(&self) -> ((f32, f32), (f32, f32)) {
        (
            self.to_slide(0., self.height / 2.),
            self.to_slide(self.width, self.height / 2.),
        )
    }

    /// The frame of a line from `start` to `end`: its center is the middle of
    /// the line, its width the length and its rotation the angle. A line of
    /// no length keeps `rotation`.
    pub fn from_line(start: (f32, f32), end: (f32, f32), rotation: f32) -> Frame {
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        let length = dx.hypot(dy);
        let rotation = if length > 1e-4 {
            let degrees = dy.atan2(dx).to_degrees();
            // Keep exact multiples of 90° exact.
            let rounded = degrees.round();
            normalize_degrees(if nearly(degrees, rounded) {
                rounded
            } else {
                degrees
            })
        } else {
            normalize_degrees(rotation)
        };
        let (cx, cy) = ((start.0 + end.0) / 2., (start.1 + end.1) / 2.);
        Frame {
            x: cx - length / 2.,
            y: cy,
            width: length,
            height: 0.,
            rotation,
        }
    }

    /// The two frames overlap or touch, rotated ones included.
    pub fn intersects(&self, other: &Frame) -> bool {
        let (a, b) = (self.corners(), other.corners());
        let axes = [
            self.rotation,
            self.rotation + 90.,
            other.rotation,
            other.rotation + 90.,
        ];
        axes.into_iter().all(|degrees| {
            let (ax, ay) = rotate_vector(1., 0., degrees);
            let span = |corners: &[(f32, f32); 4]| {
                corners
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(low, high), (x, y)| {
                        let at = x * ax + y * ay;
                        (low.min(at), high.max(at))
                    })
            };
            let (a, b) = (span(&a), span(&b));
            a.0 <= b.1 && b.0 <= a.1
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Text(TextElement),
    Group(GroupElement),
    Rectangle(RectangleElement),
    Ellipse(EllipseElement),
    Line(LineElement),
}

impl ElementKind {
    /// A rectangle, an ellipse or a line.
    pub fn is_shape(&self) -> bool {
        matches!(
            self,
            ElementKind::Rectangle(_) | ElementKind::Ellipse(_) | ElementKind::Line(_)
        )
    }

    /// The fill of a rectangle or an ellipse.
    pub fn fill(&self) -> Option<&Fill> {
        match self {
            ElementKind::Rectangle(shape) => Some(&shape.fill),
            ElementKind::Ellipse(shape) => Some(&shape.fill),
            _ => None,
        }
    }

    /// The stroke of a shape; `None` for a shape without one.
    pub fn stroke(&self) -> Option<&Stroke> {
        match self {
            ElementKind::Rectangle(shape) => shape.stroke.as_ref(),
            ElementKind::Ellipse(shape) => shape.stroke.as_ref(),
            ElementKind::Line(line) => Some(&line.stroke),
            _ => None,
        }
    }

    /// Checks the style values of a shape; other kinds pass.
    pub fn validate(&self) -> Result<(), ApplyError> {
        if let Some(fill) = self.fill() {
            fill.validate()?;
        }
        if let Some(stroke) = self.stroke() {
            stroke.validate()?;
        }
        if let ElementKind::Rectangle(shape) = self
            && !(shape.corner_radius.is_finite() && shape.corner_radius >= 0.)
        {
            return Err(ApplyError::InvalidStyle);
        }
        Ok(())
    }
}

/// A rectangle, with corners rounded when `corner_radius` is more than 0.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectangleElement {
    #[serde(default)]
    pub fill: Fill,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// In slide units, the same for every corner. A radius larger than half
    /// the shorter side draws as half the shorter side.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corner_radius: f32,
}

/// An ellipse that touches the four sides of its frame; a circle when the
/// frame is square.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EllipseElement {
    #[serde(default)]
    pub fill: Fill,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

/// A straight line. Its frame has a height of 0: the line goes from the
/// left end to the right end of the frame, and the rotation of the frame is
/// its angle. See [`Frame::line_ends`] and [`Frame::from_line`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineElement {
    #[serde(default)]
    pub stroke: Stroke,
    /// Drawn at the start of the line.
    #[serde(default, skip_serializing_if = "Arrowhead::is_none")]
    pub start: Arrowhead,
    /// Drawn at the end of the line.
    #[serde(default, skip_serializing_if = "Arrowhead::is_none")]
    pub end: Arrowhead,
}

/// Elements moved, resized and turned together. The frame of a group is the
/// box around its children's frames in the axes of the group's rotation,
/// kept by [`Presentation::apply`]. Children frames stay in slide units,
/// like every frame, so a child's rotation is its angle on the slide.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupElement {
    /// Children in paint order: the last one is on top.
    #[serde(default)]
    pub children: Vec<Element>,
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

    /// The embedded faces, sorted.
    pub fn faces(&self) -> impl Iterator<Item = &FontFace> {
        self.faces.keys()
    }

    /// The embedded faces and their bytes, sorted by face.
    pub fn iter(&self) -> impl Iterator<Item = (&FontFace, &FontData)> {
        self.faces.iter()
    }
}

/// The ids a presentation gives to the next slide, element, image and
/// video. Ids are never reused, so a saved presentation keeps them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextIds {
    pub slide: u64,
    pub element: u64,
    pub image: u64,
    pub video: u64,
}

#[derive(Clone, Debug)]
pub struct Presentation {
    pub size: SlideSize,
    /// Slides in presentation order.
    pub slides: Vec<Slide>,
    pub fonts: FontLibrary,
    pub images: ImageLibrary,
    pub videos: VideoLibrary,
    /// Id given to the next created slide; only ever grows.
    next_slide_id: u64,
    /// Id given to the next created element; only ever grows.
    next_element_id: u64,
    /// Id given to the next embedded image; only ever grows.
    next_image_id: u64,
    /// Id given to the next embedded video; only ever grows.
    next_video_id: u64,
    /// Counts the changes applied, undo and redo included. External agents
    /// compare it to detect edits made since they last read the document.
    revision: u64,
}

/// Compares the content only: two presentations reached by different edits
/// are equal, whatever their [`Presentation::revision`].
impl PartialEq for Presentation {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self.slides == other.slides
            && self.fonts == other.fonts
            && self.images == other.images
            && self.next_slide_id == other.next_slide_id
            && self.next_element_id == other.next_element_id
            && self.next_image_id == other.next_image_id
            && self.videos == other.videos
            && self.next_video_id == other.next_video_id
    }
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
            images: ImageLibrary::default(),
            videos: VideoLibrary::default(),
            next_slide_id: 1,
            next_element_id: 1,
            next_image_id: 1,
            next_video_id: 1,
            revision: 0,
        };
        let id = presentation.new_slide_id();
        presentation.slides.push(Slide::new(id));
        presentation
    }

    /// Builds a saved presentation again through the checks of the edits:
    /// the fonts must be readable, and every element must reference
    /// embedded fonts, images and videos. The ids to give next never go
    /// below `next`. The revision starts at 0.
    pub fn from_saved(
        size: SlideSize,
        fonts: Vec<(FontFace, FontData)>,
        images: Vec<(ImageId, ImageData)>,
        videos: Vec<(VideoId, VideoData)>,
        slides: Vec<Slide>,
        next: NextIds,
    ) -> Result<Self, ApplyError> {
        let mut presentation = Self {
            size,
            slides: Vec::new(),
            fonts: FontLibrary::default(),
            images: ImageLibrary::default(),
            videos: VideoLibrary::default(),
            next_slide_id: next.slide.max(1),
            next_element_id: next.element.max(1),
            next_image_id: next.image.max(1),
            next_video_id: next.video.max(1),
            revision: 0,
        };
        for (face, data) in fonts {
            presentation.apply_op(Operation::AddFont { face, data })?;
        }
        for (id, data) in images {
            presentation.apply_op(Operation::AddImage { id, data })?;
        }
        for (id, data) in videos {
            presentation.apply_op(Operation::AddVideo { id, data })?;
        }
        for slide in slides {
            let id = slide.id;
            presentation.apply_one(Operation::AddSlide {
                index: usize::MAX,
                slide: Slide::new(id),
            })?;
            for element in slide.elements {
                presentation.apply_one(Operation::AddElement {
                    slide: id,
                    parent: None,
                    index: usize::MAX,
                    element,
                })?;
            }
        }
        if presentation.slides.is_empty() {
            let id = presentation.new_slide_id();
            presentation.slides.push(Slide::new(id));
        }
        Ok(presentation)
    }

    /// The ids the presentation gives next, for [`Self::from_saved`].
    pub fn next_ids(&self) -> NextIds {
        NextIds {
            slide: self.next_slide_id,
            element: self.next_element_id,
            image: self.next_image_id,
            video: self.next_video_id,
        }
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

    /// A copy of slide `id` for an [`Operation::AddSlide`], with a new slide
    /// id and new ids for all its elements, groups and their children
    /// included. Images, videos and fonts are shared by id.
    pub fn copy_slide(&mut self, id: SlideId) -> Option<Slide> {
        fn renumber(presentation: &mut Presentation, elements: &mut [Element]) {
            for element in elements {
                element.id = presentation.new_element_id();
                if let Some(group) = element.as_group_mut() {
                    renumber(presentation, &mut group.children);
                }
            }
        }
        let mut slide = self.slide(id)?.clone();
        slide.id = self.new_slide_id();
        renumber(self, &mut slide.elements);
        Some(slide)
    }

    /// The [`Operation::MoveSlide`]s that put the slides `moved` together,
    /// in their present order, at `to`: an index in the present list of
    /// slides (`len` for the end). `None` when the order does not change.
    pub fn slide_moves(&self, moved: &[SlideId], to: usize) -> Option<Vec<Operation>> {
        let mut current: Vec<SlideId> = self.slides.iter().map(|slide| slide.id).collect();
        let is_moved = |id: &SlideId| moved.contains(id);
        let block: Vec<SlideId> = current.iter().copied().filter(is_moved).collect();
        let mut target: Vec<SlideId> = current.iter().copied().filter(|id| !is_moved(id)).collect();
        let at = current[..to.min(current.len())]
            .iter()
            .filter(|id| !is_moved(id))
            .count();
        target.splice(at..at, block);
        let mut operations = Vec::new();
        for (index, id) in target.iter().enumerate() {
            if current[index] != *id {
                let from = current.iter().position(|slide| slide == id)?;
                current.remove(from);
                current.insert(index, *id);
                operations.push(Operation::MoveSlide { id: *id, index });
            }
        }
        (!operations.is_empty()).then_some(operations)
    }

    /// Reserves an image id for an [`Operation::AddImage`].
    pub fn new_image_id(&mut self) -> ImageId {
        let id = ImageId(self.next_image_id);
        self.next_image_id += 1;
        id
    }

    /// Reserves a video id for an [`Operation::AddVideo`].
    pub fn new_video_id(&mut self) -> VideoId {
        let id = VideoId(self.next_video_id);
        self.next_video_id += 1;
        id
    }

    /// Whether a fill of an element of any slide matches.
    fn fill_in_use(&self, uses: impl Fn(&Fill) -> bool) -> bool {
        self.slides.iter().any(|slide| {
            slide
                .walk()
                .iter()
                .any(|node| node.element.kind.fill().is_some_and(&uses))
        })
    }

    /// Whether a fill of an element of any slide uses the image, as its
    /// picture or as the channel of its shader.
    pub fn image_in_use(&self, id: ImageId) -> bool {
        self.fill_in_use(|fill| match fill {
            Fill::Image(image) => image.id == id,
            Fill::Shader(shader) => shader.channel0 == Some(id),
            _ => false,
        })
    }

    /// Whether a fill of an element of any slide uses the video.
    pub fn video_in_use(&self, id: VideoId) -> bool {
        self.fill_in_use(|fill| matches!(fill, Fill::Video(video) if video.id == id))
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
            .find_map(|slide| find(&slide.elements, id))
    }

    /// The slide, parent and index of the element.
    pub fn locate(&self, id: ElementId) -> Option<Location> {
        self.slides.iter().find_map(|slide| {
            let (parent, index) = position(&slide.elements, None, id)?;
            Some(Location {
                slide: slide.id,
                parent,
                index,
            })
        })
    }

    /// Groups holding the element, from its parent up to the slide.
    pub fn ancestors(&self, id: ElementId) -> Vec<ElementId> {
        let mut ancestors = Vec::new();
        let mut current = id;
        while let Some(parent) = self.locate(current).and_then(|location| location.parent) {
            ancestors.push(parent);
            current = parent;
        }
        ancestors
    }

    /// The element or one of its ancestors is locked.
    pub fn is_locked(&self, id: ElementId) -> bool {
        self.flag_in_chain(id, |element| element.locked)
    }

    /// The element or one of its ancestors is hidden.
    pub fn is_hidden(&self, id: ElementId) -> bool {
        self.flag_in_chain(id, |element| element.hidden)
    }

    fn flag_in_chain(&self, id: ElementId, flag: impl Fn(&Element) -> bool) -> bool {
        std::iter::once(id)
            .chain(self.ancestors(id))
            .any(|id| self.element(id).is_some_and(&flag))
    }

    fn check_unlocked(&self, id: ElementId) -> Result<(), ApplyError> {
        if self.is_locked(id) {
            Err(ApplyError::Locked(id))
        } else {
            Ok(())
        }
    }

    fn element_mut(&mut self, id: ElementId) -> Result<&mut Element, ApplyError> {
        self.slides
            .iter_mut()
            .find_map(|slide| find_mut(&mut slide.elements, id))
            .ok_or(ApplyError::UnknownElement(id))
    }

    /// The children of `parent` on the slide at `slide_index`, or the
    /// elements of the slide itself when `parent` is `None`.
    fn children_mut(
        &mut self,
        slide_index: usize,
        parent: Option<ElementId>,
    ) -> Result<&mut Vec<Element>, ApplyError> {
        let slide = &mut self.slides[slide_index];
        match parent {
            None => Ok(&mut slide.elements),
            Some(parent) => {
                let element = find_mut(&mut slide.elements, parent)
                    .ok_or(ApplyError::UnknownElement(parent))?;
                Ok(&mut element
                    .as_group_mut()
                    .ok_or(ApplyError::NotGroup(parent))?
                    .children)
            }
        }
    }

    /// Checks that `parent` can receive a child on `slide`.
    fn check_parent(&self, slide: SlideId, parent: Option<ElementId>) -> Result<(), ApplyError> {
        let Some(parent) = parent else {
            return Ok(());
        };
        let location = self
            .locate(parent)
            .ok_or(ApplyError::UnknownElement(parent))?;
        if location.slide != slide {
            return Err(ApplyError::InvalidParent(parent));
        }
        if self.element(parent).and_then(Element::as_group).is_none() {
            return Err(ApplyError::NotGroup(parent));
        }
        self.check_unlocked(parent)
    }

    /// The operations that put the elements in a new group `group`, as one
    /// batch. The group takes the place of the topmost element; the elements
    /// keep their paint order. An element inside another listed element
    /// stays where it is.
    pub fn group_operations(
        &self,
        group: ElementId,
        ids: &[ElementId],
    ) -> Result<Vec<Operation>, ApplyError> {
        let mut located = Vec::new();
        for &id in ids {
            let location = self.locate(id).ok_or(ApplyError::UnknownElement(id))?;
            located.push((id, location));
        }
        let Some(&(_, first)) = located.first() else {
            return Err(ApplyError::EmptyGroup);
        };
        if located
            .iter()
            .any(|(_, location)| location.slide != first.slide)
        {
            return Err(ApplyError::GroupAcrossSlides);
        }
        let slide = self.slide(first.slide).expect("located slide exists");
        let order: Vec<ElementId> = slide.walk().iter().map(|node| node.element.id).collect();
        let mut members: Vec<ElementId> = located
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| {
                !self
                    .ancestors(*id)
                    .iter()
                    .any(|ancestor| ids.contains(ancestor))
            })
            .collect();
        members.sort_by_key(|id| order.iter().position(|other| other == id));
        members.dedup();
        let top = *members.last().expect("at least one element");
        let place = self.locate(top).expect("located above");
        let frames: Vec<Frame> = members
            .iter()
            .filter_map(|id| self.element(*id).map(|element| element.frame))
            .collect();
        let frame = union(&frames).unwrap_or_default();
        let mut operations = vec![Operation::AddElement {
            slide: place.slide,
            parent: place.parent,
            index: place.index + 1,
            element: Element::new(group, frame, ElementKind::Group(GroupElement::default())),
        }];
        operations.extend(members.into_iter().map(|id| Operation::MoveElement {
            id,
            parent: Some(group),
            index: usize::MAX,
        }));
        Ok(operations)
    }

    /// The operations that put the children of a group in its place and
    /// remove it, as one batch.
    pub fn ungroup_operations(&self, id: ElementId) -> Result<Vec<Operation>, ApplyError> {
        let element = self.element(id).ok_or(ApplyError::UnknownElement(id))?;
        let group = element.as_group().ok_or(ApplyError::NotGroup(id))?;
        let place = self.locate(id).expect("the element exists");
        let mut operations: Vec<Operation> = group
            .children
            .iter()
            .enumerate()
            .map(|(offset, child)| Operation::MoveElement {
                id: child.id,
                parent: place.parent,
                index: place.index + offset,
            })
            .collect();
        operations.push(Operation::RemoveElement { id });
        Ok(operations)
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
        let _span = crate::perf::span("fit");
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
        let before = *frame;
        if sizing == TextSizing::AutoWidth {
            frame.width = size.0;
        }
        frame.height = size.1;
        // A rotated box grows from its rotated top-left corner, as an
        // unrotated one does, instead of from its center.
        if frame.rotation != 0. && (frame.width != before.width || frame.height != before.height) {
            let (x, y) = before.to_slide(0., 0.);
            *frame = frame.with_top_left_at(x, y);
        }
        Ok(())
    }

    /// Number of changes applied so far; see the field.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies one edit and returns the operation that undoes it.
    ///
    /// On error the presentation is left unchanged, including for a
    /// [`Operation::Batch`] that fails halfway. A successful edit increments
    /// the [revision](Self::revision).
    pub fn apply(&mut self, operation: Operation) -> Result<Operation, ApplyError> {
        let inverse = self.apply_one(operation)?;
        self.revision += 1;
        Ok(inverse)
    }

    /// Applies one operation and, unless it is a batch, fits the frames of
    /// the groups to their children again.
    fn apply_one(&mut self, operation: Operation) -> Result<Operation, ApplyError> {
        let batch = matches!(operation, Operation::Batch(_));
        let inverse = self.apply_op(operation)?;
        if !batch {
            for slide in &mut self.slides {
                refit_groups(&mut slide.elements);
            }
        }
        Ok(inverse)
    }

    fn apply_op(&mut self, operation: Operation) -> Result<Operation, ApplyError> {
        match operation {
            Operation::AddSlide { index, slide } => {
                if self.index_of(slide.id).is_some() {
                    return Err(ApplyError::DuplicateSlide(slide.id));
                }
                let nodes = slide.walk();
                if let Some(node) = nodes
                    .iter()
                    .find(|node| self.element(node.element.id).is_some())
                {
                    return Err(ApplyError::DuplicateElement(node.element.id));
                }
                self.next_slide_id = self.next_slide_id.max(slide.id.0 + 1);
                for node in &nodes {
                    self.next_element_id = self.next_element_id.max(node.element.id.0 + 1);
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
                parent,
                index,
                mut element,
            } => {
                let slide_index = self
                    .index_of(slide)
                    .ok_or(ApplyError::UnknownSlide(slide))?;
                normalize_frames(&mut element);
                self.check_parent(slide, parent)?;
                let mut tree = Vec::new();
                each_in_tree(&element, &mut |element| tree.push(element));
                let mut seen = std::collections::HashSet::new();
                for node in &tree {
                    if self.element(node.id).is_some() || !seen.insert(node.id) {
                        return Err(ApplyError::DuplicateElement(node.id));
                    }
                    if let Some(text) = node.as_text() {
                        self.check_font(&text.style.font)?;
                    }
                    check_frame(&node.frame)?;
                    check_opacity(node.opacity)?;
                    node.kind.validate()?;
                    if let Some(fill) = node.kind.fill() {
                        self.check_fill(fill)?;
                    }
                }
                let ids: Vec<ElementId> = tree.iter().map(|node| node.id).collect();
                let id = element.id;
                for &id in &ids {
                    self.next_element_id = self.next_element_id.max(id.0 + 1);
                }
                let elements = self.children_mut(slide_index, parent)?;
                let index = index.min(elements.len());
                elements.insert(index, element);
                for id in ids {
                    self.fit(id)?;
                }
                Ok(Operation::RemoveElement { id })
            }
            Operation::RemoveElement { id } => {
                let location = self.locate(id).ok_or(ApplyError::UnknownElement(id))?;
                self.check_unlocked(id)?;
                let slide_index = self.index_of(location.slide).expect("located slide exists");
                let element = self
                    .children_mut(slide_index, location.parent)?
                    .remove(location.index);
                Ok(Operation::AddElement {
                    slide: location.slide,
                    parent: location.parent,
                    index: location.index,
                    element,
                })
            }
            Operation::MoveElement { id, parent, index } => {
                let from = self.locate(id).ok_or(ApplyError::UnknownElement(id))?;
                self.check_unlocked(id)?;
                self.check_parent(from.slide, parent)?;
                if let Some(parent) = parent
                    && (parent == id || self.ancestors(parent).contains(&id))
                {
                    return Err(ApplyError::InvalidParent(parent));
                }
                let slide_index = self.index_of(from.slide).expect("located slide exists");
                let element = self
                    .children_mut(slide_index, from.parent)?
                    .remove(from.index);
                let elements = self.children_mut(slide_index, parent)?;
                let index = index.min(elements.len());
                elements.insert(index, element);
                Ok(Operation::MoveElement {
                    id,
                    parent: from.parent,
                    index: from.index,
                })
            }
            Operation::SetFrame { id, frame } => {
                check_frame(&frame)?;
                let mut frame = Frame {
                    rotation: normalize_degrees(frame.rotation),
                    ..frame
                };
                self.check_unlocked(id)?;
                let element = self.element_mut(id)?;
                if element.is_line() {
                    frame = normalize_line(frame);
                }
                let old = element.frame;
                if element.as_group().is_some() {
                    return self.set_group_frame(id, old, frame);
                }
                element.frame = frame;
                // A frame is always fitted to its text, so a move alone
                // cannot change the size the text needs.
                if old.width != frame.width || old.height != frame.height {
                    self.fit(id)?;
                }
                Ok(Operation::SetFrame { id, frame: old })
            }
            Operation::SetGroupFrames { id, frames } => {
                self.check_unlocked(id)?;
                if self.element(id).and_then(Element::as_group).is_none() {
                    return Err(ApplyError::NotGroup(id));
                }
                for (child, frame) in &frames {
                    check_frame(frame)?;
                    if *child != id && !self.ancestors(*child).contains(&id) {
                        return Err(ApplyError::InvalidParent(id));
                    }
                }
                let mut old = Vec::with_capacity(frames.len());
                for (child, frame) in frames {
                    let element = self.element_mut(child)?;
                    let frame = if element.is_line() {
                        normalize_line(frame)
                    } else {
                        frame
                    };
                    old.push((child, std::mem::replace(&mut element.frame, frame)));
                    self.fit(child)?;
                }
                Ok(Operation::SetGroupFrames { id, frames: old })
            }
            Operation::SetLayer { id, patch } => {
                if let Some(opacity) = patch.opacity {
                    check_opacity(opacity)?;
                }
                let element = self.element_mut(id)?;
                let old = patch.apply_to(element);
                Ok(Operation::SetLayer { id, patch: old })
            }
            Operation::SetTextSizing { id, sizing } => {
                self.check_unlocked(id)?;
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
                self.check_unlocked(id)?;
                let text = self.text_mut(id)?;
                let old = patch.apply_to(&mut text.style);
                self.fit(id)?;
                Ok(Operation::SetTextStyle { id, patch: old })
            }
            Operation::SetShapeStyle { id, patch } => {
                patch.validate()?;
                if let Some(fill) = &patch.fill {
                    self.check_fill(fill)?;
                }
                self.check_unlocked(id)?;
                let element = self.element_mut(id)?;
                let old = patch.apply_to(id, &mut element.kind)?;
                Ok(Operation::SetShapeStyle { id, patch: old })
            }
            Operation::ReplaceText { id, range, text } => {
                self.check_unlocked(id)?;
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
                let in_use = self.slides.iter().any(|slide| {
                    slide
                        .walk()
                        .iter()
                        .filter_map(|node| node.element.as_text())
                        .any(|text| text.style.font == face)
                });
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
            Operation::AddImage { id, data } => {
                if self.images.contains(id) {
                    return Err(ApplyError::DuplicateImage(id));
                }
                self.next_image_id = self.next_image_id.max(id.0 + 1);
                self.images.images.insert(id, data);
                Ok(Operation::RemoveImage { id })
            }
            Operation::RemoveImage { id } => {
                if self.image_in_use(id) {
                    return Err(ApplyError::ImageInUse(id));
                }
                let data = self
                    .images
                    .images
                    .remove(&id)
                    .ok_or(ApplyError::MissingImage(id))?;
                Ok(Operation::AddImage { id, data })
            }
            Operation::AddVideo { id, data } => {
                if self.videos.contains(id) {
                    return Err(ApplyError::DuplicateVideo(id));
                }
                self.next_video_id = self.next_video_id.max(id.0 + 1);
                self.videos.videos.insert(id, data);
                Ok(Operation::RemoveVideo { id })
            }
            Operation::RemoveVideo { id } => {
                if self.video_in_use(id) {
                    return Err(ApplyError::VideoInUse(id));
                }
                let data = self
                    .videos
                    .videos
                    .remove(&id)
                    .ok_or(ApplyError::MissingVideo(id))?;
                Ok(Operation::AddVideo { id, data })
            }
            Operation::Batch(operations) => {
                let mut inverses = Vec::with_capacity(operations.len());
                for operation in operations {
                    match self.apply_one(operation) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(error) => {
                            for inverse in inverses.into_iter().rev() {
                                self.apply_one(inverse)
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

    /// Moves a group, or resizes and turns it by mapping its descendants
    /// with [`map_frame`]. A resize or a turn is undone by restoring the
    /// frames of the group and its descendants: the text fitted after
    /// scaling cannot be scaled back exactly.
    fn set_group_frame(
        &mut self,
        id: ElementId,
        old: Frame,
        frame: Frame,
    ) -> Result<Operation, ApplyError> {
        // The box of a rotated group is computed, so a frame read back from
        // it can differ by rounding.
        if nearly(frame.width, old.width)
            && nearly(frame.height, old.height)
            && nearly(normalize_degrees(frame.rotation - old.rotation), 0.)
        {
            let element = self.element_mut(id)?;
            translate(element, frame.x - old.x, frame.y - old.y);
            return Ok(Operation::SetFrame { id, frame: old });
        }
        let mut nodes = Vec::new();
        let element = self.element(id).ok_or(ApplyError::UnknownElement(id))?;
        each_in_tree(element, &mut |node| {
            if node.id != id {
                let mode = MapMode::of(node);
                nodes.push((node.id, node.frame, mode, node.as_group().is_some()));
            }
        });
        let turn = frame.rotation - old.rotation;
        let mut restore = Vec::with_capacity(nodes.len() + 1);
        for (node, before, mode, group) in nodes {
            restore.push((node, before));
            let target = &mut self.element_mut(node)?.frame;
            if group {
                // Its box follows its children when the groups are fitted.
                target.rotation = normalize_degrees(before.rotation + turn);
            } else {
                *target = map_frame(&before, &old, &frame, mode);
                self.fit(node)?;
            }
        }
        restore.push((id, old));
        self.element_mut(id)?.frame.rotation = frame.rotation;
        Ok(Operation::SetGroupFrames {
            id,
            frames: restore,
        })
    }

    /// Checks that the image or the video of a fill is embedded.
    fn check_fill(&self, fill: &Fill) -> Result<(), ApplyError> {
        match fill {
            Fill::Image(image) if !self.images.contains(image.id) => {
                Err(ApplyError::MissingImage(image.id))
            }
            Fill::Shader(shader) => match shader.channel0 {
                Some(image) if !self.images.contains(image) => Err(ApplyError::MissingImage(image)),
                _ => Ok(()),
            },
            Fill::Video(video) if !self.videos.contains(video.id) => {
                Err(ApplyError::MissingVideo(video.id))
            }
            _ => Ok(()),
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

/// Brings the rotations of the element and its descendants into (-180, 180]
/// and the frames of lines to a height of 0.
fn normalize_frames(element: &mut Element) {
    element.frame.rotation = normalize_degrees(element.frame.rotation);
    if element.is_line() {
        element.frame = normalize_line(element.frame);
    }
    if let Some(group) = element.as_group_mut() {
        group.children.iter_mut().for_each(normalize_frames);
    }
}

/// The frame of a line given as a box: the horizontal center axis of the
/// box, with the same center.
fn normalize_line(frame: Frame) -> Frame {
    Frame {
        y: frame.y + frame.height / 2.,
        height: 0.,
        ..frame
    }
}

fn check_opacity(opacity: f32) -> Result<(), ApplyError> {
    if (0. ..=1.).contains(&opacity) {
        Ok(())
    } else {
        Err(ApplyError::InvalidOpacity)
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

pub use crate::images::{ImageData, ImageLibrary};
pub use crate::operation::{ApplyError, LayerPatch, Operation, ShapeStylePatch, TextStylePatch};
pub use crate::style::{
    Arrowhead, Dash, Fill, GradientStop, HeadKind, HeadSize, ImageFill, ImageFit, ImageId,
    LinearGradient, RadialGradient, ShaderFill, SolidFill, Start, Stroke, Vec2, VideoFill, VideoId,
};
pub use crate::videos::{VideoData, VideoLibrary};

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
                parent: None,
                index: usize::MAX,
                element: Element::new(id, frame, text(content, sizing)),
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
    fn a_copied_slide_gets_new_ids_for_all_its_elements() {
        let mut presentation = with_inter();
        let ids: Vec<ElementId> = ["A", "B", "C"]
            .into_iter()
            .map(|name| {
                add_text(
                    &mut presentation,
                    name,
                    TextSizing::AutoWidth,
                    Frame::default(),
                )
            })
            .collect();
        let group = presentation.new_element_id();
        let operations = presentation.group_operations(group, &ids[1..]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        let original = presentation.slides[0].id;

        let copy = presentation.copy_slide(original).unwrap();
        assert_ne!(copy.id, original);
        let old: Vec<ElementId> = presentation.slides[0]
            .walk()
            .iter()
            .map(|node| node.element.id)
            .collect();
        let new: Vec<ElementId> = copy.walk().iter().map(|node| node.element.id).collect();
        assert_eq!(new.len(), old.len());
        assert!(new.iter().all(|id| !old.contains(id)));
        let id = copy.id;
        presentation
            .apply(Operation::AddSlide {
                index: 1,
                slide: copy,
            })
            .unwrap();
        assert_eq!(presentation.slides[1].id, id);
        assert_eq!(
            presentation.slides[1].walk()[2]
                .element
                .as_text()
                .unwrap()
                .content,
            "B"
        );
        assert!(presentation.copy_slide(SlideId(99)).is_none());
    }

    fn move_block(presentation: &mut Presentation, moved: &[SlideId], to: usize) -> bool {
        match presentation.slide_moves(moved, to) {
            Some(operations) => {
                presentation.apply(Operation::Batch(operations)).unwrap();
                true
            }
            None => false,
        }
    }

    #[test]
    fn slide_moves_keep_the_block_together_in_order() {
        let mut presentation = Presentation::new();
        let s: Vec<SlideId> = std::iter::once(presentation.slides[0].id)
            .chain((0..4).map(|_| add_slide(&mut presentation)))
            .collect();
        // Down: 1 and 3 after 4.
        assert!(move_block(&mut presentation, &[s[1], s[3]], 5));
        assert_eq!(ids(&presentation), [1, 3, 5, 2, 4]);
        // Up: 2 and 4 to the start.
        assert!(move_block(&mut presentation, &[s[3], s[1]], 0));
        assert_eq!(ids(&presentation), [2, 4, 1, 3, 5]);
        // Into the middle.
        assert!(move_block(&mut presentation, &[s[0]], 1));
        assert_eq!(ids(&presentation), [2, 1, 4, 3, 5]);
        // Onto itself: no change.
        assert!(!move_block(&mut presentation, &[s[0], s[3]], 1));
        assert!(!move_block(&mut presentation, &[s[0], s[3]], 2));
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
        let element = Element::new(id, Frame::default(), text("Hi", TextSizing::AutoWidth));
        let add = Operation::AddElement {
            slide,
            parent: None,
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
    fn moving_an_auto_sized_box_keeps_its_size() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "Hello",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let fitted = presentation.element(id).unwrap().frame;
        let moved = Frame {
            x: 500.,
            y: 40.,
            ..fitted
        };
        let undo = presentation
            .apply(Operation::SetFrame { id, frame: moved })
            .unwrap();
        assert_eq!(presentation.element(id).unwrap().frame, moved);
        presentation.apply(undo).unwrap();
        assert_eq!(presentation.element(id).unwrap().frame, fitted);
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

    fn at(x: f32, y: f32, width: f32) -> Frame {
        Frame {
            x,
            y,
            width,
            ..Frame::default()
        }
    }

    /// Two wrapping texts grouped on the first slide: (group, first, second).
    fn grouped() -> (Presentation, ElementId, ElementId, ElementId) {
        let mut presentation = with_inter();
        let first = add_text(
            &mut presentation,
            "One",
            TextSizing::AutoHeight,
            at(100., 100., 200.),
        );
        let second = add_text(
            &mut presentation,
            "Two",
            TextSizing::AutoWidth,
            at(400., 300., 0.),
        );
        let group = presentation.new_element_id();
        let operations = presentation
            .group_operations(group, &[second, first])
            .unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        (presentation, group, first, second)
    }

    fn frame(presentation: &Presentation, id: ElementId) -> Frame {
        presentation.element(id).unwrap().frame
    }

    #[test]
    fn grouping_keeps_paint_order_and_fits_the_group_frame() {
        let (presentation, group, first, second) = grouped();
        let slide = &presentation.slides[0];
        assert_eq!(slide.elements.len(), 1);
        let children: Vec<_> = slide.elements[0].children().iter().map(|e| e.id).collect();
        assert_eq!(children, [first, second]);
        let bounds = frame(&presentation, group);
        let second_frame = frame(&presentation, second);
        assert_eq!((bounds.x, bounds.y), (100., 100.));
        assert_eq!(bounds.x + bounds.width, second_frame.x + second_frame.width);
        assert_eq!(
            presentation.locate(second),
            Some(Location {
                slide: slide.id,
                parent: Some(group),
                index: 1
            })
        );
        let nodes = slide.walk();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[2].depth, 1);
    }

    #[test]
    fn group_and_ungroup_undo_to_the_same_document() {
        let mut presentation = with_inter();
        let first = add_text(
            &mut presentation,
            "A",
            TextSizing::AutoWidth,
            at(0., 0., 0.),
        );
        let second = add_text(
            &mut presentation,
            "B",
            TextSizing::AutoWidth,
            at(50., 50., 0.),
        );
        let group = presentation.new_element_id();
        let operations = presentation
            .group_operations(group, &[first, second])
            .unwrap();
        round_trip(&mut presentation, Operation::Batch(operations.clone()));
        presentation.apply(Operation::Batch(operations)).unwrap();
        let operations = presentation.ungroup_operations(group).unwrap();
        round_trip(&mut presentation, Operation::Batch(operations.clone()));
        presentation.apply(Operation::Batch(operations)).unwrap();
        let ids: Vec<_> = presentation.slides[0]
            .elements
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [first, second]);
    }

    #[test]
    fn moving_a_group_moves_its_descendants() {
        let (mut presentation, group, first, second) = grouped();
        let outer = presentation.new_element_id();
        let operations = presentation.group_operations(outer, &[group]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        let before = frame(&presentation, second);
        let moved = Frame {
            x: frame(&presentation, outer).x + 10.,
            y: frame(&presentation, outer).y - 20.,
            ..frame(&presentation, outer)
        };
        round_trip(
            &mut presentation,
            Operation::SetFrame {
                id: outer,
                frame: moved,
            },
        );
        presentation
            .apply(Operation::SetFrame {
                id: outer,
                frame: moved,
            })
            .unwrap();
        let after = frame(&presentation, second);
        assert_eq!((after.x, after.y), (before.x + 10., before.y - 20.));
        assert_eq!(frame(&presentation, first).x, 110.);
        assert_eq!(frame(&presentation, group).x, 110.);
    }

    #[test]
    fn resizing_a_group_scales_boxes_but_not_fonts() {
        let (mut presentation, group, first, second) = grouped();
        let old = frame(&presentation, group);
        let size = |p: &Presentation, id| p.element(id).unwrap().as_text().unwrap().style.size;
        let resized = Frame {
            width: old.width * 2.,
            height: old.height * 2.,
            ..old
        };
        let second_before = frame(&presentation, second);
        round_trip(
            &mut presentation,
            Operation::SetFrame {
                id: group,
                frame: resized,
            },
        );
        presentation
            .apply(Operation::SetFrame {
                id: group,
                frame: resized,
            })
            .unwrap();
        // The wrapping text doubles its width; the auto-width one only moves.
        assert_eq!(frame(&presentation, first).width, 400.);
        let second_after = frame(&presentation, second);
        assert_eq!(second_after.width, second_before.width);
        assert_eq!(second_after.x, old.x + (second_before.x - old.x) * 2.);
        assert_eq!(size(&presentation, first), TextStyle::default().size);
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn close_frames(a: &Frame, b: &Frame) -> bool {
        close(a.x, b.x)
            && close(a.y, b.y)
            && close(a.width, b.width)
            && close(a.height, b.height)
            && close(a.rotation, b.rotation)
    }

    #[test]
    fn rotated_frames_map_points_both_ways() {
        let frame = Frame {
            x: 100.,
            y: 100.,
            width: 200.,
            height: 100.,
            rotation: 90.,
        };
        // Turned a quarter clockwise around its center (200, 150).
        let (x, y) = frame.to_slide(0., 0.);
        assert!(close(x, 250.) && close(y, 50.), "{x}, {y}");
        let (u, v) = frame.to_local(x, y);
        assert!(close(u, 0.) && close(v, 0.));
        assert!(frame.contains(200., 240.));
        assert!(!frame.contains(290., 150.));
        let bounds = frame.bounds();
        assert!(close_frames(
            &bounds,
            &Frame {
                x: 150.,
                y: 50.,
                width: 100.,
                height: 200.,
                rotation: 0.,
            }
        ));
        assert_eq!(normalize_degrees(270.), -90.);
        assert_eq!(normalize_degrees(-180.), 180.);
        assert_eq!(normalize_degrees(-360.), 0.);
    }

    #[test]
    fn rotated_frames_intersect_by_their_turned_shape() {
        let diamond = Frame {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
            rotation: 45.,
        };
        // The bounds reach the corner (0, 0); the diamond does not.
        let corner = Frame {
            x: -5.,
            y: -5.,
            width: 15.,
            height: 15.,
            rotation: 0.,
        };
        assert!(corner.intersects(&diamond.bounds()));
        assert!(!corner.intersects(&diamond));
        assert!(diamond.intersects(&Frame {
            x: 45.,
            y: 45.,
            ..corner
        }));
    }

    #[test]
    fn a_group_box_follows_the_axes_of_its_rotation() {
        let child = Frame {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 50.,
            rotation: 30.,
        };
        let box_ = union_in([&child], 30.).unwrap();
        assert!(close_frames(&box_, &child), "{box_:?}");
    }

    #[test]
    fn turning_a_group_turns_its_children_and_is_idempotent() {
        let (mut presentation, group, first, second) = grouped();
        let old = frame(&presentation, group);
        let turned = Frame {
            rotation: 90.,
            ..old
        };
        let set = Operation::SetFrame {
            id: group,
            frame: turned,
        };
        round_trip(&mut presentation, set.clone());
        presentation.apply(set.clone()).unwrap();
        let once = presentation.slides.clone();
        let box_ = frame(&presentation, group);
        assert!(close_frames(&box_, &turned), "{box_:?} {turned:?}");
        assert_eq!(frame(&presentation, first).rotation, 90.);
        assert_eq!(frame(&presentation, second).rotation, 90.);
        presentation.apply(set).unwrap();
        for (id, before) in [(first, &once), (second, &once)] {
            let before = find(&before[0].elements, id).unwrap().frame;
            assert!(
                close_frames(&frame(&presentation, id), &before),
                "the angle is final, not added"
            );
        }

        // Editing a child keeps the axes of the group.
        let child = frame(&presentation, first);
        presentation
            .apply(Operation::SetFrame {
                id: first,
                frame: Frame {
                    x: child.x + 10.,
                    ..child
                },
            })
            .unwrap();
        assert_eq!(frame(&presentation, group).rotation, 90.);
    }

    #[test]
    fn a_rotated_text_grows_from_its_rotated_top_left_corner() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "Hi",
            TextSizing::AutoWidth,
            Frame {
                x: 100.,
                y: 100.,
                rotation: 30.,
                ..Frame::default()
            },
        );
        let before = frame(&presentation, id);
        let corner = before.to_slide(0., 0.);
        presentation
            .apply(Operation::ReplaceText {
                id,
                range: 2..2,
                text: " there, a longer line".into(),
            })
            .unwrap();
        let after = frame(&presentation, id);
        assert!(after.width > before.width);
        let moved = after.to_slide(0., 0.);
        assert!(close(moved.0, corner.0) && close(moved.1, corner.1));
    }

    #[test]
    fn mapping_a_child_turned_a_quarter_swaps_its_scale() {
        let from = Frame {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
            rotation: 0.,
        };
        let to = Frame {
            width: 200.,
            ..from
        };
        let child = Frame {
            x: 25.,
            y: 40.,
            width: 50.,
            height: 20.,
            rotation: 90.,
        };
        let mapped = map_frame(&child, &from, &to, MapMode::Box);
        // Its height lies along the group's width.
        assert!(close(mapped.width, 50.) && close(mapped.height, 40.));
    }

    #[test]
    fn an_element_cannot_move_into_itself_or_a_descendant() {
        let (mut presentation, group, first, _) = grouped();
        let outer = presentation.new_element_id();
        let operations = presentation.group_operations(outer, &[group]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        for parent in [outer, group] {
            assert_eq!(
                presentation.apply(Operation::MoveElement {
                    id: outer,
                    parent: Some(parent),
                    index: 0,
                }),
                Err(ApplyError::InvalidParent(parent))
            );
        }
        assert_eq!(
            presentation.apply(Operation::MoveElement {
                id: group,
                parent: Some(first),
                index: 0,
            }),
            Err(ApplyError::NotGroup(first))
        );
        round_trip(
            &mut presentation,
            Operation::MoveElement {
                id: first,
                parent: None,
                index: 0,
            },
        );
    }

    #[test]
    fn locked_elements_accept_only_layer_changes() {
        let (mut presentation, group, first, _) = grouped();
        let lock = |locked| Operation::SetLayer {
            id: group,
            patch: LayerPatch {
                locked: Some(locked),
                ..LayerPatch::default()
            },
        };
        let undo_lock = presentation.apply(lock(true)).unwrap();
        assert!(presentation.is_locked(first));
        let edits = [
            Operation::RemoveElement { id: first },
            Operation::ReplaceText {
                id: first,
                range: 0..0,
                text: "x".into(),
            },
            Operation::SetFrame {
                id: first,
                frame: at(0., 0., 10.),
            },
            Operation::MoveElement {
                id: first,
                parent: None,
                index: 0,
            },
        ];
        for edit in edits {
            assert_eq!(
                presentation.apply(edit.clone()),
                Err(ApplyError::Locked(first)),
                "{edit:?}"
            );
        }
        let rename = Operation::SetLayer {
            id: first,
            patch: LayerPatch {
                name: Some(Some("Title".into())),
                ..LayerPatch::default()
            },
        };
        round_trip(&mut presentation, rename);
        // Undoing the lock unlocks: the history stays usable.
        presentation.apply(undo_lock).unwrap();
        assert!(!presentation.is_locked(first));
    }

    #[test]
    fn an_unlocked_ancestor_moves_a_locked_child() {
        let (mut presentation, group, first, _) = grouped();
        presentation
            .apply(Operation::SetLayer {
                id: first,
                patch: LayerPatch {
                    locked: Some(true),
                    ..LayerPatch::default()
                },
            })
            .unwrap();
        let old = frame(&presentation, group);
        let resized = Frame {
            width: old.width + 100.,
            ..old
        };
        round_trip(
            &mut presentation,
            Operation::SetFrame {
                id: group,
                frame: resized,
            },
        );
    }

    #[test]
    fn hidden_texts_are_not_drawn_and_fonts_in_groups_stay_in_use() {
        let (mut presentation, group, _, _) = grouped();
        let face = TextStyle::default().font;
        assert_eq!(
            presentation.apply(Operation::RemoveFont { face: face.clone() }),
            Err(ApplyError::FontInUse(face))
        );
        assert_eq!(presentation.slides[0].visible_texts().count(), 2);
        presentation
            .apply(Operation::SetLayer {
                id: group,
                patch: LayerPatch {
                    hidden: Some(true),
                    ..LayerPatch::default()
                },
            })
            .unwrap();
        assert_eq!(presentation.slides[0].visible_texts().count(), 0);
    }

    #[test]
    fn groups_read_and_write_json() {
        let (presentation, group, _, _) = grouped();
        let element = presentation.element(group).unwrap();
        let json = serde_json::to_value(element).unwrap();
        assert!(json["group"]["children"].is_array());
        assert!(
            json.get("hidden").is_none(),
            "default layer fields are left out"
        );
        let back: Element = serde_json::from_value(json).unwrap();
        assert_eq!(&back, element);
        let patch: LayerPatch = serde_json::from_str(r#"{"name": null}"#).unwrap();
        assert_eq!(patch.name, Some(None));
    }

    /// Adds a shape to the first slide and returns its id.
    pub fn add_shape(
        presentation: &mut Presentation,
        frame: Frame,
        kind: ElementKind,
    ) -> ElementId {
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        presentation
            .apply(Operation::AddElement {
                slide,
                parent: None,
                index: usize::MAX,
                element: Element::new(id, frame, kind),
            })
            .unwrap();
        id
    }

    /// Changes the style of a shape.
    pub fn set_style(presentation: &mut Presentation, id: ElementId, patch: ShapeStylePatch) {
        presentation
            .apply(Operation::SetShapeStyle { id, patch })
            .unwrap();
    }

    fn line() -> ElementKind {
        ElementKind::Line(LineElement::default())
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn shapes_read_and_write_json() {
        let json = r#"{"id": 3, "frame": {"x": 1, "y": 2, "width": 30, "height": 40},
            "opacity": 0.5,
            "rectangle": {"corner_radius": 8, "stroke": {"width": 2, "dash": "dotted"},
              "fill": {"linear_gradient": {"angle": 90, "stops": [
                {"position": 0, "color": "FFFFFF"}, {"position": 1, "color": "000000"}]}}}}"#;
        let element: Element = serde_json::from_str(json).unwrap();
        let ElementKind::Rectangle(shape) = &element.kind else {
            panic!("a rectangle");
        };
        assert_eq!(shape.corner_radius, 8.);
        assert_eq!(shape.stroke.unwrap().dash, Dash::Dotted);
        assert_eq!(shape.stroke.unwrap().color, Stroke::default().color);
        assert_eq!(element.opacity, 0.5);
        let back: Element =
            serde_json::from_value(serde_json::to_value(&element).unwrap()).unwrap();
        assert_eq!(back, element);

        let ellipse: Element = serde_json::from_str(r#"{"id": 4, "ellipse": {}}"#).unwrap();
        assert_eq!(ellipse.kind.fill(), Some(&Fill::default()));
        assert_eq!(ellipse.kind.stroke(), None);
        let json = serde_json::to_value(&ellipse).unwrap();
        assert!(json.get("opacity").is_none(), "an opacity of 1 is left out");
        let line: Element =
            serde_json::from_str(r#"{"id": 5, "line": {"end": "triangle"}}"#).unwrap();
        let ElementKind::Line(line) = line.kind else {
            panic!("a line");
        };
        assert_eq!(line.end.kind, HeadKind::Triangle);
        assert!(line.start.is_none());
    }

    #[test]
    fn line_frames_keep_their_center_with_no_height() {
        let mut presentation = Presentation::new();
        let id = add_shape(
            &mut presentation,
            Frame {
                x: 100.,
                y: 100.,
                width: 200.,
                height: 50.,
                rotation: 0.,
            },
            line(),
        );
        let frame = frame(&presentation, id);
        assert_eq!((frame.y, frame.height), (125., 0.));
        presentation
            .apply(Operation::SetFrame {
                id,
                frame: Frame {
                    height: 20.,
                    ..frame
                },
            })
            .unwrap();
        let moved = presentation.element(id).unwrap().frame;
        assert_eq!((moved.y, moved.height), (135., 0.));
    }

    #[test]
    fn lines_are_placed_by_their_ends() {
        let frame = Frame::from_line((0., 0.), (100., 100.), 0.);
        assert_eq!(frame.rotation, 45.);
        let (start, end) = frame.line_ends();
        assert!(near(start, (0., 0.)) && near(end, (100., 100.)));
        let vertical = Frame::from_line((10., 50.), (10., 0.), 0.);
        assert_eq!(vertical.rotation, -90.);
        assert_eq!(Frame::from_line((5., 5.), (5., 5.), 30.).rotation, 30.);
    }

    #[test]
    fn a_line_in_a_stretched_group_keeps_its_ends_on_the_group() {
        let mut presentation = Presentation::new();
        let id = add_shape(
            &mut presentation,
            Frame::from_line((0., 0.), (100., 100.), 0.),
            line(),
        );
        let other = add_shape(
            &mut presentation,
            Frame {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
                rotation: 0.,
            },
            ElementKind::Rectangle(RectangleElement::default()),
        );
        let group = presentation.new_element_id();
        let operations = presentation.group_operations(group, &[id, other]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        let from = presentation.element(group).unwrap().frame;
        assert_eq!((from.width, from.height), (100., 100.));
        presentation
            .apply(Operation::SetFrame {
                id: group,
                frame: Frame {
                    width: 200.,
                    ..from
                },
            })
            .unwrap();
        let (start, end) = presentation.element(id).unwrap().frame.line_ends();
        assert!(near(start, (0., 0.)), "{start:?}");
        assert!(near(end, (200., 100.)), "{end:?}");
    }

    #[test]
    fn a_group_of_one_horizontal_line_has_no_height() {
        let mut presentation = Presentation::new();
        let id = add_shape(
            &mut presentation,
            Frame::from_line((0., 50.), (100., 50.), 0.),
            line(),
        );
        let group = presentation.new_element_id();
        let operations = presentation.group_operations(group, &[id]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        let from = presentation.element(group).unwrap().frame;
        assert_eq!(from.height, 0.);
        presentation
            .apply(Operation::SetFrame {
                id: group,
                frame: Frame {
                    width: 50.,
                    height: 30.,
                    ..from
                },
            })
            .unwrap();
        let (start, end) = presentation.element(id).unwrap().frame.line_ends();
        assert!(near(start, (0., 50.)) && near(end, (50., 50.)), "{end:?}");
    }

    #[test]
    fn shape_styles_undo_and_reject_fields_the_shape_lacks() {
        let mut presentation = Presentation::new();
        let rectangle = add_shape(
            &mut presentation,
            Frame::default(),
            ElementKind::Rectangle(RectangleElement::default()),
        );
        let line = add_shape(&mut presentation, Frame::default(), line());
        let before = presentation.clone();
        let patch = ShapeStylePatch {
            fill: Some(Fill::None),
            stroke: Some(Some(Stroke::default())),
            corner_radius: Some(12.),
            ..ShapeStylePatch::default()
        };
        assert_eq!(patch.label(), "Shape style");
        let inverse = presentation
            .apply(Operation::SetShapeStyle {
                id: rectangle,
                patch,
            })
            .unwrap();
        let kind = &presentation.element(rectangle).unwrap().kind;
        assert_eq!(kind.fill(), Some(&Fill::None));
        assert!(kind.stroke().is_some());
        presentation.apply(inverse).unwrap();
        assert_eq!(presentation, before);

        let fill = ShapeStylePatch {
            fill: Some(Fill::None),
            end: Some(Arrowhead::new(HeadKind::Arrow, HeadSize::Large)),
            ..ShapeStylePatch::default()
        };
        assert_eq!(
            presentation.apply(Operation::SetShapeStyle {
                id: line,
                patch: fill
            }),
            Err(ApplyError::NotApplicable {
                id: line,
                field: "fill"
            })
        );
        let remove = ShapeStylePatch {
            stroke: Some(None),
            ..ShapeStylePatch::default()
        };
        assert_eq!(
            presentation.apply(Operation::SetShapeStyle {
                id: line,
                patch: remove
            }),
            Err(ApplyError::StrokeRequired(line))
        );
        let radius = ShapeStylePatch {
            corner_radius: Some(-1.),
            ..ShapeStylePatch::default()
        };
        assert_eq!(
            presentation.apply(Operation::SetShapeStyle {
                id: rectangle,
                patch: radius
            }),
            Err(ApplyError::InvalidStyle)
        );
        assert_eq!(presentation, before);
    }

    #[test]
    fn shapes_check_their_style_and_their_lock() {
        let mut presentation = Presentation::new();
        let slide = presentation.slides[0].id;
        let bad = ElementKind::Ellipse(EllipseElement {
            fill: Fill::Solid(SolidFill {
                color: Rgb(0),
                opacity: 2.,
            }),
            stroke: None,
        });
        let id = presentation.new_element_id();
        assert_eq!(
            presentation.apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element: Element::new(id, Frame::default(), bad),
            }),
            Err(ApplyError::InvalidStyle)
        );
        let id = add_shape(
            &mut presentation,
            Frame::default(),
            ElementKind::Ellipse(EllipseElement::default()),
        );
        presentation
            .apply(Operation::SetLayer {
                id,
                patch: LayerPatch {
                    locked: Some(true),
                    ..LayerPatch::default()
                },
            })
            .unwrap();
        assert_eq!(
            presentation.apply(Operation::SetShapeStyle {
                id,
                patch: ShapeStylePatch {
                    fill: Some(Fill::None),
                    ..ShapeStylePatch::default()
                },
            }),
            Err(ApplyError::Locked(id))
        );
    }

    #[test]
    fn opacity_is_a_layer_field_that_groups_multiply() {
        let (mut presentation, group, first, _) = grouped();
        for (id, opacity) in [(group, 0.5), (first, 0.5)] {
            presentation
                .apply(Operation::SetLayer {
                    id,
                    patch: LayerPatch {
                        opacity: Some(opacity),
                        locked: Some(true),
                        ..LayerPatch::default()
                    },
                })
                .unwrap();
        }
        let node = presentation.slides[0]
            .walk()
            .into_iter()
            .find(|node| node.element.id == first)
            .unwrap();
        assert_eq!(node.opacity, 0.25);
        let patch = LayerPatch {
            opacity: Some(0.8),
            ..LayerPatch::default()
        };
        assert_eq!(patch.label(), "Opacity");
        let inverse = presentation
            .apply(Operation::SetLayer { id: first, patch })
            .expect("a locked element accepts a new opacity");
        presentation.apply(inverse).unwrap();
        assert_eq!(presentation.element(first).unwrap().opacity, 0.5);
        assert_eq!(
            presentation.apply(Operation::SetLayer {
                id: first,
                patch: LayerPatch {
                    opacity: Some(1.5),
                    ..LayerPatch::default()
                },
            }),
            Err(ApplyError::InvalidOpacity)
        );
    }

    #[test]
    fn images_embed_undo_and_stay_while_a_fill_uses_them() {
        let mut presentation = Presentation::new();
        let data = ImageData::read(crate::images::tests::png(8, 4)).unwrap();
        let image = presentation.new_image_id();
        let fill = Fill::Image(ImageFill {
            id: image,
            fit: ImageFit::Cover,
            opacity: 1.,
        });
        let rectangle = || {
            ElementKind::Rectangle(RectangleElement {
                fill: fill.clone(),
                ..RectangleElement::default()
            })
        };
        let slide = presentation.slides[0].id;
        let id = presentation.new_element_id();
        assert_eq!(
            presentation.apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element: Element::new(id, Frame::default(), rectangle()),
            }),
            Err(ApplyError::MissingImage(image))
        );
        let before = presentation.clone();
        let inverse = presentation
            .apply(Operation::AddImage {
                id: image,
                data: data.clone(),
            })
            .unwrap();
        assert_eq!(
            presentation.apply(Operation::AddImage { id: image, data }),
            Err(ApplyError::DuplicateImage(image))
        );
        add_shape(&mut presentation, Frame::default(), rectangle());
        assert!(presentation.image_in_use(image));
        assert_eq!(
            presentation.apply(Operation::RemoveImage { id: image }),
            Err(ApplyError::ImageInUse(image))
        );
        let mut unused = before.clone();
        let added = unused
            .apply(Operation::AddImage {
                id: image,
                data: presentation.images.get(image).unwrap().clone(),
            })
            .unwrap();
        assert_eq!(added, inverse);
        unused.apply(inverse).unwrap();
        assert_eq!(unused, before);
    }

    #[test]
    fn videos_embed_undo_and_stay_while_a_fill_uses_them() {
        if !crate::videos::tests::plugins_or_skip() {
            return;
        }
        let mut presentation = Presentation::new();
        let data = VideoData::read(crate::videos::tests::mp4(320, 180)).unwrap();
        let video = presentation.new_video_id();
        let rectangle = || {
            ElementKind::Rectangle(RectangleElement {
                fill: Fill::Video(VideoFill::new(video)),
                ..RectangleElement::default()
            })
        };
        let slide = presentation.slides[0].id;
        let id = presentation.new_element_id();
        assert_eq!(
            presentation.apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element: Element::new(id, Frame::default(), rectangle()),
            }),
            Err(ApplyError::MissingVideo(video))
        );
        let before = presentation.clone();
        let inverse = presentation
            .apply(Operation::AddVideo {
                id: video,
                data: data.clone(),
            })
            .unwrap();
        assert_eq!(
            presentation.apply(Operation::AddVideo {
                id: video,
                data: data.clone()
            }),
            Err(ApplyError::DuplicateVideo(video))
        );
        add_shape(&mut presentation, Frame::default(), rectangle());
        assert!(presentation.video_in_use(video));
        assert_eq!(
            presentation.apply(Operation::RemoveVideo { id: video }),
            Err(ApplyError::VideoInUse(video))
        );
        let mut unused = before.clone();
        let added = unused
            .apply(Operation::AddVideo { id: video, data })
            .unwrap();
        assert_eq!(added, inverse);
        unused.apply(inverse).unwrap();
        assert_eq!(unused, before);
    }

    #[test]
    fn a_shader_keeps_the_image_of_its_channel() {
        let mut presentation = Presentation::new();
        let data = ImageData::read(crate::images::tests::png(8, 4)).unwrap();
        let image = presentation.new_image_id();
        let shader = |channel0| {
            ElementKind::Ellipse(EllipseElement {
                fill: Fill::Shader(ShaderFill {
                    channel0,
                    ..ShaderFill::default()
                }),
                stroke: None,
            })
        };
        let slide = presentation.slides[0].id;
        let id = presentation.new_element_id();
        assert_eq!(
            presentation.apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element: Element::new(id, Frame::default(), shader(Some(image))),
            }),
            Err(ApplyError::MissingImage(image))
        );
        presentation
            .apply(Operation::AddImage { id: image, data })
            .unwrap();
        add_shape(&mut presentation, Frame::default(), shader(Some(image)));
        assert_eq!(
            presentation.apply(Operation::RemoveImage { id: image }),
            Err(ApplyError::ImageInUse(image))
        );
        add_shape(&mut presentation, Frame::default(), shader(None));
    }
}
