//! Structural parity: reads an exported deck back and compares each shape
//! with the element it comes from. The expected values are worked out here
//! from the document model, apart from the writer, so a mistake in the
//! writer shows as a difference.

use crate::document::{
    Element, ElementKind, Fill, Frame, GradientStop, HAlign, HeadKind, HeadSize, Presentation,
    Stroke, TextCase, TextElement, TextSizing, TextStyle, VAlign,
};

use super::inspect::{
    Color, Deck, FillSummary, InspectError, LineSummary, RunSummary, ShapeKind, ShapeSummary,
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
            deck,
            out: &mut out,
            at: format!("slide {}", slide.id.0),
        };
        check.elements(presentation, &slide.elements, &summary.shapes, 1.);
    }
    Ok(out)
}

struct Check<'a> {
    deck: &'a Deck,
    out: &'a mut Vec<String>,
    at: String,
}

impl Check<'_> {
    fn fail(&mut self, message: impl std::fmt::Display) {
        self.out.push(format!("{}: {message}", self.at));
    }

    fn elements(
        &mut self,
        presentation: &Presentation,
        elements: &[Element],
        shapes: &[ShapeSummary],
        opacity: f32,
    ) {
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
            self.element(presentation, element, shape, opacity * element.opacity);
        }
        self.at = at;
    }

    fn element(
        &mut self,
        presentation: &Presentation,
        element: &Element,
        shape: &ShapeSummary,
        opacity: f32,
    ) {
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
        if let ElementKind::Text(text) = &element.kind {
            self.text(presentation, element, text, shape, opacity);
        } else {
            self.frame(&frame, &shape.frame);
        }

        match &element.kind {
            ElementKind::Group(group) => {
                self.elements(presentation, &group.children, &shape.children, opacity)
            }
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
                self.fill(
                    presentation,
                    &rectangle.fill,
                    shape.fill.as_ref(),
                    opacity,
                    &frame,
                );
                self.stroke(
                    rectangle.stroke.as_ref(),
                    None,
                    shape.line.as_ref(),
                    opacity,
                );
            }
            ElementKind::Ellipse(ellipse) => {
                self.equal("geometry", shape.geometry.as_deref(), Some("ellipse"));
                self.fill(
                    presentation,
                    &ellipse.fill,
                    shape.fill.as_ref(),
                    opacity,
                    &frame,
                );
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

    /// A text box: moved so that the viewer baseline (a fifth of the font
    /// size above the bottom of an exact line) is the Sliderino one, and
    /// one line of the deck for each line of the layout.
    fn text(
        &mut self,
        presentation: &Presentation,
        element: &Element,
        text: &TextElement,
        shape: &ShapeSummary,
        opacity: f32,
    ) {
        let Some(layout) = presentation
            .fonts
            .get(&text.style.font)
            .and_then(|font| crate::text_layout::layout(text, &element.frame, font).ok())
        else {
            self.fail("has no layout");
            return;
        };
        let first = &layout.lines[0];
        let height = first.height;
        let shift = (height - 0.2 * text.style.size) - (first.baseline - first.top);
        let (dx, dy) = crate::document::rotate_vector(0., -shift, element.frame.rotation);
        let moved = Frame {
            x: element.frame.x + dx,
            y: element.frame.y + dy,
            ..element.frame
        };
        self.frame(&moved, &shape.frame);

        let Some(body) = &shape.text else {
            self.fail("has no text body");
            return;
        };
        let justify = text.style.align == HAlign::Justify;
        self.equal(
            "wrap",
            body.wrap.as_str(),
            if justify { "square" } else { "none" },
        );
        let anchor = match (text.sizing, text.style.vertical_align) {
            (TextSizing::Fixed, VAlign::Middle) => "ctr",
            (TextSizing::Fixed, VAlign::Bottom) => "b",
            _ => "t",
        };
        self.equal("anchor", body.anchor.as_str(), anchor);
        self.equal("insets", body.insets, [0.; 4]);
        self.equal("autofit", body.autofit, false);

        let paragraphs: Vec<&str> = text.content.split('\n').collect();
        if body.paragraphs.len() != paragraphs.len() {
            self.fail(format!(
                "{} paragraph(s), not {}",
                body.paragraphs.len(),
                paragraphs.len()
            ));
            return;
        }
        let mut lines = layout.lines.iter();
        let align = match text.style.align {
            HAlign::Left => "l",
            HAlign::Center => "ctr",
            HAlign::Right => "r",
            HAlign::Justify => "just",
        };
        for (index, (paragraph, source)) in body.paragraphs.iter().zip(&paragraphs).enumerate() {
            let last = index + 1 == paragraphs.len();
            self.equal("align", paragraph.align.as_str(), align);
            let near = |a: f32, b: f32| (a - b).abs() <= 0.02;
            if !paragraph.line_height.is_some_and(|got| near(got, height)) {
                self.fail(format!(
                    "line height {:?}, not {height}",
                    paragraph.line_height
                ));
            }
            let after = if last {
                0.
            } else {
                text.style.paragraph_spacing
            };
            if !near(paragraph.space_after, after) || paragraph.space_before != 0. {
                self.fail(format!(
                    "paragraph spacing {} before, {} after, not 0 and {after}",
                    paragraph.space_before, paragraph.space_after
                ));
            }
            let mut expected: Vec<String> = Vec::new();
            for line in lines.by_ref() {
                expected.push(
                    text.content[line.range.clone()]
                        .trim_end_matches([' ', '\t'])
                        .to_string(),
                );
                if line.ends_paragraph {
                    break;
                }
            }
            if justify {
                expected = vec![source.trim_end_matches([' ', '\t']).to_string()];
            }
            self.equal("lines", &paragraph.lines, &expected);
            for run in &paragraph.runs {
                self.run(run, &text.style, opacity);
            }
        }
    }

    fn run(&mut self, run: &RunSummary, style: &TextStyle, opacity: f32) {
        let near = |a: f32, b: f32| (a - b).abs() <= 0.02;
        if !near(run.size, style.size) {
            self.fail(format!("font size {}, not {}", run.size, style.size));
        }
        if !near(run.spacing, style.size * style.letter_spacing / 100.) {
            self.fail(format!("character spacing {}", run.spacing));
        }
        self.equal("bold", run.bold, style.font.weight >= 600);
        self.equal("italic", run.italic, style.font.italic);
        self.equal("underline", run.underline, style.underline);
        self.equal("strike", run.strike, style.strikethrough);
        self.equal("caps", run.caps, style.case == TextCase::Upper);
        self.equal(
            "typeface",
            run.typeface.as_str(),
            style.font.family.as_str(),
        );
        self.color("text", run.color.as_ref(), style.color.0, opacity);
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

    fn fill(
        &mut self,
        presentation: &Presentation,
        fill: &Fill,
        got: Option<&FillSummary>,
        opacity: f32,
        frame: &Frame,
    ) {
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
            (
                Fill::Image(image),
                Some(FillSummary::Blip {
                    part,
                    alpha,
                    source,
                    fill,
                }),
            ) => {
                let Some(data) = presentation.images.get(image.id) else {
                    self.fail(format!("image {} is missing", image.id.0));
                    return;
                };
                if (alpha - image.opacity * opacity).abs() > FRACTION {
                    self.fail(format!(
                        "image alpha {alpha}, not {}",
                        image.opacity * opacity
                    ));
                }
                self.picture(part, (data.width, data.height));
                let (x, y, w, h) = crate::shape::fit_rect(
                    image.fit,
                    frame.width,
                    frame.height,
                    (data.width, data.height),
                );
                let expected = [
                    x / frame.width,
                    y / frame.height,
                    (frame.width - x - w) / frame.width,
                    (frame.height - y - h) / frame.height,
                ];
                if source.iter().any(|inset| *inset != 0.)
                    || fill
                        .iter()
                        .zip(expected)
                        .any(|(got, want)| (got - want).abs() > 1e-4)
                {
                    self.fail(format!(
                        "image placed at {fill:?} (crop {source:?}), not {expected:?}"
                    ));
                }
            }
            (Fill::Video(_) | Fill::Shader(_), _) => {}
            (fill, got) => self.fail(format!("fill {got:?}, not {}", fill.label())),
        }
    }

    /// The picture part `part` is an image of `size` upright pixels, with
    /// no EXIF turn left for the viewer to miss.
    fn picture(&mut self, part: &str, size: (u32, u32)) {
        let Some(bytes) = self.deck.part(part) else {
            self.fail(format!("the picture {part} is missing"));
            return;
        };
        match crate::images::ImageData::read(bytes.to_vec().into()) {
            Ok(data) if data.orientation <= 1 && (data.width, data.height) == size => {}
            Ok(data) => self.fail(format!(
                "the picture {part} is {}x{} turned by EXIF {}, not {}x{} upright",
                data.width, data.height, data.orientation, size.0, size.1
            )),
            Err(error) => self.fail(format!("the picture {part}: {error}")),
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
