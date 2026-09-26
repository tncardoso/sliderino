//! Paint of the shapes: fills, strokes and arrowheads.
//!
//! Only what PDF, PPTX and HTML all draw the same way is offered: solid
//! colors, linear and radial gradients, embedded images, centered strokes
//! with three dash presets and five arrowhead kinds.

use serde::{Deserialize, Serialize};

use crate::document::{ApplyError, Rgb};

/// A pair of numbers, such as a point in slide units or a fraction of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn half() -> Self {
        Self::new(0.5, 0.5)
    }
}

fn one() -> f32 {
    1.
}

fn is_one(value: &f32) -> bool {
    *value == 1.
}

fn unit(value: f32) -> bool {
    (0. ..=1.).contains(&value)
}

/// The outline of a shape, centered on its edge. In JSON every field is
/// optional and defaults to [`Stroke::default`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stroke {
    pub color: Rgb,
    /// 0.0 to 1.0.
    pub opacity: f32,
    /// In slide units; more than 0.
    pub width: f32,
    pub dash: Dash,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            color: Rgb(0x111111),
            opacity: 1.,
            width: 4.,
            dash: Dash::Solid,
        }
    }
}

impl Stroke {
    pub fn validate(&self) -> Result<(), ApplyError> {
        if self.width.is_finite() && self.width > 0. && unit(self.opacity) {
            Ok(())
        } else {
            Err(ApplyError::InvalidStyle)
        }
    }
}

/// Dash presets. Dashes are 4 stroke widths long with gaps of 3; dots are
/// one stroke width long with gaps of one. Ends are cut flat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl Dash {
    pub const ALL: [Dash; 3] = [Dash::Solid, Dash::Dashed, Dash::Dotted];

    pub fn label(self) -> &'static str {
        match self {
            Dash::Solid => "Solid",
            Dash::Dashed => "Dashed",
            Dash::Dotted => "Dotted",
        }
    }
}

/// The shape drawn at one end of a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadKind {
    #[default]
    None,
    /// A filled triangle.
    Triangle,
    /// An open V made of two strokes.
    Arrow,
    Diamond,
    Circle,
}

impl HeadKind {
    pub const ALL: [HeadKind; 5] = [
        HeadKind::None,
        HeadKind::Triangle,
        HeadKind::Arrow,
        HeadKind::Diamond,
        HeadKind::Circle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HeadKind::None => "None",
            HeadKind::Triangle => "Triangle",
            HeadKind::Arrow => "Arrow",
            HeadKind::Diamond => "Diamond",
            HeadKind::Circle => "Circle",
        }
    }
}

/// Length and width of an arrowhead, as a multiple of the stroke width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl HeadSize {
    pub const ALL: [HeadSize; 3] = [HeadSize::Small, HeadSize::Medium, HeadSize::Large];

    pub fn factor(self) -> f32 {
        match self {
            HeadSize::Small => 2.,
            HeadSize::Medium => 3.,
            HeadSize::Large => 5.,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HeadSize::Small => "Small",
            HeadSize::Medium => "Medium",
            HeadSize::Large => "Large",
        }
    }
}

/// An end of a line. In JSON, a kind (`"triangle"`, medium size) or
/// `{"kind": "triangle", "size": "large"}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Arrowhead {
    pub kind: HeadKind,
    pub size: HeadSize,
}

impl Arrowhead {
    pub const NONE: Arrowhead = Arrowhead {
        kind: HeadKind::None,
        size: HeadSize::Medium,
    };

    pub fn new(kind: HeadKind, size: HeadSize) -> Self {
        Self { kind, size }
    }

    pub fn is_none(&self) -> bool {
        self.kind == HeadKind::None
    }
}

impl Serialize for Arrowhead {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Full {
            kind: HeadKind,
            size: HeadSize,
        }
        if self.size == HeadSize::Medium {
            self.kind.serialize(serializer)
        } else {
            Full {
                kind: self.kind,
                size: self.size,
            }
            .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Arrowhead {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            #[serde(default)]
            kind: HeadKind,
            #[serde(default)]
            size: HeadSize,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Kind(HeadKind),
            Full(Full),
        }
        match Raw::deserialize(deserializer).map_err(|_| {
            serde::de::Error::custom(
                "expected an arrowhead: \"none\", \"triangle\", \"arrow\", \"diamond\", \
                 \"circle\" or {\"kind\": .., \"size\": \"small\" | \"medium\" | \"large\"}",
            )
        })? {
            Raw::Kind(kind) => Ok(Arrowhead::new(kind, HeadSize::Medium)),
            Raw::Full(full) => Ok(Arrowhead::new(full.kind, full.size)),
        }
    }
}

/// The inside of a rectangle or an ellipse. In JSON, `"none"` or an object
/// with one key: `{"solid": {..}}`, `{"linear_gradient": {..}}`,
/// `{"radial_gradient": {..}}` or `{"image": {..}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    None,
    Solid(SolidFill),
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
    Image(ImageFill),
}

impl Default for Fill {
    /// The light gray of a new shape.
    fn default() -> Self {
        Fill::Solid(SolidFill {
            color: Rgb(0xD9D9D9),
            opacity: 1.,
        })
    }
}

impl Fill {
    pub fn is_none(&self) -> bool {
        matches!(self, Fill::None)
    }

    /// The name of the fill type, as the inspector shows it.
    pub fn label(&self) -> &'static str {
        match self {
            Fill::None => "None",
            Fill::Solid(_) => "Solid",
            Fill::LinearGradient(_) => "Linear",
            Fill::RadialGradient(_) => "Radial",
            Fill::Image(_) => "Image",
        }
    }

    pub fn stops(&self) -> Option<&[GradientStop]> {
        match self {
            Fill::LinearGradient(gradient) => Some(&gradient.stops),
            Fill::RadialGradient(gradient) => Some(&gradient.stops),
            _ => None,
        }
    }

    pub fn stops_mut(&mut self) -> Option<&mut Vec<GradientStop>> {
        match self {
            Fill::LinearGradient(gradient) => Some(&mut gradient.stops),
            Fill::RadialGradient(gradient) => Some(&mut gradient.stops),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), ApplyError> {
        let valid = match self {
            Fill::None => true,
            Fill::Solid(solid) => unit(solid.opacity),
            Fill::LinearGradient(gradient) => {
                gradient.angle.is_finite() && valid_stops(&gradient.stops)
            }
            Fill::RadialGradient(gradient) => {
                let finite = [
                    gradient.center.x,
                    gradient.center.y,
                    gradient.radius.x,
                    gradient.radius.y,
                ]
                .iter()
                .all(|value| value.is_finite());
                finite
                    && gradient.radius.x > 0.
                    && gradient.radius.y > 0.
                    && valid_stops(&gradient.stops)
            }
            Fill::Image(image) => unit(image.opacity),
        };
        if valid {
            Ok(())
        } else {
            Err(ApplyError::InvalidStyle)
        }
    }
}

/// Most stops a gradient holds, as in PowerPoint.
pub const MAX_STOPS: usize = 10;

fn valid_stops(stops: &[GradientStop]) -> bool {
    (2..=MAX_STOPS).contains(&stops.len())
        && stops
            .iter()
            .all(|stop| unit(stop.position) && unit(stop.opacity))
        && stops
            .windows(2)
            .all(|pair| pair[0].position <= pair[1].position)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidFill {
    pub color: Rgb,
    /// 0.0 to 1.0.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
}

/// A color at a point of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    /// 0.0 (start) to 1.0 (end); stops are in increasing order.
    pub position: f32,
    pub color: Rgb,
    /// 0.0 to 1.0.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
}

impl GradientStop {
    pub fn new(position: f32, color: Rgb) -> Self {
        Self {
            position,
            color,
            opacity: 1.,
        }
    }
}

/// Colors that change along a line through the center of the shape's
/// unrotated box: the gradient turns with the shape. The line is long
/// enough for the first and last colors to reach the corners, as in CSS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearGradient {
    /// Direction in degrees, clockwise: 0 goes from left to right, 90 from
    /// top to bottom.
    #[serde(default)]
    pub angle: f32,
    /// 2 to 10 stops.
    pub stops: Vec<GradientStop>,
}

/// Colors that change from a center outward, along an ellipse. Center and
/// radii are fractions of the shape's box, so the gradient follows a
/// resize: the default radii of 0.5 touch the box edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadialGradient {
    #[serde(default = "Vec2::half")]
    pub center: Vec2,
    #[serde(default = "Vec2::half")]
    pub radius: Vec2,
    /// 2 to 10 stops.
    pub stops: Vec<GradientStop>,
}

/// Stable identity of an image embedded in the presentation. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImageId(pub u64);

/// An embedded image painted inside the shape.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageFill {
    pub id: ImageId,
    #[serde(default)]
    pub fit: ImageFit,
    /// 0.0 to 1.0.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
}

/// How an image fills the box of its shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Covers the box, keeps its proportions and crops what is outside.
    #[default]
    Cover,
    /// Fits inside the box and keeps its proportions; the rest stays empty.
    Contain,
    /// Fills the box exactly; the proportions change.
    Stretch,
}

impl ImageFit {
    pub const ALL: [ImageFit; 3] = [ImageFit::Cover, ImageFit::Contain, ImageFit::Stretch];

    pub fn label(self) -> &'static str {
        match self {
            ImageFit::Cover => "Cover",
            ImageFit::Contain => "Contain",
            ImageFit::Stretch => "Stretch",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrowheads_read_a_kind_or_an_object() {
        let short: Arrowhead = serde_json::from_str("\"triangle\"").unwrap();
        assert_eq!(short, Arrowhead::new(HeadKind::Triangle, HeadSize::Medium));
        let full: Arrowhead =
            serde_json::from_str(r#"{"kind": "circle", "size": "large"}"#).unwrap();
        assert_eq!(full, Arrowhead::new(HeadKind::Circle, HeadSize::Large));
        assert_eq!(serde_json::to_string(&short).unwrap(), "\"triangle\"");
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"kind":"circle","size":"large"}"#
        );
        assert!(serde_json::from_str::<Arrowhead>("\"star\"").is_err());
    }

    #[test]
    fn fills_read_none_or_one_key() {
        let none: Fill = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(none, Fill::None);
        let radial: Fill = serde_json::from_str(
            r#"{"radial_gradient": {"stops": [
                {"position": 0, "color": "FFFFFF"}, {"position": 1, "color": "000000"}]}}"#,
        )
        .unwrap();
        let Fill::RadialGradient(radial) = radial else {
            panic!("a radial gradient");
        };
        assert_eq!(radial.center, Vec2::new(0.5, 0.5));
        assert_eq!(radial.radius, Vec2::new(0.5, 0.5));
    }

    #[test]
    fn gradients_need_two_to_ten_ordered_stops() {
        let gradient = |positions: &[f32]| {
            Fill::LinearGradient(LinearGradient {
                angle: 0.,
                stops: positions
                    .iter()
                    .map(|position| GradientStop::new(*position, Rgb(0)))
                    .collect(),
            })
        };
        assert!(gradient(&[0., 1.]).validate().is_ok());
        assert!(gradient(&[0.]).validate().is_err());
        assert!(gradient(&[0.5, 0.2]).validate().is_err());
        assert!(gradient(&[0., 1.5]).validate().is_err());
        assert!(gradient(&[0.; 11]).validate().is_err());
    }

    #[test]
    fn strokes_need_a_positive_width() {
        let stroke = |width: f32| Stroke {
            width,
            ..Stroke::default()
        };
        assert!(stroke(1.).validate().is_ok());
        assert!(stroke(0.).validate().is_err());
        assert!(stroke(f32::NAN).validate().is_err());
    }
}
