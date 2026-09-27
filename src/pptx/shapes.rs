//! DrawingML for the shapes: transforms, geometry, fills and outlines.
//!
//! Opacity is multiplied into every color, as the CPU renderer does
//! (`render.rs`): a group with opacity makes each of its shapes more
//! transparent, not the group as one layer.

use crate::document::{
    Arrowhead, Dash, Fill, Frame, GradientStop, HeadKind, HeadSize, Rgb, Stroke, Vec2,
};

use super::units::{angle, emu, percent};
use super::xml::Xml;

/// `a:xfrm` of a frame. Lines use [`line_transform`].
pub fn transform(xml: &mut Xml, frame: &Frame) {
    xml.start("a:xfrm")
        .attr_opt("rot", (frame.rotation != 0.).then(|| angle(frame.rotation)));
    offset_extent(xml, frame.x, frame.y, frame.width, frame.height);
    xml.end();
}

/// `a:xfrm` of a group: its box, with the children in the same units.
pub fn group_transform(xml: &mut Xml, frame: &Frame) {
    xml.start("a:xfrm")
        .attr_opt("rot", (frame.rotation != 0.).then(|| angle(frame.rotation)));
    offset_extent(xml, frame.x, frame.y, frame.width, frame.height);
    xml.empty("a:chOff", &[("x", &emu(frame.x)), ("y", &emu(frame.y))])
        .empty(
            "a:chExt",
            &[("cx", &emu(frame.width)), ("cy", &emu(frame.height))],
        );
    xml.end();
}

/// `a:xfrm` of a line: a box of height 0 on the horizontal center axis of
/// the frame. The preset line goes from its left end to its right end.
pub fn line_transform(xml: &mut Xml, frame: &Frame) {
    xml.start("a:xfrm")
        .attr_opt("rot", (frame.rotation != 0.).then(|| angle(frame.rotation)));
    offset_extent(xml, frame.x, frame.y + frame.height / 2., frame.width, 0.);
    xml.end();
}

fn offset_extent(xml: &mut Xml, x: f32, y: f32, width: f32, height: f32) {
    xml.empty("a:off", &[("x", &emu(x)), ("y", &emu(y))]).empty(
        "a:ext",
        &[("cx", &emu(width).max(0)), ("cy", &emu(height).max(0))],
    );
}

/// `a:prstGeom` with its adjust values.
pub fn geometry(xml: &mut Xml, preset: &str, adjust: Option<i64>) {
    xml.start("a:prstGeom")
        .attr("prst", preset)
        .start("a:avLst");
    if let Some(value) = adjust {
        xml.empty(
            "a:gd",
            &[("name", &"adj"), ("fmla", &format!("val {value}"))],
        );
    }
    xml.end().end();
}

/// The `roundRect` adjust value of a corner radius: the radius in
/// 100 000ths of the shorter side, at most half of it.
pub fn corner_adjust(radius: f32, width: f32, height: f32) -> i64 {
    let side = width.min(height);
    if side <= 0. {
        return 0;
    }
    percent(radius / side).clamp(0, 50_000)
}

/// `a:srgbClr`, with `a:alpha` when it is not opaque.
pub fn color(xml: &mut Xml, color: Rgb, opacity: f32) {
    xml.start("a:srgbClr").attr("val", color.hex());
    let alpha = percent(opacity.clamp(0., 1.));
    if alpha < 100_000 {
        xml.empty("a:alpha", &[("val", &alpha)]);
    }
    xml.end();
}

pub fn solid(xml: &mut Xml, rgb: Rgb, opacity: f32) {
    xml.start("a:solidFill");
    color(xml, rgb, opacity);
    xml.end();
}

/// The fill of a shape in `a:spPr`. Pictures (image, video and shader
/// fills) are written by the media module; this writes none for them.
pub fn fill(xml: &mut Xml, fill: &Fill, opacity: f32, width: f32, height: f32) {
    match fill {
        Fill::None | Fill::Image(_) | Fill::Video(_) | Fill::Shader(_) => {
            xml.empty("a:noFill", &[]);
        }
        Fill::Solid(solid_fill) => solid(xml, solid_fill.color, solid_fill.opacity * opacity),
        Fill::LinearGradient(gradient) => {
            xml.start("a:gradFill").attr("rotWithShape", 1);
            stops(xml, &gradient.stops, opacity);
            xml.empty("a:lin", &[("ang", &angle(gradient.angle)), ("scaled", &0)]);
            xml.end();
        }
        Fill::RadialGradient(gradient) => {
            let (focus, moved) = radial_path(
                gradient.center,
                gradient.radius,
                width,
                height,
                &gradient.stops,
            );
            xml.start("a:gradFill").attr("rotWithShape", 1);
            stops(xml, &moved, opacity);
            xml.start("a:path").attr("path", "circle");
            rect(xml, "a:fillToRect", focus);
            xml.end().end();
        }
    }
}

fn stops(xml: &mut Xml, stops: &[GradientStop], opacity: f32) {
    xml.start("a:gsLst");
    for stop in stops {
        xml.start("a:gs").attr("pos", percent(stop.position));
        color(xml, stop.color, stop.opacity * opacity);
        xml.end();
    }
    xml.end();
}

/// Insets (left, top, right, bottom) of a rectangle, in 100 000ths of the
/// box.
type Insets = [i64; 4];

fn rect(xml: &mut Xml, name: &'static str, [l, t, r, b]: Insets) {
    xml.empty(name, &[("l", &l), ("t", &t), ("r", &r), ("b", &b)]);
}

/// A radial gradient as a circle path gradient: the focus point, as the
/// insets of the `fillToRect`, and the stops.
///
/// A circle path gradient is a circle in slide units around the focus, with
/// a radius of half the diagonal of the box (as LibreOffice draws the
/// decks of PowerPoint). The stops are moved so that they fall where the
/// Sliderino ellipse puts them. Stops past the end of the path are cut,
/// with the color at the end. This is exact when the ellipse is a circle
/// on the slide; otherwise the mean of its two radii is used.
pub fn radial_path(
    center: Vec2,
    radius: Vec2,
    width: f32,
    height: f32,
    stops: &[GradientStop],
) -> (Insets, Vec<GradientStop>) {
    let focus = [
        percent(center.x),
        percent(center.y),
        percent(1. - center.x),
        percent(1. - center.y),
    ];
    let reach = width.hypot(height) / 2.;
    let own = (radius.x * width + radius.y * height) / 2.;
    let scale = if reach > 0. { own / reach } else { 1. };
    (focus, scale_stops(stops, scale))
}

/// Stop positions times `scale`, cut at 1.
fn scale_stops(stops: &[GradientStop], scale: f32) -> Vec<GradientStop> {
    let mut out = Vec::new();
    for (index, stop) in stops.iter().enumerate() {
        let position = stop.position * scale;
        if position <= 1. {
            out.push(GradientStop { position, ..*stop });
            continue;
        }
        // The color where the path ends, between this stop and the one
        // before.
        if let Some(before) = index.checked_sub(1).map(|index| stops[index]) {
            let start = before.position * scale;
            let t = if position > start {
                (1. - start) / (position - start)
            } else {
                0.
            };
            out.push(GradientStop {
                position: 1.,
                color: mix(before.color, stop.color, t),
                opacity: before.opacity + (stop.opacity - before.opacity) * t,
            });
        } else {
            out.push(GradientStop {
                position: 1.,
                ..*stop
            });
        }
        break;
    }
    out
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let channel = |shift: u32| {
        let a = ((a.0 >> shift) & 0xFF) as f32;
        let b = ((b.0 >> shift) & 0xFF) as f32;
        ((a + (b - a) * t).round() as u32).min(255) << shift
    };
    Rgb(channel(16) | channel(8) | channel(0))
}

/// `a:ln` of a stroke; `None` writes a line without fill.
pub fn outline(xml: &mut Xml, stroke: Option<&Stroke>, opacity: f32, heads: [Arrowhead; 2]) {
    let Some(stroke) = stroke else {
        xml.start("a:ln").empty("a:noFill", &[]).end();
        return;
    };
    xml.start("a:ln")
        .attr("w", emu(stroke.width))
        .attr("cap", "flat")
        .attr("algn", "ctr");
    solid(xml, stroke.color, stroke.opacity * opacity);
    match dash_lengths(stroke.dash) {
        None => {
            xml.empty("a:prstDash", &[("val", &"solid")]);
        }
        Some((dash, space)) => {
            xml.start("a:custDash")
                .empty("a:ds", &[("d", &dash), ("sp", &space)])
                .end();
        }
    }
    xml.empty("a:miter", &[("lim", &800_000)]);
    for (name, head) in [("a:headEnd", heads[0]), ("a:tailEnd", heads[1])] {
        if let Some(kind) = head_type(head.kind) {
            let size = head_size(head.size);
            xml.empty(name, &[("type", &kind), ("w", &size), ("len", &size)]);
        }
    }
    xml.end();
}

/// Dash and space lengths in 100 000ths of the stroke width, as
/// `shape::dash_array` gives them.
pub fn dash_lengths(dash: Dash) -> Option<(i64, i64)> {
    match dash {
        Dash::Solid => None,
        Dash::Dashed => Some((400_000, 300_000)),
        Dash::Dotted => Some((100_000, 100_000)),
    }
}

pub fn head_type(kind: HeadKind) -> Option<&'static str> {
    match kind {
        HeadKind::None => None,
        HeadKind::Triangle => Some("triangle"),
        HeadKind::Arrow => Some("arrow"),
        HeadKind::Diamond => Some("diamond"),
        HeadKind::Circle => Some("oval"),
    }
}

/// PowerPoint head sizes are 2, 3 and 5 stroke widths: the factors of
/// [`HeadSize`].
pub fn head_size(size: HeadSize) -> &'static str {
    match size {
        HeadSize::Small => "sm",
        HeadSize::Medium => "med",
        HeadSize::Large => "lg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{LinearGradient, SolidFill};

    fn written(write: impl FnOnce(&mut Xml)) -> String {
        let mut xml = Xml::fragment();
        write(&mut xml);
        xml.finish()
    }

    #[test]
    fn corner_radius_is_a_fraction_of_the_shorter_side() {
        assert_eq!(corner_adjust(20., 400., 200.), 10_000);
        assert_eq!(corner_adjust(500., 400., 200.), 50_000);
        assert_eq!(corner_adjust(5., 0., 200.), 0);
    }

    #[test]
    fn opacity_multiplies_into_the_alpha() {
        let fill = Fill::Solid(SolidFill {
            color: Rgb(0x336699),
            opacity: 0.5,
        });
        let xml = written(|xml| super::fill(xml, &fill, 0.5, 1., 1.));
        assert_eq!(
            xml,
            r#"<a:solidFill><a:srgbClr val="336699"><a:alpha val="25000"/></a:srgbClr></a:solidFill>"#
        );
    }

    #[test]
    fn linear_gradients_are_not_scaled_and_turn_with_the_shape() {
        let fill = Fill::LinearGradient(LinearGradient {
            angle: 90.,
            stops: vec![
                GradientStop::new(0., Rgb(0xFF0000)),
                GradientStop::new(1., Rgb(0x0000FF)),
            ],
        });
        let xml = written(|xml| super::fill(xml, &fill, 1., 10., 10.));
        assert!(xml.starts_with(r#"<a:gradFill rotWithShape="1">"#));
        assert!(xml.contains(r#"<a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs>"#));
        assert!(xml.contains(r#"<a:lin ang="5400000" scaled="0"/>"#));
    }

    #[test]
    fn a_centered_radial_gradient_ends_before_the_corners() {
        let stops = [
            GradientStop::new(0., Rgb(0x000000)),
            GradientStop::new(1., Rgb(0xFFFFFF)),
        ];
        let (focus, moved) =
            radial_path(Vec2::new(0.5, 0.5), Vec2::new(0.5, 0.5), 400., 400., &stops);
        assert_eq!(focus, [50_000; 4]);
        assert!((moved[1].position - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn stops_past_the_path_are_cut_with_the_color_at_its_end() {
        let stops = [
            GradientStop::new(0., Rgb(0x000000)),
            GradientStop::new(1., Rgb(0xFF0000)),
        ];
        let moved = scale_stops(&stops, 2.);
        assert_eq!(moved.len(), 2);
        assert_eq!(moved[1].position, 1.);
        assert_eq!(moved[1].color, Rgb(0x800000));
    }

    #[test]
    fn strokes_are_centered_with_flat_caps_and_exact_dashes() {
        let stroke = Stroke {
            dash: Dash::Dashed,
            width: 4.,
            ..Stroke::default()
        };
        let heads = [
            Arrowhead::new(HeadKind::Circle, HeadSize::Small),
            Arrowhead::new(HeadKind::Triangle, HeadSize::Large),
        ];
        let xml = written(|xml| outline(xml, Some(&stroke), 1., heads));
        assert!(xml.starts_with(r#"<a:ln w="30480" cap="flat" algn="ctr">"#));
        assert!(xml.contains(r#"<a:ds d="400000" sp="300000"/>"#));
        assert!(xml.contains(r#"<a:headEnd type="oval" w="sm" len="sm"/>"#));
        assert!(xml.contains(r#"<a:tailEnd type="triangle" w="lg" len="lg"/>"#));
    }

    #[test]
    fn a_shape_without_stroke_has_no_line() {
        assert_eq!(
            written(|xml| outline(xml, None, 1., [Arrowhead::NONE; 2])),
            "<a:ln><a:noFill/></a:ln>"
        );
    }
}
