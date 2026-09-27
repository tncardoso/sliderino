//! Rewrites the family name inside a font file, so that an uploaded font
//! whose family clashes with another one can be embedded under a new name.
//!
//! GPUI finds a font only by the family name in its `name` table, so a face
//! embedded as "Roboto (2)" must also say "Roboto (2)" in its bytes. The
//! rewrite copies the tables of one face into a new single-face file and
//! replaces the `name` table. The original family stays in the file, in the
//! record [`ORIGINAL_FAMILY_ID`], so that later uploads of the same family
//! can join the renamed one.

use ttf_parser::name::Name;
use ttf_parser::{PlatformId, RawFace, Tag};

/// Font-specific name id that keeps the family the file had before it was
/// renamed. Ids from 256 name instances and axes of variable fonts; the last
/// one is the least likely to be in use.
pub const ORIGINAL_FAMILY_ID: u16 = 32767;

const FAMILY_ID: u16 = 1;
const SUBFAMILY_ID: u16 = 2;
const FULL_NAME_ID: u16 = 4;
const POSTSCRIPT_ID: u16 = 6;
const TYPOGRAPHIC_FAMILY_ID: u16 = 16;
const TYPOGRAPHIC_SUBFAMILY_ID: u16 = 17;
const WWS_FAMILY_ID: u16 = 21;
const WWS_SUBFAMILY_ID: u16 = 22;

const WINDOWS: u16 = 3;
const UNICODE_BMP: u16 = 1;
const ENGLISH_US: u16 = 0x409;

/// A single-face copy of face `index` of `bytes`, whose family is `family`.
/// The family names of the face, its full name and its PostScript name
/// change; the other names stay. `None` when the file cannot be read.
pub fn rename(bytes: &[u8], index: u32, family: &str) -> Option<Vec<u8>> {
    let raw = RawFace::parse(bytes, index).ok()?;
    let face = ttf_parser::Face::parse(bytes, index).ok()?;
    let old_family = family_name(&face)?;
    let original = original_family(&face).unwrap_or_else(|| old_family.clone());

    let english = |id: u16| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == id && name.is_unicode())
            .find(|name| name.language_id == ENGLISH_US)
            .or_else(|| {
                face.names()
                    .into_iter()
                    .find(|name| name.name_id == id && name.is_unicode())
            })
            .and_then(|name| name.to_string())
    };
    let retitle = |id: u16| {
        let old = english(id).unwrap_or_else(|| old_family.clone());
        match old.strip_prefix(&old_family) {
            Some(rest) => format!("{family}{rest}"),
            None => family.to_string(),
        }
    };
    let postscript = {
        let old = english(POSTSCRIPT_ID).unwrap_or_default();
        let style = old.split_once('-').map_or("", |(_, style)| style);
        let base: String = family
            .chars()
            .filter(|ch| ch.is_ascii_graphic() && !"[](){}<>/%".contains(*ch))
            .collect();
        if style.is_empty() {
            base
        } else {
            format!("{base}-{style}")
        }
    };

    let replaced = [
        FAMILY_ID,
        FULL_NAME_ID,
        POSTSCRIPT_ID,
        TYPOGRAPHIC_FAMILY_ID,
        WWS_FAMILY_ID,
        ORIGINAL_FAMILY_ID,
    ];
    let mut records: Vec<NameRecord> = face
        .names()
        .into_iter()
        // Format 1 language tags are dropped with the table format.
        .filter(|name| !replaced.contains(&name.name_id) && name.language_id < 0x8000)
        .map(|name| NameRecord::copy(&name))
        .collect();
    for (id, text) in [
        (FAMILY_ID, retitle(FAMILY_ID)),
        (FULL_NAME_ID, retitle(FULL_NAME_ID)),
        (POSTSCRIPT_ID, postscript),
        (TYPOGRAPHIC_FAMILY_ID, family.to_string()),
        (ORIGINAL_FAMILY_ID, original),
    ] {
        records.push(NameRecord::windows(id, &text));
    }
    rebuild(bytes, index, &raw, records, |_, _| None)
}

/// A single-face copy of face `index` of `bytes` as the regular, bold,
/// italic or bold italic member of `family`, as Windows links the styles
/// of a family: the family and style names say so, the typographic names
/// go, and the weight class is 400 or 700. PowerPoint finds an embedded
/// font by these names, so a SemiBold face can be the bold of its family.
/// `None` when the file cannot be read.
pub fn style_member(
    bytes: &[u8],
    index: u32,
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<Vec<u8>> {
    let raw = RawFace::parse(bytes, index).ok()?;
    let face = ttf_parser::Face::parse(bytes, index).ok()?;
    let style = match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    };
    let replaced = [
        FAMILY_ID,
        SUBFAMILY_ID,
        FULL_NAME_ID,
        POSTSCRIPT_ID,
        TYPOGRAPHIC_FAMILY_ID,
        TYPOGRAPHIC_SUBFAMILY_ID,
        WWS_FAMILY_ID,
        WWS_SUBFAMILY_ID,
    ];
    let mut records: Vec<NameRecord> = face
        .names()
        .into_iter()
        .filter(|name| !replaced.contains(&name.name_id) && name.language_id < 0x8000)
        .map(|name| NameRecord::copy(&name))
        .collect();
    let postscript: String = format!("{family}-{style}")
        .chars()
        .filter(|ch| ch.is_ascii_graphic() && !"[](){}<>/%".contains(*ch))
        .collect();
    let full = if style == "Regular" {
        family.to_string()
    } else {
        format!("{family} {style}")
    };
    for (id, text) in [
        (FAMILY_ID, family.to_string()),
        (SUBFAMILY_ID, style.to_string()),
        (FULL_NAME_ID, full),
        (POSTSCRIPT_ID, postscript),
    ] {
        records.push(NameRecord::windows(id, &text));
    }
    rebuild(bytes, index, &raw, records, |tag, table| {
        let mut table = table.to_vec();
        if tag == Tag::from_bytes(b"OS/2") && table.len() >= 64 {
            let weight: u16 = if bold { 700 } else { 400 };
            table[4..6].copy_from_slice(&weight.to_be_bytes());
            // fsSelection: ITALIC is bit 0, BOLD bit 5, REGULAR bit 6.
            let mut selection = u16::from_be_bytes([table[62], table[63]]) & !0b110_0001;
            selection |= match (bold, italic) {
                (false, false) => 1 << 6,
                (true, false) => 1 << 5,
                (false, true) => 1,
                (true, true) => (1 << 5) | 1,
            };
            table[62..64].copy_from_slice(&selection.to_be_bytes());
            return Some(table);
        }
        if tag == Tag::from_bytes(b"head") && table.len() >= 46 {
            // macStyle: bold is bit 0, italic bit 1.
            let mut style = u16::from_be_bytes([table[44], table[45]]) & !0b11;
            style |= u16::from(bold) | (u16::from(italic) << 1);
            table[44..46].copy_from_slice(&style.to_be_bytes());
            return Some(table);
        }
        None
    })
}

/// A single-face file of the tables of face `index`, with a new `name`
/// table of `records`. `patch` gives a new copy of a table, or `None` to
/// keep it.
fn rebuild(
    bytes: &[u8],
    index: u32,
    raw: &RawFace,
    records: Vec<NameRecord>,
    patch: impl Fn(Tag, &[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let name = name_table(records);
    let mut owned: Vec<(Tag, std::borrow::Cow<[u8]>)> = Vec::new();
    for record in raw.table_records {
        if record.tag == Tag::from_bytes(b"name") {
            continue;
        }
        let start = record.offset as usize;
        let data = bytes.get(start..start.checked_add(record.length as usize)?)?;
        let table = match patch(record.tag, data) {
            Some(table) => std::borrow::Cow::Owned(table),
            None => std::borrow::Cow::Borrowed(data),
        };
        owned.push((record.tag, table));
    }
    owned.push((Tag::from_bytes(b"name"), std::borrow::Cow::Borrowed(&name)));
    let tables: Vec<(Tag, &[u8])> = owned.iter().map(|(tag, data)| (*tag, &**data)).collect();
    Some(sfnt(sfnt_version(bytes, index)?, tables))
}

/// The family that the face had before Sliderino renamed it, or its family.
pub fn original_family(face: &ttf_parser::Face) -> Option<String> {
    face.names()
        .into_iter()
        .find(|name| name.name_id == ORIGINAL_FAMILY_ID && name.is_unicode())
        .and_then(|name| name.to_string())
        .or_else(|| family_name(face))
}

/// The family as fontdb reads it: the typographic family, else the family.
fn family_name(face: &ttf_parser::Face) -> Option<String> {
    [TYPOGRAPHIC_FAMILY_ID, FAMILY_ID]
        .into_iter()
        .find_map(|id| {
            let names: Vec<Name> = face
                .names()
                .into_iter()
                .filter(|name| name.name_id == id && name.is_unicode())
                .collect();
            names
                .iter()
                .find(|name| name.language_id == ENGLISH_US)
                .or(names.first())
                .and_then(|name| name.to_string())
        })
}

/// The version tag of face `index`: the start of its table directory.
fn sfnt_version(bytes: &[u8], index: u32) -> Option<u32> {
    let read = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let start = if bytes.starts_with(b"ttcf") {
        read(12 + 4 * index as usize)? as usize
    } else {
        0
    };
    read(start)
}

struct NameRecord {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    data: Vec<u8>,
}

impl NameRecord {
    fn copy(name: &Name) -> Self {
        Self {
            platform: platform_number(name.platform_id),
            encoding: name.encoding_id,
            language: name.language_id,
            id: name.name_id,
            data: name.name.to_vec(),
        }
    }

    fn windows(id: u16, text: &str) -> Self {
        Self {
            platform: WINDOWS,
            encoding: UNICODE_BMP,
            language: ENGLISH_US,
            id,
            data: text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        }
    }
}

fn platform_number(platform: PlatformId) -> u16 {
    match platform {
        PlatformId::Unicode => 0,
        PlatformId::Macintosh => 1,
        PlatformId::Iso => 2,
        PlatformId::Windows => 3,
        PlatformId::Custom => 4,
    }
}

/// A format 0 `name` table, with the records sorted as the format asks.
fn name_table(mut records: Vec<NameRecord>) -> Vec<u8> {
    records.sort_by_key(|r| (r.platform, r.encoding, r.language, r.id));
    let storage_start = 6 + 12 * records.len();
    let mut out = Vec::new();
    let mut storage = Vec::new();
    push16(&mut out, 0);
    push16(&mut out, records.len() as u16);
    push16(&mut out, storage_start as u16);
    for record in &records {
        for value in [
            record.platform,
            record.encoding,
            record.language,
            record.id,
            record.data.len() as u16,
            storage.len() as u16,
        ] {
            push16(&mut out, value);
        }
        storage.extend_from_slice(&record.data);
    }
    out.extend(storage);
    out
}

/// A single-face font file of `tables`, with checksums.
fn sfnt(version: u32, mut tables: Vec<(Tag, &[u8])>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let count = tables.len() as u16;
    let power = if count == 0 {
        0
    } else {
        15 - count.leading_zeros() as u16
    };
    let search_range = (1u16 << power) * 16;
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    push16(&mut out, count);
    push16(&mut out, search_range);
    push16(&mut out, power);
    push16(&mut out, count * 16 - search_range);

    let mut offset = 12 + 16 * tables.len();
    let mut data = Vec::new();
    let mut head_at = None;
    for (tag, table) in &tables {
        let mut table = table.to_vec();
        if *tag == Tag::from_bytes(b"head") && table.len() >= 12 {
            // checkSumAdjustment is summed as 0, then set below.
            table[8..12].fill(0);
            head_at = Some(offset);
        }
        out.extend_from_slice(&tag.to_bytes());
        out.extend_from_slice(&checksum(&table).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(table.len() as u32).to_be_bytes());
        let padded = table.len().next_multiple_of(4);
        table.resize(padded, 0);
        offset += padded;
        data.extend(table);
    }
    out.extend(data);
    if let Some(at) = head_at {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn push16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inter() -> Vec<u8> {
        crate::assets::fonts()[2].to_vec()
    }

    fn families(bytes: &[u8]) -> Vec<(String, String)> {
        let mut db = fontdb::Database::new();
        db.load_font_data(bytes.to_vec());
        db.faces()
            .map(|info| (info.families[0].0.clone(), info.post_script_name.clone()))
            .collect()
    }

    #[test]
    fn renamed_font_reads_back_with_the_new_family() {
        let renamed = rename(&inter(), 0, "Inter (2)").unwrap();
        assert_eq!(
            families(&renamed),
            [("Inter (2)".to_string(), "Inter2-SemiBold".to_string())]
        );
        let face = ttf_parser::Face::parse(&renamed, 0).unwrap();
        assert_eq!(original_family(&face).as_deref(), Some("Inter"));
        assert_eq!(
            face.names()
                .into_iter()
                .find(|n| n.name_id == 4)
                .and_then(|n| n.to_string())
                .as_deref(),
            Some("Inter (2) SemiBold")
        );
        let data = crate::document::FontData {
            bytes: renamed.into(),
            index: 0,
        };
        assert!(crate::text_layout::FontMetrics::read(&data).is_ok());
    }

    #[test]
    fn a_semibold_face_becomes_the_bold_member_of_its_family() {
        let member = style_member(&inter(), 0, "Inter", true, false).unwrap();
        assert_eq!(checksum(&member), 0xB1B0_AFBA);
        let face = ttf_parser::Face::parse(&member, 0).unwrap();
        let name = |id: u16| {
            face.names()
                .into_iter()
                .find(|n| n.name_id == id && n.is_unicode())
                .and_then(|n| n.to_string())
        };
        assert_eq!(name(1).as_deref(), Some("Inter"));
        assert_eq!(name(2).as_deref(), Some("Bold"));
        assert_eq!(name(4).as_deref(), Some("Inter Bold"));
        assert_eq!(name(6).as_deref(), Some("Inter-Bold"));
        assert_eq!(name(16), None);
        assert_eq!(face.weight(), ttf_parser::Weight::Bold);
        assert!(face.is_bold());
        assert!(!face.is_italic());
        assert_eq!(families(&member)[0].0, "Inter");
    }

    #[test]
    fn renaming_twice_keeps_the_first_family() {
        let once = rename(&inter(), 0, "Inter (2)").unwrap();
        let twice = rename(&once, 0, "Inter (3)").unwrap();
        let face = ttf_parser::Face::parse(&twice, 0).unwrap();
        assert_eq!(families(&twice)[0].0, "Inter (3)");
        assert_eq!(original_family(&face).as_deref(), Some("Inter"));
    }

    #[test]
    fn renamed_font_has_a_valid_checksum() {
        let renamed = rename(&inter(), 0, "X").unwrap();
        assert_eq!(checksum(&renamed), 0xB1B0_AFBA);
    }

    #[test]
    fn a_face_of_a_collection_becomes_a_single_font() {
        // A collection of two copies of the same face.
        let face = inter();
        let mut ttc = Vec::new();
        ttc.extend_from_slice(b"ttcf");
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&2u32.to_be_bytes());
        let first = 20u32;
        let raw = RawFace::parse(&face, 0).unwrap();
        let directory = 12 + 16 * raw.table_records.len() as usize;
        let second = first + directory as u32;
        ttc.extend_from_slice(&first.to_be_bytes());
        ttc.extend_from_slice(&second.to_be_bytes());
        let data_start = second as usize + directory;
        let directory_of = |out: &mut Vec<u8>| {
            out.extend_from_slice(&face[..12]);
            for record in raw.table_records {
                out.extend_from_slice(&record.tag.to_bytes());
                out.extend_from_slice(&record.check_sum.to_be_bytes());
                out.extend_from_slice(&(record.offset + data_start as u32).to_be_bytes());
                out.extend_from_slice(&record.length.to_be_bytes());
            }
        };
        directory_of(&mut ttc);
        directory_of(&mut ttc);
        ttc.extend_from_slice(&face);
        assert_eq!(ttf_parser::fonts_in_collection(&ttc), Some(2));
        let renamed = rename(&ttc, 1, "Inter (2)").unwrap();
        assert!(!renamed.starts_with(b"ttcf"));
        assert_eq!(families(&renamed)[0].0, "Inter (2)");
    }
}
