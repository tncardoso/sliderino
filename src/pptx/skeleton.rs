//! The parts every deck has: the slide master, one blank layout, the theme,
//! the properties and the document metadata. Sliderino slides set every
//! color, font and size explicitly, so these parts only give neutral
//! defaults.

use super::xml::Xml;
use super::{A_NS, P_NS, R_NS};

/// Id of the slide master in `p:sldMasterIdLst`. Master and layout ids
/// share one range that starts at 2^31.
pub const MASTER_ID: u32 = 2_147_483_648;
const LAYOUT_ID: u32 = MASTER_ID + 1;

/// A group shape tree with nothing in it, as layouts have.
fn empty_tree(xml: &mut Xml, name: &str) {
    xml.start("p:cSld")
        .attr("name", name)
        .start("p:spTree")
        .start("p:nvGrpSpPr")
        .empty("p:cNvPr", &[("id", &1), ("name", &"")])
        .empty("p:cNvGrpSpPr", &[])
        .empty("p:nvPr", &[])
        .end();
    group_properties(xml);
    xml.end().end();
}

/// The `p:grpSpPr` of a shape tree: the identity transform.
pub fn group_properties(xml: &mut Xml) {
    xml.start("p:grpSpPr")
        .start("a:xfrm")
        .empty("a:off", &[("x", &0), ("y", &0)])
        .empty("a:ext", &[("cx", &0), ("cy", &0)])
        .empty("a:chOff", &[("x", &0), ("y", &0)])
        .empty("a:chExt", &[("cx", &0), ("cy", &0)])
        .end()
        .end();
}

pub fn slide_master(layout_rel: &str) -> String {
    let mut xml = Xml::new();
    xml.start("p:sldMaster")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS);
    xml.start("p:cSld")
        .start("p:bg")
        .start("p:bgPr")
        .start("a:solidFill")
        .empty("a:srgbClr", &[("val", &"FFFFFF")])
        .end()
        .empty("a:effectLst", &[])
        .end()
        .end()
        .start("p:spTree")
        .start("p:nvGrpSpPr")
        .empty("p:cNvPr", &[("id", &1), ("name", &"")])
        .empty("p:cNvGrpSpPr", &[])
        .empty("p:nvPr", &[])
        .end();
    group_properties(&mut xml);
    xml.end().end();
    xml.empty(
        "p:clrMap",
        &[
            ("bg1", &"lt1"),
            ("tx1", &"dk1"),
            ("bg2", &"lt2"),
            ("tx2", &"dk2"),
            ("accent1", &"accent1"),
            ("accent2", &"accent2"),
            ("accent3", &"accent3"),
            ("accent4", &"accent4"),
            ("accent5", &"accent5"),
            ("accent6", &"accent6"),
            ("hlink", &"hlink"),
            ("folHlink", &"folHlink"),
        ],
    );
    xml.start("p:sldLayoutIdLst")
        .empty(
            "p:sldLayoutId",
            &[("id", &LAYOUT_ID), ("r:id", &layout_rel)],
        )
        .end();
    xml.start("p:txStyles")
        .empty("p:titleStyle", &[])
        .empty("p:bodyStyle", &[])
        .empty("p:otherStyle", &[])
        .end();
    xml.end();
    xml.finish()
}

pub fn slide_layout() -> String {
    let mut xml = Xml::new();
    xml.start("p:sldLayout")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS)
        .attr("type", "blank")
        .attr("preserve", 1);
    empty_tree(&mut xml, "Blank");
    xml.start("p:clrMapOvr")
        .empty("a:masterClrMapping", &[])
        .end();
    xml.end();
    xml.finish()
}

/// The theme: Office colors, Arial as the theme font, and the three
/// required format styles of each kind.
pub fn theme() -> String {
    let mut xml = Xml::new();
    xml.start("a:theme")
        .attr("xmlns:a", A_NS)
        .attr("name", "Sliderino");
    xml.start("a:themeElements");

    xml.start("a:clrScheme").attr("name", "Sliderino");
    for (name, color) in [
        ("a:dk1", "000000"),
        ("a:lt1", "FFFFFF"),
        ("a:dk2", "44546A"),
        ("a:lt2", "E7E6E6"),
        ("a:accent1", "4472C4"),
        ("a:accent2", "ED7D31"),
        ("a:accent3", "A5A5A5"),
        ("a:accent4", "FFC000"),
        ("a:accent5", "5B9BD5"),
        ("a:accent6", "70AD47"),
        ("a:hlink", "0563C1"),
        ("a:folHlink", "954F72"),
    ] {
        xml.start(name).empty("a:srgbClr", &[("val", &color)]).end();
    }
    xml.end();

    xml.start("a:fontScheme").attr("name", "Sliderino");
    for name in ["a:majorFont", "a:minorFont"] {
        xml.start(name)
            .empty("a:latin", &[("typeface", &"Arial")])
            .empty("a:ea", &[("typeface", &"")])
            .empty("a:cs", &[("typeface", &"")])
            .end();
    }
    xml.end();

    xml.start("a:fmtScheme").attr("name", "Sliderino");
    xml.start("a:fillStyleLst");
    for _ in 0..3 {
        xml.start("a:solidFill")
            .empty("a:schemeClr", &[("val", &"phClr")])
            .end();
    }
    xml.end();
    xml.start("a:lnStyleLst");
    for width in [6350, 12700, 19050] {
        xml.start("a:ln")
            .attr("w", width)
            .start("a:solidFill")
            .empty("a:schemeClr", &[("val", &"phClr")])
            .end()
            .end();
    }
    xml.end();
    xml.start("a:effectStyleLst");
    for _ in 0..3 {
        xml.start("a:effectStyle").empty("a:effectLst", &[]).end();
    }
    xml.end();
    xml.start("a:bgFillStyleLst");
    for _ in 0..3 {
        xml.start("a:solidFill")
            .empty("a:schemeClr", &[("val", &"phClr")])
            .end();
    }
    xml.end();
    xml.end();

    xml.end().end();
    xml.finish()
}

pub fn presentation_properties() -> String {
    let mut xml = Xml::new();
    xml.start("p:presentationPr")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS)
        .end();
    xml.finish()
}

pub fn view_properties() -> String {
    let mut xml = Xml::new();
    xml.start("p:viewPr")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS)
        .end();
    xml.finish()
}

/// No table styles: tables set their fills and borders on each cell.
pub fn table_styles() -> String {
    let mut xml = Xml::new();
    xml.start("a:tblStyleLst")
        .attr("xmlns:a", A_NS)
        .attr("def", super::NO_TABLE_STYLE)
        .end();
    xml.finish()
}

pub fn core_properties() -> String {
    let mut xml = Xml::new();
    xml.start("cp:coreProperties")
        .attr(
            "xmlns:cp",
            "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
        )
        .attr("xmlns:dc", "http://purl.org/dc/elements/1.1/")
        .attr("xmlns:dcterms", "http://purl.org/dc/terms/")
        .attr("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance")
        .start("dc:creator")
        .text("Sliderino")
        .end()
        .end();
    xml.finish()
}

pub fn extended_properties(slides: usize) -> String {
    let mut xml = Xml::new();
    xml.start("Properties")
        .attr(
            "xmlns",
            "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties",
        )
        .start("Application")
        .text("Sliderino")
        .end()
        .start("Slides")
        .text(&slides.to_string())
        .end()
        .end();
    xml.finish()
}
