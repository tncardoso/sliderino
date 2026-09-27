//! The fonts of the deck: which face each text uses in PowerPoint, and the
//! embedded font parts.
//!
//! PowerPoint knows four faces of a family: regular, bold, italic and bold
//! italic. A face of weight 600 or more is bold, a lighter one regular.
//! Each slot embeds the face that the texts use most, under the family
//! name, so a SemiBold text shows the SemiBold face. When two faces want
//! one slot, the other texts show the chosen face, with a warning.
//!
//! Font parts are Embedded OpenType (EOT) files without compression, as
//! PowerPoint writes them. PowerPoint embeds only TrueType outlines: a CFF
//! font is not embedded, and neither is a font whose license restricts
//! embedding.

use std::collections::BTreeMap;

use crate::document::{ElementKind, FontFace, Presentation, TextStyle};

use super::Warning;
use super::package::{Package, Rels, rel};
use super::xml::Xml;

/// Bold slot: a weight of 600 or more.
pub fn is_bold(face: &FontFace) -> bool {
    face.weight >= 600
}

/// The slot of a face: 0 regular, 1 bold, 2 italic, 3 bold italic.
fn slot(face: &FontFace) -> usize {
    usize::from(is_bold(face)) + 2 * usize::from(face.italic)
}

const SLOT_NAMES: [&str; 4] = ["p:regular", "p:bold", "p:italic", "p:boldItalic"];

/// The face each slot of each family embeds.
#[derive(Default)]
pub struct FontPlan {
    families: BTreeMap<String, [Option<FontFace>; 4]>,
}

impl FontPlan {
    /// Counts the characters of each face in the visible texts and table
    /// cells, and gives each slot its most used face.
    pub fn new(presentation: &Presentation, warnings: &mut Vec<Warning>) -> FontPlan {
        let mut uses: BTreeMap<FontFace, usize> = BTreeMap::new();
        let mut count = |style: &TextStyle, content: &str| {
            *uses.entry(style.font.clone()).or_default() += content.chars().count().max(1);
        };
        for slide in &presentation.slides {
            for node in slide.walk().into_iter().filter(|node| !node.hidden) {
                match &node.element.kind {
                    ElementKind::Text(text) => count(&text.style, &text.content),
                    ElementKind::Table(table) => {
                        for (row, column) in table.anchors() {
                            count(
                                &table.style_of(row, column),
                                &table.rows[row][column].content,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut plan = FontPlan::default();
        let mut ranked: Vec<(&FontFace, &usize)> = uses.iter().collect();
        // Most used first; ties go to the face nearest the slot weight.
        ranked.sort_by_key(|(face, used)| {
            let target: i32 = if is_bold(face) { 700 } else { 400 };
            (
                std::cmp::Reverse(**used),
                (face.weight as i32 - target).abs(),
            )
        });
        for (face, _) in ranked {
            let slots = plan.families.entry(face.family.clone()).or_default();
            match &slots[slot(face)] {
                None => slots[slot(face)] = Some(face.clone()),
                Some(chosen) => warnings.push(Warning {
                    slide: None,
                    element: None,
                    message: format!(
                        "{} {} shows as {} {} in PowerPoint: a family has one {} face",
                        face.family,
                        face.style_name(),
                        chosen.family,
                        chosen.style_name(),
                        ["regular", "bold", "italic", "bold italic"][slot(face)],
                    ),
                }),
            }
        }
        plan
    }

    /// Writes the font parts and gives the `p:embeddedFontLst`, or `None`
    /// when no font is embedded.
    pub fn write(
        &self,
        presentation: &Presentation,
        package: &mut Package,
        rels: &mut Rels,
        warnings: &mut Vec<Warning>,
    ) -> Option<String> {
        let mut xml = Xml::fragment();
        let mut any = false;
        let mut number = 0;
        for (family, slots) in &self.families {
            let mut entries = Vec::new();
            for (index, face) in slots.iter().enumerate() {
                let Some(face) = face else { continue };
                let Some(data) = presentation.fonts.get(face) else {
                    continue;
                };
                let bold = index % 2 == 1;
                let italic = index >= 2;
                match embeddable(&data.bytes, data.index) {
                    Ok(()) => {}
                    Err(reason) => {
                        warnings.push(Warning {
                            slide: None,
                            element: None,
                            message: format!(
                                "{family} {} is not embedded: {reason}",
                                face.style_name()
                            ),
                        });
                        continue;
                    }
                }
                let Some(eot) =
                    crate::font_file::style_member(&data.bytes, data.index, family, bold, italic)
                        .and_then(|member| eot(&member))
                else {
                    warnings.push(Warning {
                        slide: None,
                        element: None,
                        message: format!(
                            "{family} {} is not embedded: the font file cannot be read",
                            face.style_name()
                        ),
                    });
                    continue;
                };
                number += 1;
                let name = format!("ppt/fonts/font{number}.fntdata");
                package.binary(&name, eot, true);
                entries.push((
                    index,
                    rels.add(rel::FONT, format!("fonts/font{number}.fntdata")),
                ));
            }
            if entries.is_empty() {
                continue;
            }
            any = true;
            xml.start("p:embeddedFont").empty(
                "p:font",
                &[("typeface", family), ("pitchFamily", &2), ("charset", &0)],
            );
            for (index, id) in entries {
                xml.empty(SLOT_NAMES[index], &[("r:id", &id)]);
            }
            xml.end();
        }
        any.then(|| xml.finish())
    }
}

/// Why face `index` of `bytes` cannot be embedded, if it cannot.
fn embeddable(bytes: &[u8], index: u32) -> Result<(), &'static str> {
    let face = ttf_parser::Face::parse(bytes, index).map_err(|_| "the font file cannot be read")?;
    if face.tables().cff.is_some() || face.tables().cff2.is_some() {
        return Err("PowerPoint embeds only fonts with TrueType outlines");
    }
    if face.permissions() == Some(ttf_parser::Permissions::Restricted) {
        return Err("its license does not permit embedding");
    }
    Ok(())
}

/// An Embedded OpenType file (version 2.1, no compression, no XOR) that
/// holds the single-face TrueType font `font`.
pub fn eot(font: &[u8]) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(font, 0).ok()?;
    let raw = ttf_parser::RawFace::parse(font, 0).ok()?;
    let table = |tag: &[u8; 4]| raw.table(ttf_parser::Tag::from_bytes(tag));
    let os2 = table(b"OS/2")?;
    let head = table(b"head")?;
    let be16 = |data: &[u8], at: usize| {
        data.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let be32 = |data: &[u8], at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == id && name.is_unicode())
            .find(|name| name.language_id == 0x409)
            .and_then(|name| name.to_string())
            .unwrap_or_default()
    };

    let mut out = Vec::new();
    let le32 = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
    let le16 = |out: &mut Vec<u8>, value: u16| out.extend_from_slice(&value.to_le_bytes());
    le32(&mut out, 0); // EOTSize, set at the end.
    le32(&mut out, font.len() as u32);
    le32(&mut out, 0x0002_0001);
    le32(&mut out, 0); // Flags: no subsetting, compression or XOR.
    out.extend_from_slice(os2.get(32..42)?); // PANOSE
    out.push(1); // DEFAULT_CHARSET
    out.push(u8::from(face.is_italic()));
    le32(&mut out, be16(os2, 4)? as u32);
    le16(&mut out, be16(os2, 8)?);
    le16(&mut out, 0x504C);
    for at in [42, 46, 50, 54] {
        le32(&mut out, be32(os2, at)?);
    }
    for at in [78, 82] {
        le32(&mut out, be32(os2, at).unwrap_or(0));
    }
    le32(&mut out, be32(head, 8)?);
    for _ in 0..4 {
        le32(&mut out, 0);
    }
    for text in [name(1), name(2), name(5), name(4), String::new()] {
        le16(&mut out, 0); // Padding
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        le16(&mut out, bytes.len() as u16);
        out.extend_from_slice(&bytes);
    }
    out.extend_from_slice(font);
    let size = out.len() as u32;
    out[..4].copy_from_slice(&size.to_le_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_from_600_are_bold() {
        assert!(!is_bold(&FontFace::new("Inter", 500, false)));
        assert!(is_bold(&FontFace::new("Inter", 600, false)));
        assert_eq!(slot(&FontFace::new("Inter", 300, true)), 2);
        assert_eq!(slot(&FontFace::new("Inter", 800, true)), 3);
    }

    #[test]
    fn an_eot_file_wraps_the_font_after_its_header() {
        let font =
            crate::font_file::style_member(&crate::assets::fonts()[2], 0, "Inter", true, false)
                .unwrap();
        let eot = eot(&font).unwrap();
        let le32 = |at: usize| u32::from_le_bytes(eot[at..at + 4].try_into().unwrap());
        assert_eq!(le32(0) as usize, eot.len());
        assert_eq!(le32(4) as usize, font.len());
        assert_eq!(le32(8), 0x0002_0001);
        assert_eq!(u16::from_le_bytes([eot[34], eot[35]]), 0x504C);
        assert!(eot.ends_with(&font));
        // The family name follows the fixed part of the header.
        let size = u16::from_le_bytes([eot[82], eot[83]]) as usize;
        let family: Vec<u16> = eot[84..84 + size]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16(&family).unwrap(), "Inter");
    }
}
