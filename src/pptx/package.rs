//! The Open Packaging Conventions container of a PPTX: parts, their
//! relationships and `[Content_Types].xml`, written to a zip archive.

use std::io::{Cursor, Write as _};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::xml::Xml;

pub const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Relationship types, without the common prefix of [`REL`].
pub mod rel {
    pub const OFFICE_DOCUMENT: &str = "officeDocument";
    pub const CORE: &str =
        "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
    pub const EXTENDED: &str = "extended-properties";
    pub const SLIDE_MASTER: &str = "slideMaster";
    pub const SLIDE_LAYOUT: &str = "slideLayout";
    pub const SLIDE: &str = "slide";
    pub const THEME: &str = "theme";
    pub const FONT: &str = "font";
    pub const PRES_PROPS: &str = "presProps";
    pub const VIEW_PROPS: &str = "viewProps";
    pub const TABLE_STYLES: &str = "tableStyles";
}

/// Content types of the parts.
pub mod content {
    pub const RELS: &str = "application/vnd.openxmlformats-package.relationships+xml";
    pub const XML: &str = "application/xml";
    pub const PRESENTATION: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
    pub const SLIDE: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
    pub const SLIDE_MASTER: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
    pub const SLIDE_LAYOUT: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
    pub const THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
    pub const PRES_PROPS: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml";
    pub const VIEW_PROPS: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml";
    pub const TABLE_STYLES: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml";
    pub const CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
    pub const EXTENDED: &str =
        "application/vnd.openxmlformats-officedocument.extended-properties+xml";
    pub const PNG: &str = "image/png";
    pub const JPEG: &str = "image/jpeg";
    pub const MP4: &str = "video/mp4";
    /// Embedded OpenType, as PowerPoint writes embedded fonts.
    pub const FONT_DATA: &str = "application/x-fontdata";
}

/// The relationships of one part (or of the package).
#[derive(Default)]
pub struct Rels {
    entries: Vec<Relationship>,
}

struct Relationship {
    id: String,
    kind: String,
    target: String,
}

impl Rels {
    /// Adds a relationship and gives its id. `kind` is a full URI or a
    /// name under [`REL`]. `target` is relative to the source part.
    pub fn add(&mut self, kind: &str, target: impl Into<String>) -> String {
        let id = format!("rId{}", self.entries.len() + 1);
        let kind = if kind.contains("://") {
            kind.to_string()
        } else {
            format!("{REL}/{kind}")
        };
        self.entries.push(Relationship {
            id: id.clone(),
            kind,
            target: target.into(),
        });
        id
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn to_xml(&self) -> String {
        let mut xml = Xml::new();
        xml.start("Relationships").attr("xmlns", RELS_NS);
        for entry in &self.entries {
            xml.start("Relationship")
                .attr("Id", &entry.id)
                .attr("Type", &entry.kind)
                .attr("Target", &entry.target)
                .end();
        }
        xml.end();
        xml.finish()
    }
}

/// The `.rels` part name of `part`: `ppt/slides/slide1.xml` gives
/// `ppt/slides/_rels/slide1.xml.rels`.
pub fn rels_name(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((folder, file)) => format!("{folder}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

struct Part {
    name: String,
    content_type: Option<&'static str>,
    bytes: Vec<u8>,
    compress: bool,
}

/// The parts of a package, in the order they are written.
#[derive(Default)]
pub struct Package {
    parts: Vec<Part>,
}

impl Package {
    /// Adds an XML part with its content type (an Override entry).
    pub fn xml(&mut self, name: impl Into<String>, content_type: &'static str, xml: String) {
        self.parts.push(Part {
            name: name.into(),
            content_type: Some(content_type),
            bytes: xml.into_bytes(),
            compress: true,
        });
    }

    /// Adds the relationships of `part`, when it has any.
    pub fn rels(&mut self, part: &str, rels: &Rels) {
        if rels.is_empty() {
            return;
        }
        self.parts.push(Part {
            name: rels_name(part),
            content_type: None,
            bytes: rels.to_xml().into_bytes(),
            compress: true,
        });
    }

    /// Adds a binary part. Its content type comes from a Default entry for
    /// its extension. Media is already compressed: it is stored.
    pub fn binary(&mut self, name: impl Into<String>, bytes: Vec<u8>, compress: bool) {
        self.parts.push(Part {
            name: name.into(),
            content_type: None,
            bytes,
            compress,
        });
    }

    fn content_types(&self) -> String {
        let mut xml = Xml::new();
        xml.start("Types")
            .attr(
                "xmlns",
                "http://schemas.openxmlformats.org/package/2006/content-types",
            )
            .empty(
                "Default",
                &[("Extension", &"rels"), ("ContentType", &content::RELS)],
            )
            .empty(
                "Default",
                &[("Extension", &"xml"), ("ContentType", &content::XML)],
            );
        let mut extensions: Vec<&str> = Vec::new();
        for part in &self.parts {
            if part.content_type.is_some() {
                continue;
            }
            let extension = part.name.rsplit('.').next().unwrap_or_default();
            if extension != "rels" && !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        for extension in extensions {
            let content_type = match extension {
                "png" => content::PNG,
                "jpeg" | "jpg" => content::JPEG,
                "mp4" => content::MP4,
                "fntdata" => content::FONT_DATA,
                _ => "application/octet-stream",
            };
            xml.empty(
                "Default",
                &[("Extension", &extension), ("ContentType", &content_type)],
            );
        }
        for part in &self.parts {
            if let Some(content_type) = part.content_type {
                let name = format!("/{}", part.name);
                xml.empty(
                    "Override",
                    &[("PartName", &name), ("ContentType", &content_type)],
                );
            }
        }
        xml.end();
        xml.finish()
    }

    /// The zip archive: `[Content_Types].xml` first, then the parts.
    pub fn finish(self) -> std::io::Result<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("[Content_Types].xml", deflated)?;
        zip.write_all(self.content_types().as_bytes())?;
        for part in &self.parts {
            let options = if part.compress { deflated } else { stored };
            let options = options.large_file(part.bytes.len() >= u32::MAX as usize);
            zip.start_file(&part.name, options)?;
            zip.write_all(&part.bytes)?;
        }
        Ok(zip.finish()?.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_live_next_to_their_part() {
        assert_eq!(
            rels_name("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
        assert_eq!(rels_name(""), "_rels/.rels");
    }

    #[test]
    fn relationship_ids_count_up() {
        let mut rels = Rels::default();
        assert_eq!(rels.add(rel::SLIDE_LAYOUT, "../x.xml"), "rId1");
        assert_eq!(rels.add(rel::THEME, "../t.xml"), "rId2");
        assert!(rels.to_xml().contains(&format!("{REL}/theme")));
    }

    #[test]
    fn content_types_come_first_with_one_default_per_extension() {
        let mut package = Package::default();
        package.xml("ppt/presentation.xml", content::PRESENTATION, "<x/>".into());
        package.binary("ppt/media/image1.png", vec![1], false);
        package.binary("ppt/media/image2.png", vec![2], false);
        let bytes = package.finish().unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(zip.by_index(0).unwrap().name(), "[Content_Types].xml");
        let mut types = String::new();
        std::io::Read::read_to_string(&mut zip.by_index(0).unwrap(), &mut types).unwrap();
        assert_eq!(types.matches(r#"Extension="png""#).count(), 1);
        assert!(types.contains(r#"PartName="/ppt/presentation.xml""#));
    }
}
