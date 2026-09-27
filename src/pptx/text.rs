//! Text bodies: the paragraphs of a text box or a table cell, from the
//! Sliderino layout.
//!
//! Each line of the layout ends with a line break (`a:br`) and the body
//! does not wrap, so PowerPoint breaks the lines where Sliderino does.
//! Justified paragraphs are the exception: PowerPoint does not justify a
//! line that ends with a break, so they wrap in the box.
//!
//! Lines have an exact height (`a:spcPts`). A viewer puts the baseline of
//! such a line [`VIEWER_DESCENT`] of the font size above its bottom, for
//! every font; Sliderino splits the extra height of the line above and
//! below the font ascent and descent. [`baseline_shift`] gives the
//! difference, and the box moves by it.

use crate::document::{FontData, HAlign, LineHeight, TextCase, TextStyle, VAlign};
use crate::text_layout::{FontMetrics, TextLayout};

use super::fonts::is_bold;
use super::shapes;
use super::units::centipoints;
use super::xml::Xml;

/// Distance from the bottom of a line of exact height to its baseline, as
/// a fraction of the font size. Measured in LibreOffice 26.8 with three
/// fonts of different metrics (see `docs/pptx-export.md`); not measured in
/// PowerPoint.
pub const VIEWER_DESCENT: f32 = 0.2;

/// How far down the viewer puts the first baseline, less how far down
/// Sliderino puts it, in slide units. Move the box up by this.
pub fn baseline_shift(style: &TextStyle, font: &FontData) -> f32 {
    let Ok(metrics) = FontMetrics::read(font) else {
        return 0.;
    };
    let scale = style.size / metrics.units_per_em;
    let ascent = metrics.ascender * scale;
    let descent = -metrics.descender * scale;
    let height = line_height(style, &metrics);
    let sliderino = (height - (ascent + descent)) / 2. + ascent;
    let viewer = height - VIEWER_DESCENT * style.size;
    viewer - sliderino
}

/// The height of a line in slide units.
fn line_height(style: &TextStyle, metrics: &FontMetrics) -> f32 {
    match style.line_height {
        LineHeight::Auto => style.size * metrics.auto_line_height_percent() / 100.,
        LineHeight::Percent(percent) => style.size * percent / 100.,
    }
}

/// `a:bodyPr`. `wrap` is true for a justified text.
pub fn body_properties(xml: &mut Xml, anchor: VAlign, wrap: bool, insets: [f32; 4]) {
    let [l, t, r, b] = insets.map(super::units::emu);
    xml.start("a:bodyPr")
        .attr("wrap", if wrap { "square" } else { "none" })
        .attr("lIns", l)
        .attr("tIns", t)
        .attr("rIns", r)
        .attr("bIns", b)
        .attr(
            "anchor",
            match anchor {
                VAlign::Top => "t",
                VAlign::Middle => "ctr",
                VAlign::Bottom => "b",
            },
        )
        .attr("rtlCol", 0)
        .empty("a:noAutofit", &[])
        .end();
}

/// The paragraphs of `content` as laid out in `layout`.
pub fn paragraphs(
    xml: &mut Xml,
    content: &str,
    style: &TextStyle,
    layout: &TextLayout,
    font: &FontData,
    opacity: f32,
) {
    let metrics = FontMetrics::read(font).ok();
    let height = metrics
        .as_ref()
        .map_or(style.size * 1.2, |metrics| line_height(style, metrics));
    let justify = style.align == HAlign::Justify;
    let paragraph_count = content.split('\n').count();
    let mut lines = layout.lines.iter();
    for index in 0..paragraph_count {
        let last = index + 1 == paragraph_count;
        xml.start("a:p").start("a:pPr").attr(
            "algn",
            match style.align {
                HAlign::Left => "l",
                HAlign::Center => "ctr",
                HAlign::Right => "r",
                HAlign::Justify => "just",
            },
        );
        xml.start("a:lnSpc")
            .empty(
                "a:spcPts",
                &[("val", &centipoints(height).clamp(0, 158_400))],
            )
            .end()
            .start("a:spcBef")
            .empty("a:spcPts", &[("val", &0)])
            .end()
            .start("a:spcAft")
            .empty(
                "a:spcPts",
                &[(
                    "val",
                    &if last {
                        0
                    } else {
                        centipoints(style.paragraph_spacing).clamp(0, 158_400)
                    },
                )],
            )
            .end()
            .empty("a:buNone", &[])
            .end();

        let mut first = true;
        let mut text = String::new();
        for line in lines.by_ref() {
            let shown = content[line.range.clone()].trim_end_matches([' ', '\t']);
            if justify {
                // The whole paragraph in one run; it wraps in the box.
                text.push_str(&content[line.range.clone()]);
            } else {
                if !first {
                    xml.start("a:br");
                    run_properties(xml, "a:rPr", style, opacity);
                    xml.end();
                }
                if !shown.is_empty() {
                    xml.start("a:r");
                    run_properties(xml, "a:rPr", style, opacity);
                    xml.start("a:t").text(shown).end().end();
                }
            }
            first = false;
            if line.ends_paragraph {
                break;
            }
        }
        if justify {
            let text = text.trim_end_matches([' ', '\t']);
            if !text.is_empty() {
                xml.start("a:r");
                run_properties(xml, "a:rPr", style, opacity);
                xml.start("a:t").text(text).end().end();
            }
        }
        run_properties(xml, "a:endParaRPr", style, opacity);
        xml.end();
    }
}

/// `a:rPr` (or `a:endParaRPr`) of a style.
pub fn run_properties(xml: &mut Xml, name: &'static str, style: &TextStyle, opacity: f32) {
    xml.start(name)
        .attr("lang", "en-US")
        .attr("sz", centipoints(style.size).clamp(100, 400_000))
        .attr("b", u8::from(is_bold(&style.font)))
        .attr("i", u8::from(style.font.italic))
        .attr("u", if style.underline { "sng" } else { "none" })
        .attr(
            "strike",
            if style.strikethrough {
                "sngStrike"
            } else {
                "noStrike"
            },
        )
        .attr_opt("cap", (style.case == TextCase::Upper).then_some("all"))
        .attr("spc", centipoints(style.size * style.letter_spacing / 100.))
        .attr("baseline", 0)
        .attr("dirty", 0);
    shapes::solid(xml, style.color, opacity);
    let family = style.font.family.as_str();
    xml.empty("a:latin", &[("typeface", &family)])
        .empty("a:ea", &[("typeface", &family)])
        .empty("a:cs", &[("typeface", &family)])
        .end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{FontFace, TextElement, TextSizing};

    fn inter() -> FontData {
        crate::document::tests::inter_regular()
    }

    fn body(content: &str, style: TextStyle, width: f32) -> String {
        let text = TextElement {
            content: content.to_string(),
            style,
            sizing: TextSizing::AutoHeight,
        };
        let frame = crate::document::Frame {
            x: 0.,
            y: 0.,
            width,
            height: 100.,
            rotation: 0.,
        };
        let layout = crate::text_layout::layout(&text, &frame, &inter()).unwrap();
        let mut xml = Xml::fragment();
        paragraphs(&mut xml, &text.content, &text.style, &layout, &inter(), 1.);
        xml.finish()
    }

    fn style() -> TextStyle {
        TextStyle {
            font: FontFace::new("Inter", 400, false),
            ..TextStyle::default()
        }
    }

    #[test]
    fn wrapped_lines_end_with_breaks_without_their_trailing_space() {
        let xml = body("one two three four", style(), 120.);
        assert!(xml.contains("<a:br>"), "{xml}");
        assert!(!xml.contains("<a:t>one </a:t>"), "{xml}");
        assert_eq!(xml.matches("<a:p>").count(), 1);
    }

    #[test]
    fn justified_paragraphs_stay_one_run() {
        let justified = TextStyle {
            align: HAlign::Justify,
            ..style()
        };
        let xml = body("one two three four\nfive", justified, 120.);
        assert!(!xml.contains("<a:br>"), "{xml}");
        assert!(xml.contains("<a:t>one two three four</a:t>"), "{xml}");
        assert_eq!(xml.matches("<a:p>").count(), 2);
    }

    #[test]
    fn exact_line_height_and_spacing_after_all_but_the_last_paragraph() {
        let spaced = TextStyle {
            size: 50.,
            line_height: LineHeight::Percent(120.),
            paragraph_spacing: 10.,
            ..style()
        };
        let xml = body("a\nb", spaced, 400.);
        // 60 units are 36 points; 10 units are 6 points.
        assert_eq!(
            xml.matches(r#"<a:lnSpc><a:spcPts val="3600"/></a:lnSpc>"#)
                .count(),
            2
        );
        assert!(xml.contains(r#"<a:spcAft><a:spcPts val="600"/></a:spcAft>"#));
        assert!(xml.contains(r#"<a:spcAft><a:spcPts val="0"/></a:spcAft>"#));
    }

    #[test]
    fn a_tight_line_moves_by_the_difference_of_the_descents() {
        // A line exactly as high as the font: Sliderino puts the baseline
        // one font descent above the bottom, the viewer a fifth of the size.
        let mut style = style();
        let metrics = FontMetrics::read(&inter()).unwrap();
        let descent = -metrics.descender / metrics.units_per_em;
        let ascent = metrics.ascender / metrics.units_per_em;
        style.line_height = LineHeight::Percent((ascent + descent) * 100.);
        let shift = baseline_shift(&style, &inter());
        assert!((shift - (descent - VIEWER_DESCENT) * style.size).abs() < 1e-3);
    }
}
