//! Structural parity: reads an exported deck back and compares each shape
//! with the element it comes from. The expected values are worked out here
//! from the document model, apart from the writer, so a mistake in the
//! writer shows as a difference.

use crate::document::{
    Element, ElementKind, Fill, Frame, GradientStop, HAlign, HeadKind, HeadSize, Presentation,
    Start, Stroke, TableElement, TextCase, TextElement, TextSizing, TextStyle, VAlign,
};

use super::inspect::{
    Color, Deck, FillSummary, InspectError, LineSummary, RunSummary, ShapeKind, ShapeSummary,
    TextSummary, TimingSummary,
};

/// Largest difference of a position or a size, in slide units.
const DISTANCE: f32 = 0.01;
/// Largest difference of an angle, in degrees.
const ANGLE: f32 = 0.01;
/// Largest difference of an alpha or a gradient position.
const FRACTION: f32 = 0.000_02;

/// A video the slide timing must start.
struct Movie {
    shape: u32,
    start: Start,
    looped: bool,
    muted: bool,
    /// Milliseconds.
    duration: u32,
}

/// A shape whose video is contained in its box, with an outline: the
/// outline is a shape of its own.
fn outlined_movie(element: &Element) -> bool {
    let contained = match element.kind.fill() {
        Some(Fill::Video(video)) => video.fit == crate::document::ImageFit::Contain,
        _ => false,
    };
    contained && element.kind.stroke().is_some()
}

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
            movies: Vec::new(),
            out: &mut out,
            at: format!("slide {}", slide.id.0),
        };
        check.elements(presentation, &slide.elements, &summary.shapes, 1.);
        check.timing(&summary.timing);
    }
    Ok(out)
}

struct Check<'a> {
    deck: &'a Deck,
    out: &'a mut Vec<String>,
    /// The videos of the slide in paint order: the picture id and what the
    /// fill asks for.
    movies: Vec<Movie>,
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
        // A contained video with an outline is a picture and a shape.
        let wanted: usize = visible
            .iter()
            .map(|element| 1 + usize::from(outlined_movie(element)))
            .sum();
        if wanted != shapes.len() {
            self.fail(format!(
                "{} shape(s) for {} visible element(s)",
                shapes.len(),
                visible.len()
            ));
        }
        let at = self.at.clone();
        let mut shapes = shapes.iter();
        for element in visible {
            let Some(shape) = shapes.next() else { break };
            self.at = format!("{at} element {}", element.id.0);
            self.element(presentation, element, shape, opacity * element.opacity);
            if outlined_movie(element)
                && let Some(outline) = shapes.next()
            {
                self.frame(&element.frame, &outline.frame);
                self.equal(
                    "outline fill",
                    outline.fill.as_ref(),
                    Some(&FillSummary::None),
                );
                self.stroke(
                    element.kind.stroke(),
                    None,
                    outline.line.as_ref(),
                    opacity * element.opacity,
                );
            }
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
        let turned_table =
            matches!(element.kind, ElementKind::Table(_)) && element.frame.rotation != 0.;
        if let Some(fill @ (Fill::Video(_) | Fill::Shader(_))) = element.kind.fill() {
            // A shader that does not render leaves a shape without fill.
            if matches!(fill, Fill::Shader(_))
                && shape.kind == ShapeKind::Shape
                && shape.fill == Some(FillSummary::None)
            {
                return;
            }
            self.movie(presentation, element, fill, shape, opacity);
            return;
        }
        if shape.kind != expected_kind && !turned_table {
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
        match &element.kind {
            ElementKind::Text(text) => self.text(presentation, element, text, shape, opacity),
            ElementKind::Table(_) => {}
            _ => self.frame(&frame, &shape.frame),
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
            ElementKind::Table(table) => self.table(presentation, element, table, shape, opacity),
            ElementKind::Text(_) => {}
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
        self.paragraphs(body, text, &layout, opacity);
    }

    /// The paragraphs of a text body: one line of the deck for each line of
    /// the layout, with the style of the text.
    fn paragraphs(
        &mut self,
        body: &TextSummary,
        text: &TextElement,
        layout: &crate::text_layout::TextLayout,
        opacity: f32,
    ) {
        let height = layout.lines[0].height;
        let justify = text.style.align == HAlign::Justify;
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
            // An empty line has no run: a paragraph of nothing is one empty line.
            self.equal("lines", &paragraph.lines, &expected);
            for run in &paragraph.runs {
                self.run(run, &text.style, opacity);
            }
        }
    }

    /// A shape with a video or shader fill: a picture that embeds the video,
    /// with a poster, placed and cropped by the fit.
    fn movie(
        &mut self,
        presentation: &Presentation,
        element: &Element,
        fill: &Fill,
        shape: &ShapeSummary,
        opacity: f32,
    ) {
        if shape.kind != ShapeKind::Picture {
            self.fail(format!("a video is a {:?}, not a picture", shape.kind));
            return;
        }
        let Some(media) = &shape.media else {
            self.fail("the picture has no video");
            return;
        };
        self.equal("media embed", media.embed.as_str(), media.video.as_str());
        let Some(bytes) = self.deck.part(&media.video) else {
            self.fail(format!("the video {} is missing", media.video));
            return;
        };
        let (fit, fill_opacity, start, looped, muted, duration) = match fill {
            Fill::Video(video) => {
                let Some(data) = presentation.videos.get(video.id) else {
                    self.fail(format!("video {} is missing", video.id.0));
                    return;
                };
                if *bytes != *data.bytes {
                    self.fail(format!(
                        "{} is not the bytes of video {}",
                        media.video, video.id.0
                    ));
                }
                (
                    video.fit,
                    video.opacity,
                    video.start,
                    video.looped,
                    video.muted,
                    data.duration,
                )
            }
            Fill::Shader(shader) => {
                if bytes.get(4..8) != Some(b"ftyp") {
                    self.fail(format!("{} is not an MP4 file", media.video));
                }
                (
                    crate::document::ImageFit::Stretch,
                    shader.opacity,
                    shader.start,
                    shader.looped,
                    true,
                    shader.duration,
                )
            }
            _ => return,
        };
        let Some(FillSummary::Blip {
            part,
            alpha,
            source,
            ..
        }) = &shape.fill
        else {
            self.fail(format!("the picture has no poster: {:?}", shape.fill));
            return;
        };
        if (alpha - fill_opacity * opacity).abs() > FRACTION {
            self.fail(format!(
                "poster alpha {alpha}, not {}",
                fill_opacity * opacity
            ));
        }
        let Some(poster) = self
            .deck
            .part(part)
            .and_then(|bytes| crate::images::ImageData::read(bytes.to_vec().into()).ok())
        else {
            self.fail(format!("the poster {part} cannot be read"));
            return;
        };
        let frame = element.frame;
        let (x, y, w, h) = crate::shape::fit_rect(
            fit,
            frame.width,
            frame.height,
            (poster.width, poster.height),
        );
        let preset = match &element.kind {
            ElementKind::Ellipse(_) => "ellipse",
            ElementKind::Rectangle(rectangle)
                if rectangle.corner_radius > 0. && frame.width.min(frame.height) > 0. =>
            {
                "roundRect"
            }
            _ => "rect",
        };
        if fit == crate::document::ImageFit::Contain {
            let local = Frame {
                x: frame.x + x,
                y: frame.y + y,
                width: w,
                height: h,
                rotation: 0.,
            };
            let placed = crate::document::turn_frame(&local, frame.center(), frame.rotation);
            self.frame(&placed, &shape.frame);
            self.equal("geometry", shape.geometry.as_deref(), Some("rect"));
            if source.iter().any(|inset| *inset != 0.) {
                self.fail(format!("a contained video is cropped by {source:?}"));
            }
        } else {
            self.frame(&frame, &shape.frame);
            self.equal("geometry", shape.geometry.as_deref(), Some(preset));
            let expected = [
                -x / w,
                -y / h,
                (x + w - frame.width) / w,
                (y + h - frame.height) / h,
            ];
            if source
                .iter()
                .zip(expected)
                .any(|(got, want)| (got - want).abs() > 1e-4)
            {
                self.fail(format!("video crop {source:?}, not {expected:?}"));
            }
            self.stroke(element.kind.stroke(), None, shape.line.as_ref(), opacity);
        }
        self.movies.push(Movie {
            shape: shape.id,
            start,
            looped,
            muted,
            duration: (duration * 1000.).round() as u32,
        });
    }

    /// The timing starts the automatic videos with the slide, then each
    /// video on a click, in paint order.
    fn timing(&mut self, got: &[TimingSummary]) {
        let movies = std::mem::take(&mut self.movies);
        let auto = movies.iter().filter(|movie| movie.start == Start::Auto);
        let clicks = movies.iter().filter(|movie| movie.start == Start::OnClick);
        let expected: Vec<TimingSummary> = auto
            .enumerate()
            .map(|(index, movie)| {
                (
                    movie,
                    true,
                    if index == 0 {
                        "afterEffect"
                    } else {
                        "withEffect"
                    },
                )
            })
            .chain(clicks.map(|movie| (movie, false, "clickEffect")))
            .map(|(movie, with_slide, kind)| TimingSummary {
                shape: movie.shape,
                kind: kind.to_string(),
                with_slide,
                duration: movie.duration.max(1),
                muted: movie.muted,
                looped: movie.looped,
            })
            .collect();
        if got != expected.as_slice() {
            self.fail(format!("timing {got:?}, not {expected:?}"));
        }
    }

    /// An editable table at the frame of the element, or, when it is
    /// turned, a group of its parts.
    fn table(
        &mut self,
        presentation: &Presentation,
        element: &Element,
        table: &TableElement,
        shape: &ShapeSummary,
        opacity: f32,
    ) {
        let Ok(layout) = crate::table::layout(table, &presentation.fonts) else {
            self.fail("has no layout");
            return;
        };
        if element.frame.rotation != 0. {
            if shape.kind != ShapeKind::Group {
                self.fail(format!("a turned table is a {:?}, not a group", shape.kind));
                return;
            }
            let parts: Vec<Element> =
                crate::table::parts(element.id, &element.frame, table, &layout)
                    .iter()
                    .map(|part| part.element().clone())
                    .collect();
            self.elements(presentation, &parts, &shape.children, opacity);
            return;
        }
        if shape.kind != ShapeKind::Frame {
            self.fail(format!(
                "a table is a {:?}, not a graphic frame",
                shape.kind
            ));
            return;
        }
        let frame = Frame {
            width: layout.width(),
            height: layout.height(),
            ..element.frame
        };
        self.frame(&frame, &shape.frame);
        let Some(got) = &shape.table else {
            self.fail("has no a:tbl");
            return;
        };
        self.equal("table style", got.style.as_str(), super::NO_TABLE_STYLE);
        let near = |a: &[f32], b: &[f32]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= DISTANCE)
        };
        if !near(&got.columns, &layout.columns) || !near(&got.rows, &layout.rows) {
            self.fail(format!(
                "grid {:?} × {:?}, not {:?} × {:?}",
                got.columns, got.rows, layout.columns, layout.rows
            ));
            return;
        }
        let owners = table.owners();
        let at = self.at.clone();
        for cell in &got.cells {
            self.at = format!("{at} cell {},{}", cell.row, cell.column);
            let owner = owners[cell.row][cell.column];
            if owner != (cell.row, cell.column) {
                self.equal(
                    "merge",
                    (cell.h_merge, cell.v_merge),
                    (owner.1 < cell.column, owner.0 < cell.row),
                );
                continue;
            }
            let span = table.span_of(cell.row, cell.column);
            self.equal(
                "span",
                (cell.grid_span, cell.row_span),
                (span.columns, span.rows),
            );
            let Some(laid) = layout
                .cells
                .iter()
                .find(|laid| (laid.row, laid.column) == (cell.row, cell.column))
            else {
                self.fail("is not in the layout");
                continue;
            };
            let style = &laid.text.style;
            let anchor = match style.vertical_align {
                VAlign::Top => "t",
                VAlign::Middle => "ctr",
                VAlign::Bottom => "b",
            };
            self.equal("anchor", cell.anchor.as_str(), anchor);
            // The text starts on the padding, and the top margin moves the
            // viewer baseline onto the Sliderino one.
            let first = &laid.layout.lines[0];
            let shift = (first.height - 0.2 * style.size) - (first.baseline - first.top);
            let [left, right, top, bottom] = cell.margins;
            let slack = 0.25 * style.size;
            let starts = match style.align {
                HAlign::Left | HAlign::Justify => (left - table.padding).abs() <= DISTANCE,
                HAlign::Right => (right - table.padding).abs() <= DISTANCE,
                HAlign::Center => (left - right).abs() <= DISTANCE,
            };
            let room = left + right >= 2. * table.padding - slack - DISTANCE;
            let vertical = (top - (table.padding - shift).max(0.)).abs() <= DISTANCE
                && (top + bottom - 2. * table.padding).abs() <= DISTANCE
                || table.padding < shift.abs();
            if !(starts && room && vertical) {
                self.fail(format!(
                    "margins {:?} for padding {} and shift {shift}",
                    cell.margins, table.padding
                ));
            }
            let rect = laid.rect;
            self.fill(
                presentation,
                table.fill_of(cell.row, cell.column),
                cell.fill.as_ref(),
                opacity,
                &rect,
            );
            let expected = crate::pptx::table::borders(table, &owners, cell.row, cell.column);
            for (side, (got, stroke)) in cell.borders.iter().zip(expected).enumerate() {
                let name = ["left", "right", "top", "bottom"][side];
                match (got, stroke) {
                    (Some(line), None) if line.fill == FillSummary::None => {}
                    (got, Some(stroke)) => self.stroke(Some(&stroke), None, got.as_ref(), opacity),
                    (got, None) => self.fail(format!("{name} border {got:?}, not none")),
                }
            }
            if laid.text.content.is_empty() {
                continue;
            }
            self.paragraphs(&cell.text, &laid.text, &laid.layout, opacity);
        }
        self.at = at;
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
