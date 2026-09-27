//! Tables: an editable `a:tbl` with the columns, rows, merges, fills and
//! borders of the Sliderino layout.
//!
//! Cell texts keep the lines of the layout, as text boxes do. The top and
//! bottom margins take the baseline shift of `text.rs`. Sliderino sizes a
//! column to its widest line; a viewer wraps a line that does not fit, so
//! the margin on the side where the lines end is smaller by [`SLACK_EM`] of
//! the font size, at most the padding.

use crate::document::{Fill, HAlign, Stroke, TextElement};
use crate::table::{TableElement, TableLayout};

use super::shapes;
use super::text;
use super::units::emu;
use super::xml::Xml;

/// Room left at the end of each line of a cell, as a fraction of the font
/// size, so that a viewer that measures the text a little wider does not
/// wrap it. LibreOffice wraps a line of a cell exactly as wide as its text.
pub const SLACK_EM: f32 = 0.25;

pub const TABLE_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";

/// What a cell draws, worked out before the XML.
pub struct CellSpec<'a> {
    pub text: &'a TextElement,
    pub layout: &'a crate::text_layout::TextLayout,
    pub font: &'a crate::document::FontData,
    pub fill: &'a Fill,
    /// Left, right, top and bottom borders; `None` for no line.
    pub borders: [Option<Stroke>; 4],
    /// Margins left, right, top, bottom in slide units.
    pub margins: [f32; 4],
}

/// The margins of a cell: the padding, less the slack where the lines end,
/// with the baseline shift moved from the top to the bottom margin.
pub fn margins(padding: f32, text: &TextElement, shift: f32) -> [f32; 4] {
    let slack = (SLACK_EM * text.style.size).min(padding);
    let (left, right) = match text.style.align {
        HAlign::Left | HAlign::Justify => (padding, padding - slack),
        HAlign::Right => (padding - slack, padding),
        HAlign::Center => (padding - slack / 2., padding - slack / 2.),
    };
    [
        left.max(0.),
        right.max(0.),
        (padding - shift).max(0.),
        (padding + shift).max(0.),
    ]
}

/// The four borders of the cell at `row`, `column` with `span`: left,
/// right, top and bottom. A merged cell takes the stroke of the first edge
/// of each side.
pub fn borders(
    table: &TableElement,
    owners: &[Vec<(usize, usize)>],
    row: usize,
    column: usize,
) -> [Option<Stroke>; 4] {
    let span = table.span_of(row, column);
    [
        table.edge_stroke(owners, false, column, row),
        table.edge_stroke(owners, false, column + span.columns, row),
        table.edge_stroke(owners, true, row, column),
        table.edge_stroke(owners, true, row + span.rows, column),
    ]
}

/// `a:graphic` holding the table. `cell` writes the fill of a cell that
/// needs relationships (images) and gives `true` when it did.
pub fn graphic(
    xml: &mut Xml,
    table: &TableElement,
    layout: &TableLayout,
    cells: &[CellSpec],
    opacity: f32,
    mut picture: impl FnMut(&mut Xml, &Fill, (f32, f32)) -> bool,
) {
    xml.start("a:graphic")
        .start("a:graphicData")
        .attr("uri", TABLE_URI)
        .start("a:tbl");
    xml.start("a:tblPr")
        .attr("firstRow", 0)
        .attr("bandRow", 0)
        .start("a:tableStyleId")
        .text(super::NO_TABLE_STYLE)
        .end()
        .end();
    xml.start("a:tblGrid");
    for width in &layout.columns {
        xml.empty("a:gridCol", &[("w", &emu(*width))]);
    }
    xml.end();
    let owners = table.owners();
    let mut specs = layout.cells.iter().zip(cells);
    let mut next = specs.next();
    for (row, height) in layout.rows.iter().enumerate() {
        xml.start("a:tr").attr("h", emu(*height));
        for (column, &owner) in owners[row].iter().enumerate() {
            xml.start("a:tc");
            if owner != (row, column) {
                // A covered cell of a merge.
                if owner.1 < column {
                    xml.attr("hMerge", 1);
                }
                if owner.0 < row {
                    xml.attr("vMerge", 1);
                }
                xml.start("a:txBody")
                    .empty("a:bodyPr", &[])
                    .empty("a:lstStyle", &[])
                    .start("a:p")
                    .empty("a:endParaRPr", &[("lang", &"en-US")])
                    .end()
                    .end()
                    .empty("a:tcPr", &[])
                    .end();
                continue;
            }
            let span = table.span_of(row, column);
            if span.columns > 1 {
                xml.attr("gridSpan", span.columns);
            }
            if span.rows > 1 {
                xml.attr("rowSpan", span.rows);
            }
            let Some((cell_layout, spec)) =
                next.filter(|(cell, _)| (cell.row, cell.column) == (row, column))
            else {
                xml.end();
                continue;
            };
            next = specs.next();
            xml.start("a:txBody")
                .empty("a:bodyPr", &[])
                .empty("a:lstStyle", &[]);
            text::paragraphs(
                xml,
                &spec.text.content,
                &spec.text.style,
                spec.layout,
                spec.font,
                opacity,
            );
            xml.end();
            let [left, right, top, bottom] = spec.margins.map(emu);
            xml.start("a:tcPr")
                .attr("marL", left)
                .attr("marR", right)
                .attr("marT", top)
                .attr("marB", bottom)
                .attr(
                    "anchor",
                    match spec.text.style.vertical_align {
                        crate::document::VAlign::Top => "t",
                        crate::document::VAlign::Middle => "ctr",
                        crate::document::VAlign::Bottom => "b",
                    },
                );
            for (name, stroke) in ["a:lnL", "a:lnR", "a:lnT", "a:lnB"]
                .into_iter()
                .zip(spec.borders)
            {
                border(xml, name, stroke.as_ref(), opacity);
            }
            let size = (cell_layout.rect.width, cell_layout.rect.height);
            if !picture(xml, spec.fill, size) {
                shapes::fill(xml, spec.fill, opacity, size.0, size.1);
            }
            xml.end().end();
        }
        xml.end();
    }
    xml.end().end().end();
}

/// One border of a cell (`a:lnL` and the others).
fn border(xml: &mut Xml, name: &'static str, stroke: Option<&Stroke>, opacity: f32) {
    let Some(stroke) = stroke else {
        xml.start(name).attr("w", 0).empty("a:noFill", &[]).end();
        return;
    };
    xml.start(name)
        .attr("w", emu(stroke.width))
        .attr("cap", "flat")
        .attr("cmpd", "sng")
        .attr("algn", "ctr");
    shapes::solid(xml, stroke.color, stroke.opacity * opacity);
    match shapes::dash_lengths(stroke.dash) {
        None => {
            xml.empty("a:prstDash", &[("val", &"solid")]);
        }
        Some((dash, space)) => {
            xml.start("a:custDash")
                .empty("a:ds", &[("d", &dash), ("sp", &space)])
                .end();
        }
    }
    xml.empty("a:miter", &[("lim", &800_000)]).end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{FontFace, TextSizing, TextStyle};

    fn text(align: HAlign) -> TextElement {
        TextElement {
            content: String::new(),
            style: TextStyle {
                font: FontFace::new("Inter", 400, false),
                align,
                ..TextStyle::default()
            },
            sizing: TextSizing::Fixed,
        }
    }

    #[test]
    fn the_slack_goes_where_the_lines_end() {
        // A 32 unit font leaves 8 units of slack.
        assert_eq!(margins(12., &text(HAlign::Left), 0.), [12., 4., 12., 12.]);
        assert_eq!(margins(12., &text(HAlign::Right), 0.), [4., 12., 12., 12.]);
        assert_eq!(margins(12., &text(HAlign::Center), 0.), [8., 8., 12., 12.]);
        assert_eq!(margins(2., &text(HAlign::Left), 0.), [2., 0., 2., 2.]);
    }

    #[test]
    fn the_baseline_shift_moves_from_the_top_to_the_bottom_margin() {
        assert_eq!(margins(12., &text(HAlign::Left), 2.)[2..], [10., 14.]);
        assert_eq!(margins(1., &text(HAlign::Left), 2.)[2..], [0., 3.]);
    }
}
