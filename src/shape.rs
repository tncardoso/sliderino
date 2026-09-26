//! Geometry of the shapes, shared by the CPU renderer, the canvas and the
//! exports so that they all draw the same outlines, arrowheads, dashes and
//! gradients. Everything is in the local units of the frame: (0, 0) is the
//! top-left corner of the unrotated frame. A line goes from (0, 0) to
//! (length, 0), since its frame has no height.

use crate::document::{
    Arrowhead, Dash, Element, ElementKind, Frame, HeadKind, ImageFit, LineElement, Vec2,
};

pub type P = (f32, f32);

/// One step of an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(P),
    Line(P),
    /// Two control points, then the end point.
    Cubic(P, P, P),
    Close,
}

/// Distance of the control points of a cubic quarter circle, as a fraction
/// of the radius.
const KAPPA: f32 = 0.552_284_8;

/// The outline of a rectangle, with corners rounded by `radius` (limited to
/// half the shorter side). Clockwise from the top-left corner.
pub fn rectangle(width: f32, height: f32, radius: f32) -> Vec<Seg> {
    let r = radius.clamp(0., width.min(height) / 2.);
    if r <= 0. {
        return vec![
            Seg::Move((0., 0.)),
            Seg::Line((width, 0.)),
            Seg::Line((width, height)),
            Seg::Line((0., height)),
            Seg::Close,
        ];
    }
    let k = r * (1. - KAPPA);
    vec![
        Seg::Move((r, 0.)),
        Seg::Line((width - r, 0.)),
        Seg::Cubic((width - k, 0.), (width, k), (width, r)),
        Seg::Line((width, height - r)),
        Seg::Cubic(
            (width, height - k),
            (width - k, height),
            (width - r, height),
        ),
        Seg::Line((r, height)),
        Seg::Cubic((k, height), (0., height - k), (0., height - r)),
        Seg::Line((0., r)),
        Seg::Cubic((0., k), (k, 0.), (r, 0.)),
        Seg::Close,
    ]
}

/// The outline of the ellipse inside a box, as four cubic curves, clockwise
/// from the rightmost point.
pub fn ellipse(width: f32, height: f32) -> Vec<Seg> {
    ellipse_at((width / 2., height / 2.), width / 2., height / 2.)
}

fn ellipse_at((cx, cy): P, rx: f32, ry: f32) -> Vec<Seg> {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    vec![
        Seg::Move((cx + rx, cy)),
        Seg::Cubic((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry)),
        Seg::Cubic((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy)),
        Seg::Cubic((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry)),
        Seg::Cubic((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy)),
        Seg::Close,
    ]
}

/// The closed outline of a rectangle or an ellipse; `None` for other kinds.
pub fn outline(kind: &ElementKind, width: f32, height: f32) -> Option<Vec<Seg>> {
    match kind {
        ElementKind::Rectangle(shape) => Some(rectangle(width, height, shape.corner_radius)),
        ElementKind::Ellipse(_) => Some(ellipse(width, height)),
        _ => None,
    }
}

/// The dash and gap lengths of a dash preset; `None` for a solid stroke.
pub fn dash_array(dash: Dash, width: f32) -> Option<[f32; 2]> {
    match dash {
        Dash::Solid => None,
        Dash::Dashed => Some([4. * width, 3. * width]),
        Dash::Dotted => Some([width, width]),
    }
}

/// An arrowhead ready to draw in the stroke color.
#[derive(Clone, Debug, PartialEq)]
pub enum Head {
    /// A closed outline to fill.
    Filled(Vec<Seg>),
    /// An open polyline to stroke with the width of the line, with miter
    /// joins.
    Open([P; 3]),
}

/// What a line draws: the stroked part of the line and its arrowheads.
#[derive(Clone, Debug, PartialEq)]
pub struct LineGeometry {
    /// The part of the line to stroke; `None` when the heads cover it all.
    pub segment: Option<(P, P)>,
    pub heads: Vec<Head>,
}

/// The line from (0, 0) to (`length`, 0) and its heads. A filled head ends
/// the stroke at its back edge, so that the line and the head do not
/// overlap: with an opacity below 1, the overlap would be darker.
pub fn line_geometry(length: f32, line: &LineElement) -> LineGeometry {
    let width = line.stroke.width;
    let mut from = 0.;
    let mut to = length;
    let mut heads = Vec::new();
    for (head, tip, direction) in [(line.start, 0., -1.), (line.end, length, 1.)] {
        let Some((shape, trim)) = arrowhead(head, tip, direction, width) else {
            continue;
        };
        heads.push(shape);
        if direction < 0. {
            from = trim;
        } else {
            to = trim;
        }
    }
    LineGeometry {
        segment: (to - from > 1e-3).then_some(((from, 0.), (to, 0.))),
        heads,
    }
}

/// The head at `tip` on the x axis, pointing toward `direction` (1 or -1),
/// and where the stroke of the line stops.
fn arrowhead(head: Arrowhead, tip: f32, direction: f32, width: f32) -> Option<(Head, f32)> {
    let size = head.size.factor() * width;
    let (length, half) = (size, size / 2.);
    let back = tip - direction * length;
    let at = |along: f32, across: f32| (tip - direction * along, across);
    Some(match head.kind {
        HeadKind::None => return None,
        HeadKind::Triangle => (
            Head::Filled(vec![
                Seg::Move(at(0., 0.)),
                Seg::Line(at(length, half)),
                Seg::Line(at(length, -half)),
                Seg::Close,
            ]),
            back,
        ),
        HeadKind::Arrow => (
            Head::Open([at(length, half), at(0., 0.), at(length, -half)]),
            tip,
        ),
        // Diamonds and circles are centered on the end of the line, as in
        // PowerPoint.
        HeadKind::Diamond => (
            Head::Filled(vec![
                Seg::Move(at(-length / 2., 0.)),
                Seg::Line(at(0., half)),
                Seg::Line(at(length / 2., 0.)),
                Seg::Line(at(0., -half)),
                Seg::Close,
            ]),
            tip - direction * length / 2.,
        ),
        HeadKind::Circle => (
            Head::Filled(ellipse_at((tip, 0.), length / 2., half)),
            tip - direction * length / 2.,
        ),
    })
}

/// Start and end of a linear gradient in a box: through the center, in the
/// direction of `angle` (degrees, clockwise, 0 = left to right), long
/// enough for the end colors to reach the corners.
pub fn linear_gradient_line(angle: f32, width: f32, height: f32) -> (P, P) {
    let (sin, cos) = angle.to_radians().sin_cos();
    let half = ((width * cos).abs() + (height * sin).abs()) / 2.;
    let (cx, cy) = (width / 2., height / 2.);
    (
        (cx - cos * half, cy - sin * half),
        (cx + cos * half, cy + sin * half),
    )
}

/// Center and radii of a radial gradient in a box, from fractions of it.
pub fn radial_gradient_ellipse(center: Vec2, radius: Vec2, width: f32, height: f32) -> (P, P) {
    (
        (center.x * width, center.y * height),
        (radius.x * width, radius.y * height),
    )
}

/// Where an image of `image` pixels goes in a box, as (x, y, width, height)
/// in local units. It can reach past the box (cover): clip to the shape.
pub fn fit_rect(fit: ImageFit, width: f32, height: f32, image: (u32, u32)) -> (f32, f32, f32, f32) {
    let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
    let scale = match fit {
        ImageFit::Stretch => return (0., 0., width, height),
        ImageFit::Cover => (width / iw).max(height / ih),
        ImageFit::Contain => (width / iw).min(height / ih),
    };
    let (w, h) = (iw * scale, ih * scale);
    ((width - w) / 2., (height - h) / 2., w, h)
}

/// How far the stroke and the heads of a shape reach past its frame.
fn reach(kind: &ElementKind) -> f32 {
    let Some(stroke) = kind.stroke() else {
        return 0.;
    };
    let heads = match kind {
        ElementKind::Line(line) => [line.start, line.end]
            .iter()
            .filter(|head| !head.is_none())
            .map(|head| head.size.factor() * stroke.width / 2. + stroke.width)
            .fold(0., f32::max),
        _ => 0.,
    };
    (stroke.width / 2.).max(heads)
}

/// The frame grown to hold the stroke and the heads, with the same center
/// and rotation.
pub fn visual_frame(element: &Element) -> Frame {
    let margin = reach(&element.kind);
    let frame = element.frame;
    Frame {
        x: frame.x - margin,
        y: frame.y - margin,
        width: frame.width + 2. * margin,
        height: frame.height + 2. * margin,
        rotation: frame.rotation,
    }
}

/// Signed distance from a point to the outline of a rounded box centered on
/// the origin with half sizes `half`: negative inside.
fn rounded_box_distance((x, y): P, half: P, radius: f32) -> f32 {
    let r = radius.clamp(0., half.0.min(half.1));
    let qx = x.abs() - (half.0 - r);
    let qy = y.abs() - (half.1 - r);
    let outside = qx.max(0.).hypot(qy.max(0.));
    outside + qx.max(qy).min(0.) - r
}

/// Approximate signed distance from a point to an ellipse centered on the
/// origin: negative inside.
fn ellipse_distance((x, y): P, (rx, ry): P) -> f32 {
    if rx <= 0. || ry <= 0. {
        return x.hypot(y) - rx.max(ry).max(0.);
    }
    let k0 = (x / rx).hypot(y / ry);
    let k1 = (x / (rx * rx)).hypot(y / (ry * ry));
    if k1 == 0. {
        return -rx.min(ry);
    }
    k0 * (k0 - 1.) / k1
}

fn segment_distance((x, y): P, (ax, ay): P, (bx, by): P) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        (((x - ax) * dx + (y - ay) * dy) / length).clamp(0., 1.)
    } else {
        0.
    };
    (x - ax - t * dx).hypot(y - ay - t * dy)
}

/// Whether a slide point hits the shape, within `slack` slide units. A
/// rectangle or an ellipse without a fill is hit only on its outline.
pub fn hit(element: &Element, x: f32, y: f32, slack: f32) -> bool {
    let frame = &element.frame;
    let (u, v) = frame.to_local(x, y);
    let half = (frame.width / 2., frame.height / 2.);
    let local = (u - half.0, v - half.1);
    let stroke = element.kind.stroke().map_or(0., |stroke| stroke.width / 2.);
    let band = stroke + slack;
    let distance = match &element.kind {
        ElementKind::Rectangle(shape) => rounded_box_distance(local, half, shape.corner_radius),
        ElementKind::Ellipse(_) => ellipse_distance(local, half),
        ElementKind::Line(line) => {
            let heads = [(line.start, 0.), (line.end, frame.width)]
                .iter()
                .filter(|(head, _)| !head.is_none())
                .any(|(head, tip)| {
                    let size = head.size.factor() * line.stroke.width;
                    (u - tip).hypot(v - frame.height / 2.) <= size / 2. + slack
                });
            let on_line = segment_distance(
                (u, v),
                (0., frame.height / 2.),
                (frame.width, frame.height / 2.),
            ) <= band;
            return on_line || heads;
        }
        _ => return frame.contains(x, y),
    };
    let filled = element.kind.fill().is_some_and(|fill| !fill.is_none());
    if filled {
        distance <= band
    } else {
        distance.abs() <= band
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{EllipseElement, Fill, HeadSize, RectangleElement, Stroke};

    fn element(frame: Frame, kind: ElementKind) -> Element {
        Element::new(crate::document::ElementId(1), frame, kind)
    }

    fn square() -> Frame {
        Frame {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
            rotation: 0.,
        }
    }

    #[test]
    fn filled_ellipses_are_hit_inside_but_not_in_the_corners() {
        let ellipse = element(square(), ElementKind::Ellipse(EllipseElement::default()));
        assert!(hit(&ellipse, 50., 50., 0.));
        assert!(hit(&ellipse, 99., 50., 0.));
        assert!(!hit(&ellipse, 5., 5., 2.));
    }

    #[test]
    fn empty_shapes_are_hit_only_on_their_outline() {
        let rectangle = element(
            square(),
            ElementKind::Rectangle(RectangleElement {
                fill: Fill::None,
                stroke: Some(Stroke::default()),
                corner_radius: 0.,
            }),
        );
        assert!(!hit(&rectangle, 50., 50., 3.));
        assert!(hit(&rectangle, 1., 50., 0.));
        assert!(hit(&rectangle, 104., 50., 3.));
        assert!(!hit(&rectangle, 110., 50., 3.));
    }

    #[test]
    fn rounded_corners_are_not_hit() {
        let rectangle = element(
            square(),
            ElementKind::Rectangle(RectangleElement {
                corner_radius: 40.,
                ..RectangleElement::default()
            }),
        );
        assert!(!hit(&rectangle, 3., 3., 0.));
        assert!(hit(&rectangle, 50., 3., 0.));
    }

    #[test]
    fn thin_turned_lines_are_hit_near_their_axis() {
        let line = element(
            Frame::from_line((0., 0.), (100., 100.), 0.),
            ElementKind::Line(LineElement {
                stroke: Stroke {
                    width: 1.,
                    ..Stroke::default()
                },
                ..LineElement::default()
            }),
        );
        assert!(hit(&line, 50., 52., 3.));
        assert!(!hit(&line, 50., 60., 3.));
        assert!(!hit(&line, 120., 120., 3.));
    }

    #[test]
    fn filled_heads_cut_the_stroke_and_open_ones_do_not() {
        let line = LineElement {
            stroke: Stroke {
                width: 2.,
                ..Stroke::default()
            },
            start: Arrowhead::new(HeadKind::Arrow, HeadSize::Medium),
            end: Arrowhead::new(HeadKind::Triangle, HeadSize::Large),
        };
        let geometry = line_geometry(100., &line);
        assert_eq!(geometry.segment, Some(((0., 0.), (90., 0.))));
        assert_eq!(geometry.heads.len(), 2);
        let short = line_geometry(5., &line);
        assert_eq!(short.segment, None);
    }

    #[test]
    fn linear_gradients_reach_the_corners() {
        assert_eq!(
            linear_gradient_line(0., 200., 100.),
            ((0., 50.), (200., 50.))
        );
        let (start, end) = linear_gradient_line(90., 200., 100.);
        assert!((start.1 - 0.).abs() < 1e-3 && (end.1 - 100.).abs() < 1e-3);
    }

    #[test]
    fn images_cover_contain_or_stretch() {
        assert_eq!(
            fit_rect(ImageFit::Cover, 100., 100., (200, 100)),
            (-50., 0., 200., 100.)
        );
        assert_eq!(
            fit_rect(ImageFit::Contain, 100., 100., (200, 100)),
            (0., 25., 100., 50.)
        );
        assert_eq!(
            fit_rect(ImageFit::Stretch, 100., 100., (200, 100)),
            (0., 0., 100., 100.)
        );
    }
}
