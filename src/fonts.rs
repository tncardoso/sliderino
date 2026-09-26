//! Fonts the author can pick: Inter, bundled with the app, plus the fonts
//! installed on the system. Picking a face embeds its bytes in the
//! presentation (see [`Operation::AddFont`](crate::document::Operation)).
//!
//! Also maps embedded faces to GPUI font ids, so the editor can paint the
//! glyph ids that `text_layout` computes from the same bytes.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui_kit::{App, Font, FontFeatures, FontId, FontStyle, FontWeight, px};

use crate::document::{FontData, FontFace};

/// Why a face cannot be embedded, shown next to it in the picker.
pub const RESTRICTED_LICENSE: &str = "The font's license forbids embedding";

pub struct Catalog {
    db: fontdb::Database,
    families: Vec<Family>,
    /// Embedding permission per face, read on first use.
    restricted: Mutex<HashMap<FontFace, bool>>,
}

pub struct Family {
    pub name: String,
    /// Faces sorted by slant, then weight.
    pub faces: Vec<FontFace>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();
static BUNDLED: OnceLock<Catalog> = OnceLock::new();

/// The bundled and system fonts. Scanning the system takes a moment and
/// blocks; the editor loads it in the background and reads it with
/// [`catalog_ready`].
pub fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(Catalog::load)
}

/// Bytes of a face to embed. Bundled faces are found without scanning the
/// system.
pub fn data(face: &FontFace) -> Option<FontData> {
    BUNDLED
        .get_or_init(|| Catalog::from_db(bundled_db()))
        .data(face)
        .or_else(|| catalog().data(face))
}

fn bundled_db() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    for bytes in crate::assets::fonts() {
        db.load_font_source(fontdb::Source::Binary(Arc::new(bytes.into_owned())));
    }
    db
}

/// The catalog if it has loaded, without waiting for it.
pub fn catalog_ready() -> Option<&'static Catalog> {
    CATALOG.get()
}

impl Catalog {
    fn load() -> Self {
        let mut db = bundled_db();
        db.load_system_fonts();
        Self::from_db(db)
    }

    fn from_db(db: fontdb::Database) -> Self {
        let mut families: Vec<Family> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Faces load in order: a bundled face wins over an installed copy.
        for info in db.faces() {
            let face = face_of(info);
            if !seen.insert(face.clone()) {
                continue;
            }
            match families
                .iter_mut()
                .find(|family| family.name == face.family)
            {
                Some(family) => family.faces.push(face),
                None => families.push(Family {
                    name: face.family.clone(),
                    faces: vec![face],
                }),
            }
        }
        families.sort_by_key(|family| family.name.to_lowercase());
        for family in &mut families {
            family.faces.sort_by_key(|face| (face.italic, face.weight));
        }
        Self {
            db,
            families,
            restricted: Mutex::new(HashMap::new()),
        }
    }

    pub fn families(&self) -> &[Family] {
        &self.families
    }

    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.iter().find(|family| family.name == name)
    }

    /// The face's bytes, to embed in a presentation.
    pub fn data(&self, face: &FontFace) -> Option<FontData> {
        let id = self
            .db
            .faces()
            .find(|info| face_of(info) == *face)
            .map(|info| info.id)?;
        self.db.with_face_data(id, |bytes, index| FontData {
            bytes: Arc::from(bytes),
            index,
        })
    }

    /// Whether the face's license (OS/2 `fsType`) forbids embedding it.
    pub fn is_restricted(&self, face: &FontFace) -> bool {
        let mut cache = self.restricted.lock().expect("font cache lock");
        *cache.entry(face.clone()).or_insert_with(|| {
            self.data(face).is_none_or(|data| {
                ttf_parser::Face::parse(&data.bytes, data.index).map_or(true, |parsed| {
                    parsed.permissions() == Some(ttf_parser::Permissions::Restricted)
                })
            })
        })
    }
}

fn face_of(info: &fontdb::FaceInfo) -> FontFace {
    let family = info
        .families
        .iter()
        .find(|(_, language)| *language == fontdb::Language::English_UnitedStates)
        .or(info.families.first())
        .map_or_else(|| info.post_script_name.clone(), |(name, _)| name.clone());
    FontFace {
        family,
        weight: info.weight.0,
        italic: info.style != fontdb::Style::Normal,
    }
}

/// The face of `faces` closest to `weight` and `italic`: the same slant when
/// the family has it, then the nearest weight (the lighter one on a tie).
pub fn closest_face(faces: &[FontFace], weight: u16, italic: bool) -> Option<&FontFace> {
    faces.iter().min_by_key(|face| {
        (
            face.italic != italic,
            face.weight.abs_diff(weight),
            face.weight,
        )
    })
}

/// GPUI font ids of the faces embedded in the open presentation.
#[derive(Default)]
pub struct FontRegistry {
    ids: HashMap<FontFace, Option<FontId>>,
}

impl FontRegistry {
    /// The GPUI font whose glyphs match `data`, loading the bytes into GPUI
    /// when the installed fonts do not provide that exact face.
    pub fn font_id(&mut self, face: &FontFace, data: &FontData, cx: &App) -> Option<FontId> {
        *self.ids.entry(face.clone()).or_insert_with(|| {
            let font = Font {
                family: face.family.clone().into(),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: FontWeight(face.weight as f32),
                style: if face.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
            };
            let text_system = cx.text_system();
            // `resolve_font` falls back to another font when the family is
            // missing, so the result is checked against the bytes.
            let resolve =
                || Some(text_system.resolve_font(&font)).filter(|id| same_face(*id, data, cx));
            resolve().or_else(|| {
                text_system
                    .add_fonts(vec![Cow::Owned(data.bytes.to_vec())])
                    .ok()?;
                resolve()
            })
        })
    }
}

/// Whether GPUI's font `id` has the glyphs and advances of `data`.
fn same_face(id: FontId, data: &FontData, cx: &App) -> bool {
    let Ok(parsed) = ttf_parser::Face::parse(&data.bytes, data.index) else {
        return false;
    };
    let text_system = cx.text_system();
    if text_system.units_per_em(id) != parsed.units_per_em() as u32 {
        return false;
    }
    let size = parsed.units_per_em() as f32;
    ['m', 'W', 'a', '0'].into_iter().all(|ch| {
        let ours = parsed
            .glyph_index(ch)
            .and_then(|glyph| parsed.glyph_hor_advance(glyph));
        let theirs = text_system.advance(id, px(size), ch).ok();
        match (ours, theirs) {
            (Some(ours), Some(theirs)) => (ours as f32 - f32::from(theirs.width)).abs() < 0.5,
            (None, _) => true,
            (Some(_), None) => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> Catalog {
        Catalog::from_db(bundled_db())
    }

    #[test]
    fn bundled_inter_has_upright_and_italic_faces() {
        let catalog = bundled();
        let inter = catalog.family("Inter").expect("Inter is bundled");
        let names: Vec<String> = inter.faces.iter().map(FontFace::style_name).collect();
        assert_eq!(
            names,
            [
                "Regular",
                "Medium",
                "SemiBold",
                "ExtraBold",
                "Italic",
                "Medium Italic",
                "SemiBold Italic",
                "ExtraBold Italic"
            ]
        );
        let regular = FontFace::new("Inter", 400, false);
        assert!(!catalog.is_restricted(&regular));
        let data = catalog.data(&regular).unwrap();
        assert!(crate::text_layout::FontMetrics::read(&data).is_ok());
    }

    #[test]
    fn closest_face_prefers_the_slant_then_the_weight() {
        let faces = [
            FontFace::new("F", 400, false),
            FontFace::new("F", 700, false),
            FontFace::new("F", 400, true),
        ];
        assert_eq!(closest_face(&faces, 600, false).unwrap().weight, 700);
        assert_eq!(closest_face(&faces, 550, false).unwrap().weight, 400);
        assert_eq!(closest_face(&faces, 800, true), Some(&faces[2]));
        assert_eq!(closest_face(&[], 400, false), None);
    }
}
