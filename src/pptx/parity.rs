//! Structural parity: reads an exported deck back and compares each shape
//! with the element it comes from. The expected values are worked out here
//! from the document model, apart from the writer, so a mistake in the
//! writer shows as a difference.

use crate::document::{
    Element, ElementKind, Fill, Frame, GradientStop, HeadKind, HeadSize, Presentation, Stroke,
};

use super::inspect::{
    Color, Deck, FillSummary, InspectError, LineSummary, ShapeKind, ShapeSummary,
};

/// Largest difference of a position or a size, in slide units.
const DISTANCE: f32 = 0.01;
/// Largest difference of an angle, in degrees.
const ANGLE: f32 = 0.01;
/// Largest difference of an alpha or a gradient position.
const FRACTION: f32 = 0.000_02;

/// The differences between `presentation` and `deck`; empty when they
/// match.
pub fn compare(presentation: &Presentation, deck: &Deck) -> Result<Vec<String>, InspectError> {
    let slides = deck.summary()?;
    let mut out = Vec::new();
    if slides.len() != presentation.slides.len() {
        out.push(format!(
            "the deck has {} slide(s), the presentation {}",
            slides.len(),
            presentation.slides.len()
        ));
    }
    for (slide, summary) in presentation.slides.iter().zip(&slides) {
        let mut check = Check {
            out: &mut out,
            at: format!("slide {}", slide.id.0),
        };
        check.elements(&slide.elements, &summary.shapes, 1.);
    }
    Ok(out)
}

struct Check<'a> {
    out: &'a mut Vec<String>,
    at: String,
}

impl Check<'_> {
    fn fail(&mut self, message: impl std::fmt::Display) {
        self.out.push(format!("{}: {message}", self.at));
    }

    fn elements(&mut self, elements: &[Element], shapes: &[ShapeSummary], opacity: f32) {
        let visible: Vec<&Element> = elements.iter().filter(|element| !element.hidden).collect();
        if visible.len() != shapes.len() {
            self.fail(format!(
                "{} shape(s) for {} visible element(s)",
                shapes.len(),
                visible.len()
            ));
        }
        let at = self.at.clone();
        for (element, shape) in visible.into_iter().zip(shapes) {
            self.at = format!("{at} element {}", element.id.0);
            self.element(element, shape, opacity * element.opacity);
        }
        self.at = at;
    }

    fn element(&mut self, element: &Element, shape: &ShapeSummary, opacity: f32) {
        let expected_kind = match &element.kind {
            ElementKind::Group(_) => ShapeKind::Group,
            ElementKind::Line(_) => ShapeKind::Connector,
            ElementKind::Table(_) => ShapeKind::Frame,
            ElementKind::Rectangle(_) | ElementKind::Ellipse(_) | ElementKind::Text(_) => {
                ShapeKind::Shape
            }
        };
        if shape.kind != expected_kind {
            self.fail(format!("is a {:?}, not a {expected_kind:?}", shape.kind));
            return;
        }
        if let Some(name) = &element.name
            && &shape.name != name
        {
            self.fail(format!("name {:?}, not {name:?}", shape.name));
        }
        let frame = match &element.kind {
            ElementKind::Line(_) => Frame {
                y: element.frame.y + element.frame.height / 2.,
                height: 0.,
                ..element.frame
            },
            _ => element.frame,
        };
        self.frame(&frame, &shape.frame);

        match &element.kind {
            ElementKind::Group(group) => self.elements(&group.children, &shape.children, opacity),
            ElementKind::Rectangle(rectangle) => {
                let side = frame.width.min(frame.height);
                let radius = rectangle.corner_radius.min(side / 2.);
                if radius > 0. && side > 0. {
                    self.equal("geometry", shape.geometry.as_deref(), Some("roundRect"));
                    let adjust = shape.adjust.unwrap_or(0) as f32 / 100_000. * side;
                    // The adjust value is rounded to 100 000ths of the side.
                    if (adjust - radius).abs() > DISTANCE + side * 1e-5 {
                        self.fail(format!("corner radius {adjust}, not {radius}"));
                    }
                } else {
                    self.equal("geometry", shape.geometry.as_deref(), Some("rect"));
                }
                self.fill(&rectangle.fill, shape.fill.as_ref(), opacity, &frame);
                self.stroke(
                    rectangle.stroke.as_ref(),
                    None,
                    shape.line.as_ref(),
                    opacity,
                );
            }
            ElementKind::Ellipse(ellipse) => {
                self.equal("geometry", shape.geometry.as_deref(), Some("ellipse"));
                self.fill(&ellipse.fill, shape.fill.as_ref(), opacity, &frame);
                self.stroke(ellipse.stroke.as_ref(), None, shape.line.as_ref(), opacity);
            }
            ElementKind::Line(line) => {
                self.equal("geometry", shape.geometry.as_deref(), Some("line"));
                self.stroke(
                    Some(&line.stroke),
                    Some([
                        (line.start.kind, line.start.size),
                        (line.end.kind, line.end.size),
                    ]),
                    shape.line.as_ref(),
                    opacity,
                );
            }
            ElementKind::Text(_) | ElementKind::Table(_) => {}
        }
    }

    fn equal<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, got: T, expected: T) {
        if got != expected {
            self.fail(format!("{what} {got:?}, not {expected:?}"));
        }
    }

    fn frame(&mut self, expected: &Frame, got: &Frame) {
        let near = |a: f32, b: f32| (a - b).abs() <= DISTANCE;
        let turn = (got.rotation - expected.rotation).rem_euclid(360.);
        if !(near(got.x, expected.x)
            && near(got.y, expected.y)
            && near(got.width, expected.width)
            && near(got.height, expected.height)
            && turn.min(360. - turn) <= ANGLE)
        {
            self.fail(format!("frame {got:?}, not {expected:?}"));
        }
    }

    fn color(&mut self, what: &str, got: Option<&Color>, rgb: u32, alpha: f32) {
        match got {
            Some(color) if color.rgb == rgb && (color.alpha - alpha).abs() <= FRACTION => {}
            _ => self.fail(format!("{what} {got:?}, not {rgb:06X} at alpha {alpha}")),
        }
    }

    fn stops(&mut self, got: &[(f32, Color)], expected: &[GradientStop], opacity: f32) {
        if got.len() != expected.len() {
            self.fail(format!(
                "{} gradient stop(s), not {}",
                got.len(),
                expected.len()
            ));
            return;
        }
        for ((position, color), stop) in got.iter().zip(expected) {
            if (position - stop.position).abs() > FRACTION {
                self.fail(format!("stop at {position}, not {}", stop.position));
            }
            self.color("stop", Some(color), stop.color.0, stop.opacity * opacity);
        }
    }

    fn fill(&mut self, fill: &Fill, got: Option<&FillSummary>, opacity: f32, frame: &Frame) {
        match (fill, got) {
            (Fill::None, Some(FillSummary::None)) => {}
            (Fill::Solid(solid), Some(FillSummary::Solid(color))) => {
                self.color("fill", Some(color), solid.color.0, solid.opacity * opacity);
            }
            (
                Fill::LinearGradient(gradient),
                Some(FillSummary::Linear {
                    angle,
                    scaled,
                    stops,
                }),
            ) => {
                let turn = (angle - gradient.angle).rem_euclid(360.);
                if turn.min(360. - turn) > ANGLE || *scaled {
                    self.fail(format!(
                        "linear gradient at {angle}° (scaled {scaled}), not {}°",
                        gradient.angle
                    ));
                }
                self.stops(stops, &gradient.stops, opacity);
            }
            (
                Fill::RadialGradient(gradient),
                Some(FillSummary::Path {
                    path,
                    fill_to,
                    tile,
                    stops,
                }),
            ) => {
                self.equal("gradient path", path.as_str(), "circle");
                let (cx, cy) = (gradient.center.x, gradient.center.y);
                let focus = [cx, cy, 1. - cx, 1. - cy];
                if fill_to
                    .iter()
                    .zip(focus)
                    .any(|(got, want)| (got - want).abs() > 1e-4)
                    || tile.iter().any(|inset| *inset != 0.)
                {
                    self.fail(format!(
                        "radial focus {fill_to:?} and tile {tile:?}, not the point {:?}",
                        gradient.center
                    ));
                }
                // The path is a circle with half the diagonal of the box.
                let (width, height) = (frame.width, frame.height);
                let reach = width.hypot(height) / 2.;
                let scale = (gradient.radius.x * width + gradient.radius.y * height) / 2. / reach;
                let expected: Vec<GradientStop> = gradient
                    .stops
                    .iter()
                    .map(|stop| GradientStop {
                        position: stop.position * scale,
                        ..*stop
                    })
                    .collect();
                let kept = expected
                    .iter()
                    .take_while(|stop| stop.position <= 1.)
                    .count();
                let cut = kept < expected.len();
                if stops.len() != kept + usize::from(cut) {
                    self.fail(format!(
                        "{} radial stop(s), not {}",
                        stops.len(),
                        kept + usize::from(cut)
                    ));
                } else {
                    self.stops(&stops[..kept], &expected[..kept], opacity);
                    if cut && (stops[kept].0 - 1.).abs() > FRACTION {
                        self.fail(format!("the cut stop is at {}, not 1", stops[kept].0));
                    }
                }
            }
            (Fill::Image(_) | Fill::Video(_) | Fill::Shader(_), _) => {}
            (fill, got) => self.fail(format!("fill {got:?}, not {}", fill.label())),
        }
    }

    fn stroke(
        &mut self,
        stroke: Option<&Stroke>,
        heads: Option<[(HeadKind, HeadSize); 2]>,
        got: Option<&LineSummary>,
        opacity: f32,
    ) {
        let Some(line) = got else {
            self.fail("has no a:ln");
            return;
        };
        let Some(stroke) = stroke else {
            if line.fill != FillSummary::None {
                self.fail(format!("outline {:?}, not none", line.fill));
            }
            return;
        };
        match line.width {
            Some(width) if (width - stroke.width).abs() <= DISTANCE => {}
            width => self.fail(format!("stroke width {width:?}, not {}", stroke.width)),
        }
        match &line.fill {
            FillSummary::Solid(color) => self.color(
                "stroke",
                Some(color),
                stroke.color.0,
                stroke.opacity * opacity,
            ),
            other => self.fail(format!("stroke fill {other:?}, not solid")),
        }
        self.equal("cap", line.cap.as_deref(), Some("flat"));
        let dash: &[(f32, f32)] = match stroke.dash {
            crate::document::Dash::Solid => &[],
            crate::document::Dash::Dashed => &[(4., 3.)],
            crate::document::Dash::Dotted => &[(1., 1.)],
        };
        self.equal("dash", line.dash.as_slice(), dash);
        let heads = heads.unwrap_or([(HeadKind::None, HeadSize::Medium); 2]);
        for ((kind, size), got, end) in [
            (heads[0], &line.head, "start"),
            (heads[1], &line.tail, "end"),
        ] {
            let name = match kind {
                HeadKind::None => None,
                HeadKind::Triangle => Some("triangle"),
                HeadKind::Arrow => Some("arrow"),
                HeadKind::Diamond => Some("diamond"),
                HeadKind::Circle => Some("oval"),
            };
            let size = match size {
                HeadSize::Small => "sm",
                HeadSize::Medium => "med",
                HeadSize::Large => "lg",
            };
            let expected = name.map(|name| (name.to_string(), size.to_string(), size.to_string()));
            self.equal(end, got.as_ref(), expected.as_ref());
        }
    }
}
