//! PPTX export: writes a presentation as an editable PowerPoint deck.
//!
//! Every element keeps its place, size and rotation. Texts keep the line
//! breaks of the Sliderino layout, so they wrap the same in PowerPoint.
//! `docs/pptx-export.md` gives the mapping and its limits.

mod fonts;
mod media;
mod package;
mod shapes;
mod skeleton;
mod slide;
pub(crate) mod table;
mod text;
pub mod units;
pub mod xml;

pub mod inspect;
pub mod parity;
pub mod visual;

use crate::document::{ElementId, Presentation, SlideId};
use package::{Package, Rels, content, rel};

pub const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const P_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

/// The table style id meaning "No Style, No Grid".
pub const NO_TABLE_STYLE: &str = "{2D5ABB26-0587-4C30-8999-92F81FD0307C}";

/// Id of the first slide in `p:sldIdLst`.
const FIRST_SLIDE_ID: u32 = 256;

/// Settings of an export. The defaults make the deck a person gets.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Adds thin lines on the text frames and baselines of the layout, to
    /// check where a viewer puts the text. For debugging only.
    pub guides: bool,
}

/// Something the deck shows differently than the editor, or leaves out.
/// A warning does not stop the export.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub slide: Option<SlideId>,
    pub element: Option<ElementId>,
    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(slide) = self.slide {
            write!(f, "slide {}: ", slide.0)?;
        }
        if let Some(element) = self.element {
            write!(f, "element {}: ", element.0)?;
        }
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(error) => write!(f, "cannot write the deck: {error}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(error: std::io::Error) -> Self {
        ExportError::Io(error)
    }
}

/// A written deck.
pub struct Export {
    pub bytes: Vec<u8>,
    pub warnings: Vec<Warning>,
}

/// Writes `presentation` as a PPTX file in memory.
pub fn export(presentation: &Presentation, options: &Options) -> Result<Export, ExportError> {
    let mut package = Package::default();
    let mut warnings = Vec::new();

    let mut root = Rels::default();
    root.add(rel::OFFICE_DOCUMENT, "ppt/presentation.xml");
    root.add(rel::CORE, "docProps/core.xml");
    root.add(rel::EXTENDED, "docProps/app.xml");
    package.rels("", &root);

    let mut presentation_rels = Rels::default();
    let master_rel = presentation_rels.add(rel::SLIDE_MASTER, "slideMasters/slideMaster1.xml");
    let mut slide_rels = Vec::new();
    let font_plan = fonts::FontPlan::new(presentation, &mut warnings);
    let mut media = media::Media::default();
    let mut slides = Vec::new();
    for (index, slide) in presentation.slides.iter().enumerate() {
        let name = format!("ppt/slides/slide{}.xml", index + 1);
        slide_rels
            .push(presentation_rels.add(rel::SLIDE, format!("slides/slide{}.xml", index + 1)));
        let written = slide::write(presentation, slide, options, &mut media, &mut warnings);
        slides.push((name, written));
    }
    presentation_rels.add(rel::THEME, "theme/theme1.xml");
    let embedded_fonts = font_plan.write(
        presentation,
        &mut package,
        &mut presentation_rels,
        &mut warnings,
    );
    presentation_rels.add(rel::PRES_PROPS, "presProps.xml");
    presentation_rels.add(rel::VIEW_PROPS, "viewProps.xml");
    presentation_rels.add(rel::TABLE_STYLES, "tableStyles.xml");

    package.xml(
        "ppt/presentation.xml",
        content::PRESENTATION,
        presentation_xml(
            presentation,
            &master_rel,
            &slide_rels,
            embedded_fonts.as_deref(),
        ),
    );
    package.rels("ppt/presentation.xml", &presentation_rels);

    for (name, written) in slides {
        package.xml(&name, content::SLIDE, written.xml);
        package.rels(&name, &written.rels);
    }

    let mut master_rels = Rels::default();
    let layout_rel = master_rels.add(rel::SLIDE_LAYOUT, "../slideLayouts/slideLayout1.xml");
    master_rels.add(rel::THEME, "../theme/theme1.xml");
    package.xml(
        "ppt/slideMasters/slideMaster1.xml",
        content::SLIDE_MASTER,
        skeleton::slide_master(&layout_rel),
    );
    package.rels("ppt/slideMasters/slideMaster1.xml", &master_rels);

    let mut layout_rels = Rels::default();
    layout_rels.add(rel::SLIDE_MASTER, "../slideMasters/slideMaster1.xml");
    package.xml(
        "ppt/slideLayouts/slideLayout1.xml",
        content::SLIDE_LAYOUT,
        skeleton::slide_layout(),
    );
    package.rels("ppt/slideLayouts/slideLayout1.xml", &layout_rels);

    package.xml("ppt/theme/theme1.xml", content::THEME, skeleton::theme());
    package.xml(
        "ppt/presProps.xml",
        content::PRES_PROPS,
        skeleton::presentation_properties(),
    );
    package.xml(
        "ppt/viewProps.xml",
        content::VIEW_PROPS,
        skeleton::view_properties(),
    );
    package.xml(
        "ppt/tableStyles.xml",
        content::TABLE_STYLES,
        skeleton::table_styles(),
    );
    package.xml(
        "docProps/core.xml",
        content::CORE,
        skeleton::core_properties(),
    );
    package.xml(
        "docProps/app.xml",
        content::EXTENDED,
        skeleton::extended_properties(presentation.slides.len()),
    );
    media.write(&mut package);

    Ok(Export {
        bytes: package.finish()?,
        warnings,
    })
}

/// Writes the deck to `path`. It is written next to it first, then
/// renamed, so a failed export leaves an old file as it was.
pub fn save(export: &Export, path: &std::path::Path) -> Result<(), ExportError> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = std::path::PathBuf::from(partial);
    let result =
        std::fs::write(&partial, &export.bytes).and_then(|()| std::fs::rename(&partial, path));
    if result.is_err() {
        std::fs::remove_file(&partial).ok();
    }
    Ok(result?)
}

fn presentation_xml(
    presentation: &Presentation,
    master_rel: &str,
    slide_rels: &[String],
    embedded_fonts: Option<&str>,
) -> String {
    let mut xml = xml::Xml::new();
    xml.start("p:presentation")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS)
        .attr_opt("embedTrueTypeFonts", embedded_fonts.map(|_| 1));
    xml.start("p:sldMasterIdLst")
        .empty(
            "p:sldMasterId",
            &[("id", &skeleton::MASTER_ID), ("r:id", &master_rel)],
        )
        .end();
    if !slide_rels.is_empty() {
        xml.start("p:sldIdLst");
        for (index, rel) in slide_rels.iter().enumerate() {
            let id = FIRST_SLIDE_ID + index as u32;
            xml.empty("p:sldId", &[("id", &id), ("r:id", rel)]);
        }
        xml.end();
    }
    let width = units::emu(presentation.size.width as f32);
    let height = units::emu(presentation.size.height as f32);
    xml.empty("p:sldSz", &[("cx", &width), ("cy", &height)])
        .empty("p:notesSz", &[("cx", &6_858_000), ("cy", &9_144_000)]);
    if let Some(fonts) = embedded_fonts {
        xml.start("p:embeddedFontLst").raw(fonts).end();
    }
    xml.end();
    xml.finish()
}

#[cfg(test)]
mod parity_tests;
