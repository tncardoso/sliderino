//! Sliderino's text layout: shaping, line breaking and placement of a text
//! box, computed from the font bytes embedded in the presentation.
//!
//! It does not use GPUI, so the editor, the CLI, the MCP server and the
//! exporters all get the same lines. The editor only rasterizes the glyph ids
//! this module places. Positions are in slide units from the top-left corner
//! of the text frame. Only left-to-right text is supported.

use std::ops::Range;

use unicode_linebreak::linebreaks;

use crate::document::{
    FontData, Frame, HAlign, LineHeight, TextCase, TextElement, TextSizing, TextStyle, VAlign,
};

/// Tolerance for float noise when checking whether text fits a width.
const EPSILON: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutError;

/// Vertical metrics of a face, in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: f32,
    pub ascender: f32,
    /// Negative: below the baseline.
    pub descender: f32,
    pub line_gap: f32,
    /// Top of the underline stroke; negative is below the baseline.
    pub underline_position: Option<f32>,
    pub underline_thickness: Option<f32>,
    /// Top of the strikeout stroke, above the baseline.
    pub strikeout_position: Option<f32>,
    pub strikeout_thickness: Option<f32>,
}

impl FontMetrics {
    pub fn read(data: &FontData) -> Result<Self, LayoutError> {
        let face = ttf_parser::Face::parse(&data.bytes, data.index).map_err(|_| LayoutError)?;
        Ok(Self::of(&face))
    }

    fn of(face: &ttf_parser::Face) -> Self {
        let underline = face.underline_metrics();
        let strikeout = face.strikeout_metrics();
        Self {
            units_per_em: face.units_per_em() as f32,
            ascender: face.ascender() as f32,
            descender: face.descender() as f32,
            line_gap: face.line_gap() as f32,
            underline_position: underline.map(|m| m.position as f32),
            underline_thickness: underline.map(|m| m.thickness as f32),
            strikeout_position: strikeout.map(|m| m.position as f32),
            strikeout_thickness: strikeout.map(|m| m.thickness as f32),
        }
    }

    /// The "Auto" line height, as a percentage of the font size.
    #[allow(dead_code, reason = "exporters write Auto as this percentage")]
    pub fn auto_line_height_percent(&self) -> f32 {
        (self.ascender - self.descender + self.line_gap) / self.units_per_em * 100.
    }
}

/// A glyph placed in the box: `x` from the frame's left edge, `y` from the
/// line's baseline (negative is up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Byte range of the content shown on the line, trailing spaces included
    /// and the paragraph's `\n` excluded.
    pub range: Range<usize>,
    pub top: f32,
    pub height: f32,
    /// Distance of the baseline from the frame top.
    pub baseline: f32,
    /// Left and right edges of the ink-bearing part of the line (trailing
    /// spaces excluded).
    pub left: f32,
    pub right: f32,
    pub glyphs: Vec<PositionedGlyph>,
    /// The line is the last one of its paragraph.
    pub ends_paragraph: bool,
    /// Caret positions: content byte index and x, in increasing order.
    stops: Vec<(usize, f32)>,
}

impl Line {
    /// X of the caret before the byte `index` (clamped to the line).
    pub fn x_of(&self, index: usize) -> f32 {
        let mut x = self.stops[0].1;
        for &(stop, stop_x) in &self.stops {
            if stop > index {
                break;
            }
            x = stop_x;
        }
        x
    }
}

/// Offsets from the baseline (positive is down) and thicknesses of the text
/// decorations, in slide units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decorations {
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strikeout_offset: f32,
    pub strikeout_thickness: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<Line>,
    /// Widest line, trailing spaces excluded.
    pub content_width: f32,
    pub content_height: f32,
    /// Height the text is placed in: the frame height of a fixed box.
    pub box_height: f32,
    pub font_size: f32,
    pub decorations: Decorations,
    /// Visible characters the font has no glyph for; drawn as `.notdef`.
    pub missing_glyphs: usize,
}

/// A rectangle in box coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl TextLayout {
    /// How far the text runs past the bottom of a fixed box; 0 when it fits.
    pub fn overflow(&self) -> f32 {
        let over = self.content_height - self.box_height;
        if over > EPSILON { over } else { 0. }
    }

    pub fn first_baseline(&self) -> f32 {
        self.lines[0].baseline
    }

    /// Index of the line showing the caret at byte `index`. At the seam of a
    /// wrapped line the caret goes to the start of the next line.
    pub fn line_of(&self, index: usize) -> usize {
        self.lines
            .iter()
            .position(|line| {
                index >= line.range.start
                    && (index < line.range.end || (index == line.range.end && line.ends_paragraph))
            })
            .unwrap_or(self.lines.len() - 1)
    }

    /// The caret before byte `index`: a vertical segment spanning the line.
    pub fn caret(&self, index: usize) -> BoxRect {
        let line = &self.lines[self.line_of(index)];
        let x = line.x_of(index);
        BoxRect {
            left: x,
            top: line.top,
            right: x,
            bottom: line.top + line.height,
        }
    }

    /// The content byte index closest to a point in box coordinates.
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        let line = self
            .lines
            .iter()
            .find(|line| y < line.top + line.height)
            .unwrap_or_else(|| self.lines.last().expect("a layout has a line"));
        // The end of a wrapped line is the start of the next one; a click past
        // it stays on this line, before its trailing space.
        let stops = if line.ends_paragraph || line.stops.len() == 1 {
            &line.stops[..]
        } else {
            &line.stops[..line.stops.len() - 1]
        };
        stops
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map(|stop| stop.0)
            .expect("a line has a stop")
    }

    /// The first index of the line at `line`, and its x for the given index.
    /// Used to move the caret up and down.
    pub fn index_on_line(&self, line: usize, x: f32) -> usize {
        let line = &self.lines[line];
        self.index_at(x, line.top + line.height / 2.)
    }

    /// Highlight rectangles of a selected byte range, one per line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<BoxRect> {
        let mut rects = Vec::new();
        let last = self.lines.len() - 1;
        for (ix, line) in self.lines.iter().enumerate() {
            let start = range.start.max(line.range.start);
            let end = range.end.min(line.range.end);
            // The selection spans the paragraph break after this line.
            let newline = line.ends_paragraph && ix != last && range.end > line.range.end;
            if start > end || (start == end && !newline) {
                continue;
            }
            let mut right = line.x_of(end);
            if newline {
                right += self.font_size * 0.3;
            }
            rects.push(BoxRect {
                left: line.x_of(start),
                top: line.top,
                right,
                bottom: line.top + line.height,
            });
        }
        rects
    }
}

/// Lays out a text element inside its frame.
pub fn layout(
    text: &TextElement,
    frame: &Frame,
    font: &FontData,
) -> Result<TextLayout, LayoutError> {
    let wrap = match text.sizing {
        TextSizing::AutoWidth => None,
        TextSizing::AutoHeight | TextSizing::Fixed => Some(frame.width),
    };
    let fixed_height = (text.sizing == TextSizing::Fixed).then_some(frame.height);
    run(
        &text.content,
        &text.style,
        wrap,
        frame.width,
        fixed_height,
        font,
    )
}

/// Size of the text's content: the frame an auto-sized box takes. For
/// [`TextSizing::AutoWidth`] `width` is ignored.
pub fn measure(text: &TextElement, width: f32, font: &FontData) -> Result<(f32, f32), LayoutError> {
    let wrap = (text.sizing != TextSizing::AutoWidth).then_some(width);
    let layout = run(&text.content, &text.style, wrap, width, None, font)?;
    Ok((layout.content_width, layout.content_height))
}

/// A run of glyphs that forms one unbreakable unit of the text.
struct Cluster {
    /// Byte range in the displayed (case-transformed) paragraph.
    start: usize,
    glyphs: Range<usize>,
    advance: f32,
    whitespace: bool,
}

struct ShapedGlyph {
    id: u16,
    x_offset: f32,
    y_offset: f32,
    advance: f32,
}

/// A paragraph as drawn: the case-transformed text and, for every byte of it
/// (plus the end), the byte of the content it comes from.
struct Paragraph {
    display: String,
    source: Vec<usize>,
}

impl Paragraph {
    fn new(content: &str, base: usize, case: TextCase) -> Self {
        let mut display = String::with_capacity(content.len());
        let mut source = Vec::with_capacity(content.len() + 1);
        for (offset, ch) in content.char_indices() {
            let start = display.len();
            match case {
                TextCase::Original => display.push(ch),
                TextCase::Upper => display.extend(ch.to_uppercase()),
            }
            source.extend(std::iter::repeat_n(base + offset, display.len() - start));
        }
        source.push(base + content.len());
        Self { display, source }
    }
}

fn run(
    content: &str,
    style: &TextStyle,
    wrap: Option<f32>,
    box_width: f32,
    fixed_height: Option<f32>,
    font: &FontData,
) -> Result<TextLayout, LayoutError> {
    let _span = crate::perf::span("text_layout");
    let face = rustybuzz::Face::from_slice(&font.bytes, font.index).ok_or(LayoutError)?;
    let metrics = FontMetrics::of(&face);
    let scale = style.size / metrics.units_per_em;
    let ascent = metrics.ascender * scale;
    let descent = -metrics.descender * scale;
    let line_height = match style.line_height {
        LineHeight::Auto => ascent + descent + metrics.line_gap * scale,
        LineHeight::Percent(percent) => style.size * percent / 100.,
    };
    let baseline = (line_height - (ascent + descent)) / 2. + ascent;
    let letter_spacing = style.size * style.letter_spacing / 100.;

    let mut lines = Vec::new();
    let mut missing_glyphs = 0;
    let mut top = 0.;
    let mut base = 0;
    let paragraphs: Vec<&str> = content.split('\n').collect();
    for (paragraph_ix, text) in paragraphs.iter().enumerate() {
        if paragraph_ix > 0 {
            top += style.paragraph_spacing;
        }
        let paragraph = Paragraph::new(text, base, style.case);
        let (glyphs, clusters, missing) = shape(&face, &paragraph.display, scale, letter_spacing);
        missing_glyphs += missing;
        let breaks = break_lines(&paragraph.display, &clusters, wrap);
        let count = breaks.len();
        for (line_ix, cluster_range) in breaks.into_iter().enumerate() {
            let is_last = line_ix + 1 == count;
            let end = if is_last {
                paragraph.display.len()
            } else {
                clusters[cluster_range.end].start
            };
            let line_clusters = &clusters[cluster_range];
            let placed = place_line(
                line_clusters,
                &glyphs,
                style.align,
                box_width,
                wrap.is_some() && !is_last,
            );
            let start = line_clusters.first().map_or(end, |cluster| cluster.start);
            let mut stops: Vec<(usize, f32)> = Vec::with_capacity(line_clusters.len() + 1);
            for (cluster, x) in line_clusters.iter().zip(&placed.cluster_x) {
                push_stop(&mut stops, paragraph.source[cluster.start], *x);
            }
            push_stop(&mut stops, paragraph.source[end], placed.end_x);
            lines.push(Line {
                range: paragraph.source[start]..paragraph.source[end],
                top,
                height: line_height,
                baseline: top + baseline,
                left: placed.left,
                right: placed.right,
                glyphs: placed.glyphs,
                ends_paragraph: is_last,
                stops,
            });
            top += line_height;
        }
        base += text.len() + 1;
    }

    let content_height = top;
    let content_width = lines
        .iter()
        .map(|line| line.right - line.left)
        .fold(0., f32::max);
    let box_height = fixed_height.unwrap_or(content_height);
    let shift = match (fixed_height, style.vertical_align) {
        (None, _) | (Some(_), VAlign::Top) => 0.,
        (Some(height), VAlign::Middle) => (height - content_height) / 2.,
        (Some(height), VAlign::Bottom) => height - content_height,
    };
    for line in &mut lines {
        line.top += shift;
        line.baseline += shift;
    }

    let decorations = Decorations {
        underline_offset: metrics
            .underline_position
            .map_or(style.size * 0.1, |position| -position * scale),
        underline_thickness: metrics
            .underline_thickness
            .map_or(style.size / 14., |thickness| thickness * scale),
        strikeout_offset: metrics
            .strikeout_position
            .map_or(-style.size * 0.3, |position| -position * scale),
        strikeout_thickness: metrics
            .strikeout_thickness
            .map_or(style.size / 14., |thickness| thickness * scale),
    };

    Ok(TextLayout {
        lines,
        content_width,
        content_height,
        box_height,
        font_size: style.size,
        decorations,
        missing_glyphs,
    })
}

fn push_stop(stops: &mut Vec<(usize, f32)>, index: usize, x: f32) {
    if stops.last().is_none_or(|last| last.0 < index) {
        stops.push((index, x));
    }
}

/// Shapes one paragraph and groups its glyphs into clusters.
fn shape(
    face: &rustybuzz::Face,
    text: &str,
    scale: f32,
    letter_spacing: f32,
) -> (Vec<ShapedGlyph>, Vec<Cluster>, usize) {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    let output = rustybuzz::shape(face, &[], buffer);

    let mut glyphs = Vec::with_capacity(output.len());
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut missing = 0;
    for (ix, (info, position)) in output
        .glyph_infos()
        .iter()
        .zip(output.glyph_positions())
        .enumerate()
    {
        let start = info.cluster as usize;
        let ch = text[start..].chars().next().unwrap_or(' ');
        if info.glyph_id == 0 && !ch.is_whitespace() && !ch.is_control() {
            missing += 1;
        }
        let advance = position.x_advance as f32 * scale;
        glyphs.push(ShapedGlyph {
            id: info.glyph_id as u16,
            x_offset: position.x_offset as f32 * scale,
            y_offset: -position.y_offset as f32 * scale,
            advance,
        });
        match clusters.last_mut() {
            Some(cluster) if cluster.start == start => {
                cluster.glyphs.end = ix + 1;
                cluster.advance += advance;
            }
            _ => clusters.push(Cluster {
                start,
                glyphs: ix..ix + 1,
                advance: advance + letter_spacing,
                whitespace: ch.is_whitespace(),
            }),
        }
    }
    (glyphs, clusters, missing)
}

/// Splits a paragraph's clusters into lines no wider than `wrap`, breaking at
/// Unicode line break opportunities (UAX #14), or inside a word too long to
/// fit on a line of its own. Returns the cluster range of each line.
fn break_lines(text: &str, clusters: &[Cluster], wrap: Option<f32>) -> Vec<Range<usize>> {
    let Some(width) = wrap else {
        // One line holding every cluster.
        return std::iter::once(0..clusters.len()).collect();
    };
    let mut lines = Vec::new();
    let mut line_start = 0;
    // Width of the clusters on the current line, trailing spaces included.
    let mut line_width = 0.;
    let mut segment_start = 0;
    for (opportunity, _) in linebreaks(text) {
        let segment_end = clusters[segment_start..]
            .iter()
            .position(|cluster| cluster.start >= opportunity)
            .map_or(clusters.len(), |offset| segment_start + offset);
        let segment = &clusters[segment_start..segment_end];
        let total: f32 = segment.iter().map(|cluster| cluster.advance).sum();
        let trailing: f32 = segment
            .iter()
            .rev()
            .take_while(|cluster| cluster.whitespace)
            .map(|cluster| cluster.advance)
            .sum();
        let visible = total - trailing;

        if segment_start > line_start && line_width + visible > width + EPSILON {
            lines.push(line_start..segment_start);
            line_start = segment_start;
            line_width = 0.;
        }
        if segment_start == line_start && visible > width + EPSILON {
            // A word wider than the box: break between its clusters.
            for (offset, cluster) in segment.iter().enumerate() {
                let ix = segment_start + offset;
                if ix > line_start
                    && !cluster.whitespace
                    && line_width + cluster.advance > width + EPSILON
                {
                    lines.push(line_start..ix);
                    line_start = ix;
                    line_width = 0.;
                }
                line_width += cluster.advance;
            }
        } else {
            line_width += total;
        }
        segment_start = segment_end;
    }
    lines.push(line_start..clusters.len());
    lines
}

struct PlacedLine {
    glyphs: Vec<PositionedGlyph>,
    /// X where each cluster starts.
    cluster_x: Vec<f32>,
    end_x: f32,
    left: f32,
    right: f32,
}

fn place_line(
    clusters: &[Cluster],
    glyphs: &[ShapedGlyph],
    align: HAlign,
    box_width: f32,
    justifiable: bool,
) -> PlacedLine {
    let trailing = clusters
        .iter()
        .rev()
        .take_while(|cluster| cluster.whitespace)
        .count();
    let visible = &clusters[..clusters.len() - trailing];
    let visible_width: f32 = visible.iter().map(|cluster| cluster.advance).sum();
    let free = box_width - visible_width;
    let (start_x, gap) = match align {
        HAlign::Left => (0., 0.),
        HAlign::Center => (free / 2., 0.),
        HAlign::Right => (free, 0.),
        HAlign::Justify => {
            let spaces = visible.iter().filter(|cluster| cluster.whitespace).count();
            if justifiable && spaces > 0 && free > 0. {
                (0., free / spaces as f32)
            } else {
                (0., 0.)
            }
        }
    };

    let mut placed = Vec::new();
    let mut cluster_x = Vec::with_capacity(clusters.len());
    let mut x = start_x;
    let mut right = start_x;
    for (ix, cluster) in clusters.iter().enumerate() {
        cluster_x.push(x);
        let mut pen = x;
        for glyph in &glyphs[cluster.glyphs.clone()] {
            placed.push(PositionedGlyph {
                id: glyph.id,
                x: pen + glyph.x_offset,
                y: glyph.y_offset,
            });
            pen += glyph.advance;
        }
        x += cluster.advance;
        if cluster.whitespace && ix < visible.len() {
            x += gap;
        }
        if ix < visible.len() {
            right = x;
        }
    }
    PlacedLine {
        glyphs: placed,
        cluster_x,
        end_x: x,
        left: start_x,
        right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::inter_regular;

    fn text(content: &str, sizing: TextSizing) -> TextElement {
        TextElement {
            content: content.into(),
            style: TextStyle::default(),
            sizing,
        }
    }

    fn frame(width: f32, height: f32) -> Frame {
        Frame {
            width,
            height,
            ..Frame::default()
        }
    }

    fn lines(layout: &TextLayout, content: &str) -> Vec<String> {
        layout
            .lines
            .iter()
            .map(|line| content[line.range.clone()].to_string())
            .collect()
    }

    fn width_of(content: &str) -> f32 {
        measure(&text(content, TextSizing::AutoWidth), 0., &inter_regular())
            .unwrap()
            .0
    }

    #[test]
    fn auto_width_never_wraps() {
        let content = "Activation grew faster than signups";
        let element = text(content, TextSizing::AutoWidth);
        let layout = layout(&element, &frame(10., 10.), &inter_regular()).unwrap();
        assert_eq!(lines(&layout, content), [content]);
    }

    #[test]
    fn wraps_at_spaces_and_keeps_trailing_spaces_on_the_line() {
        let content = "Activation grew faster than signups";
        let width = width_of("Activation grew faster") + 1.;
        let element = text(content, TextSizing::AutoHeight);
        let layout = layout(&element, &frame(width, 0.), &inter_regular()).unwrap();
        assert_eq!(
            lines(&layout, content),
            ["Activation grew faster ", "than signups"]
        );
        assert!(layout.content_width <= width);
        let line_height = layout.lines[0].height;
        assert!((layout.content_height - 2. * line_height).abs() < 0.01);
    }

    #[test]
    fn breaks_after_hyphens() {
        let content = "state-of-the-art";
        let width = width_of("state-of-") + 1.;
        let element = text(content, TextSizing::AutoHeight);
        let layout = layout(&element, &frame(width, 0.), &inter_regular()).unwrap();
        assert_eq!(lines(&layout, content), ["state-of-", "the-art"]);
    }

    #[test]
    fn a_word_wider_than_the_box_breaks_between_letters() {
        let content = "Supercalifragilistic";
        let element = text(content, TextSizing::AutoHeight);
        let width = width_of("Super") + 1.;
        let layout = layout(&element, &frame(width, 0.), &inter_regular()).unwrap();
        let shown = lines(&layout, content);
        assert!(shown.len() > 2, "{shown:?}");
        assert_eq!(shown.concat(), content);
        assert!(layout.content_width <= width);
    }

    #[test]
    fn newlines_start_paragraphs_with_spacing() {
        let content = "One\n\nThree";
        let mut element = text(content, TextSizing::AutoWidth);
        let plain = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert_eq!(lines(&plain, content), ["One", "", "Three"]);
        element.style.paragraph_spacing = 10.;
        let spaced = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert!((spaced.content_height - plain.content_height - 20.).abs() < 0.01);
    }

    #[test]
    fn line_height_auto_follows_the_font_and_percent_the_size() {
        let mut element = text("Hi", TextSizing::AutoWidth);
        let auto = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        let percent = FontMetrics::read(&inter_regular())
            .unwrap()
            .auto_line_height_percent();
        assert!((auto.lines[0].height - 32. * percent / 100.).abs() < 0.01);
        element.style.line_height = LineHeight::Percent(150.);
        let fixed = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert!((fixed.lines[0].height - 48.).abs() < 0.01);
    }

    #[test]
    fn letter_spacing_widens_every_character() {
        let mut element = text("abcd", TextSizing::AutoWidth);
        let plain = measure(&element, 0., &inter_regular()).unwrap().0;
        element.style.letter_spacing = 10.;
        let spaced = measure(&element, 0., &inter_regular()).unwrap().0;
        assert!((spaced - plain - 4. * 3.2).abs() < 0.01);
    }

    #[test]
    fn alignment_places_lines_in_the_box() {
        let content = "Hi";
        let mut element = text(content, TextSizing::Fixed);
        let width = width_of(content);
        for (align, left) in [
            (HAlign::Left, 0.),
            (HAlign::Center, (200. - width) / 2.),
            (HAlign::Right, 200. - width),
        ] {
            element.style.align = align;
            let layout = layout(&element, &frame(200., 100.), &inter_regular()).unwrap();
            assert!((layout.lines[0].left - left).abs() < 0.01, "{align:?}");
        }
    }

    #[test]
    fn justify_fills_every_line_but_the_last() {
        let content = "Activation grew faster than signups";
        let width = width_of("Activation grew faster") + 30.;
        let mut element = text(content, TextSizing::AutoHeight);
        element.style.align = HAlign::Justify;
        let layout = layout(&element, &frame(width, 0.), &inter_regular()).unwrap();
        assert_eq!(layout.lines.len(), 2);
        assert!((layout.lines[0].right - width).abs() < 0.01);
        assert!(layout.lines[1].right < width - 1.);
    }

    #[test]
    fn fixed_boxes_report_overflow_and_align_vertically() {
        let content = "One\nTwo\nThree";
        let mut element = text(content, TextSizing::Fixed);
        let short = layout(&element, &frame(300., 40.), &inter_regular()).unwrap();
        assert!(short.overflow() > 0.);
        assert!((short.overflow() - (short.content_height - 40.)).abs() < 0.01);
        let tall = layout(&element, &frame(300., 400.), &inter_regular()).unwrap();
        assert_eq!(tall.overflow(), 0.);

        element.style.vertical_align = VAlign::Bottom;
        let bottom = layout(&element, &frame(300., 400.), &inter_regular()).unwrap();
        let last = bottom.lines.last().unwrap();
        assert!((last.top + last.height - 400.).abs() < 0.01);
    }

    #[test]
    fn uppercase_keeps_indices_in_the_original_text() {
        let content = "straße";
        let mut element = text(content, TextSizing::AutoWidth);
        element.style.case = TextCase::Upper;
        let layout = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert_eq!(layout.lines[0].range, 0..content.len());
        assert_eq!(layout.index_at(10_000., 0.), content.len());
        assert!(
            layout.lines[0].glyphs.len() > content.chars().count(),
            "ß became SS"
        );
    }

    #[test]
    fn counts_characters_without_a_glyph() {
        let element = text("a\u{0E01}b", TextSizing::AutoWidth);
        let layout = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert_eq!(layout.missing_glyphs, 1);
    }

    #[test]
    fn caret_and_hit_testing_agree() {
        let content = "Hello world\nSecond";
        let element = text(content, TextSizing::AutoWidth);
        let layout = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        for index in [0, 3, 5, 11, 12, 15, content.len()] {
            let caret = layout.caret(index);
            let hit = layout.index_at(caret.left + 0.1, (caret.top + caret.bottom) / 2.);
            assert_eq!(hit, index);
        }
        assert_eq!(layout.line_of(12), 1);
        assert!(layout.caret(11).top < layout.caret(12).top);
    }

    #[test]
    fn caret_at_a_wrap_goes_to_the_next_line() {
        let content = "Activation grew faster than signups";
        let width = width_of("Activation grew faster") + 1.;
        let element = text(content, TextSizing::AutoHeight);
        let layout = layout(&element, &frame(width, 0.), &inter_regular()).unwrap();
        let seam = layout.lines[1].range.start;
        assert_eq!(layout.line_of(seam), 1);
        assert_eq!(layout.caret(seam).left, layout.lines[1].left);
    }

    #[test]
    fn selection_spans_lines() {
        let content = "One\nTwo";
        let element = text(content, TextSizing::AutoWidth);
        let layout = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        let rects = layout.selection_rects(1..6);
        assert_eq!(rects.len(), 2);
        assert!(
            rects[0].right > layout.lines[0].right,
            "includes the line break"
        );
        assert!(layout.selection_rects(2..2).is_empty());
    }

    #[test]
    fn empty_text_has_one_line() {
        let element = text("", TextSizing::AutoWidth);
        let layout = layout(&element, &frame(0., 0.), &inter_regular()).unwrap();
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.content_width, 0.);
        assert!(layout.content_height > 0.);
        assert_eq!(layout.caret(0).left, 0.);
    }
}
