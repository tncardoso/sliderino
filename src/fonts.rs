//! Fonts the author can pick: Inter, bundled with the app, plus the fonts
//! installed on the system. Picking a face embeds its bytes in the
//! presentation (see [`Operation::AddFont`](crate::document::Operation)).
//!
//! Also maps embedded faces to GPUI font ids, so the editor can paint the
//! glyph ids that `text_layout` computes from the same bytes.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use gpui_kit::{App, Font, FontFeatures, FontId, FontStyle, FontWeight, px};

use crate::document::{ElementId, FontData, FontFace, Operation, Presentation, TextStylePatch};
use crate::font_file;

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

/// The fonts bundled with the app, found without scanning the system.
fn bundled() -> &'static Catalog {
    BUNDLED.get_or_init(|| Catalog::from_db(bundled_db()))
}

/// Bytes of a face to embed. Bundled faces are found without scanning the
/// system.
pub fn data(face: &FontFace) -> Option<FontData> {
    bundled().data(face).or_else(|| catalog().data(face))
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

/// The family that text falls back to when its font is removed.
pub const FALLBACK_FAMILY: &str = "Inter";

/// Why an uploaded file cannot be embedded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFileError {
    /// A WOFF or WOFF2 web font.
    WebFont,
    /// Not a TTF, OTF or TTC font, or no face in it can lay out text.
    NotAFont,
}

impl std::fmt::Display for FontFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontFileError::WebFont => write!(
                f,
                "WOFF and WOFF2 fonts are not supported: convert the font to TTF or OTF"
            ),
            FontFileError::NotAFont => write!(f, "the file is not a TTF, OTF or TTC font"),
        }
    }
}

impl std::error::Error for FontFileError {}

/// Whether the file name says it is a font that can be uploaded.
pub fn is_font_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|font| ext.eq_ignore_ascii_case(font))
        })
}

/// One face of an uploaded file, before it is embedded.
#[derive(Clone, Debug)]
pub struct UploadedFace {
    /// The face as the file names it.
    pub face: FontFace,
    pub data: FontData,
    /// The family the file had before Sliderino renamed it, if it did.
    pub original: String,
    /// A variable font: only its default instance is used.
    pub variable: bool,
}

/// The faces of an uploaded font file. The license of the faces is not
/// checked: the author holds it.
pub fn read_file(bytes: Vec<u8>) -> Result<Vec<UploadedFace>, FontFileError> {
    if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
        return Err(FontFileError::WebFont);
    }
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let mut db = fontdb::Database::new();
    db.load_font_source(fontdb::Source::Binary(Arc::new(bytes.to_vec())));
    let faces: Vec<UploadedFace> = db
        .faces()
        .filter_map(|info| {
            let data = FontData {
                bytes: bytes.clone(),
                index: info.index,
            };
            crate::text_layout::FontMetrics::read(&data).ok()?;
            let parsed = ttf_parser::Face::parse(&bytes, info.index).ok()?;
            let face = face_of(info);
            Some(UploadedFace {
                original: font_file::original_family(&parsed)
                    .unwrap_or_else(|| face.family.clone()),
                variable: parsed.is_variable(),
                face,
                data,
            })
        })
        .collect();
    if faces.is_empty() {
        return Err(FontFileError::NotAFont);
    }
    Ok(faces)
}

/// What an upload does: the operations that embed the new faces, the faces
/// of the upload under their final names, and what the author should know.
#[derive(Clone, Debug, Default)]
pub struct UploadPlan {
    pub operations: Vec<Operation>,
    /// Every face of the upload, embedded now or before.
    pub faces: Vec<FontFace>,
    pub warnings: Vec<String>,
}

/// Plans the upload of `uploaded` into a presentation that embeds
/// `embedded`, next to the installed fonts of `catalog`.
///
/// The faces of one family keep one family name. It is the name in the file
/// when no other font has it; else an embedded family that came from the
/// same original name and has no other face of the same weight and slant;
/// else the first free "Original (2)", "Original (3)"… The bytes of a renamed
/// face say the new name, so that the editor can find the face. A face whose
/// bytes are embedded already is not embedded again.
pub fn plan_upload(
    embedded: &[(FontFace, FontData)],
    catalog: &Catalog,
    uploaded: Vec<UploadedFace>,
) -> UploadPlan {
    let mut plan = UploadPlan::default();
    let mut embedded: Vec<(FontFace, FontData)> = embedded.to_vec();
    // Faces with the same family and the same original family.
    let mut groups: Vec<(String, String, Vec<UploadedFace>)> = Vec::new();
    for face in uploaded {
        if face.variable {
            plan.warnings.push(format!(
                "{} is a variable font: only its default instance is used",
                face.face.family
            ));
        }
        match groups
            .iter_mut()
            .find(|(name, original, _)| *name == face.face.family && *original == face.original)
        {
            Some((_, _, faces)) => faces.push(face),
            None => groups.push((face.face.family.clone(), face.original.clone(), vec![face])),
        }
    }
    for (name, original, faces) in groups {
        let family = pick_family(&name, &original, &faces, &embedded, catalog);
        if family != name {
            plan.warnings.push(format!(
                "{name} is embedded as {family}: the name is in use"
            ));
        }
        for upload in faces {
            let face = FontFace {
                family: family.clone(),
                ..upload.face.clone()
            };
            let Some(data) = named(&upload, &family) else {
                plan.warnings
                    .push(format!("{} cannot be renamed", upload.face.family));
                continue;
            };
            plan.faces.push(face.clone());
            if embedded.iter().any(|(other, _)| *other == face) {
                continue;
            }
            embedded.push((face.clone(), data.clone()));
            plan.operations.push(Operation::AddFont { face, data });
        }
    }
    plan.faces.dedup();
    plan
}

/// The bytes of `upload` whose family is `family`.
fn named(upload: &UploadedFace, family: &str) -> Option<FontData> {
    if upload.face.family == family {
        return Some(upload.data.clone());
    }
    let bytes = font_file::rename(&upload.data.bytes, upload.data.index, family)?;
    Some(FontData {
        bytes: Arc::from(bytes),
        index: 0,
    })
}

/// The family name that faces named `name`, first named `original`, take
/// (see [`plan_upload`]).
fn pick_family(
    name: &str,
    original: &str,
    faces: &[UploadedFace],
    embedded: &[(FontFace, FontData)],
    catalog: &Catalog,
) -> String {
    let fits = |family: &str| {
        let installed = catalog.family(family).is_some_and(|installed| {
            // An installed family fits only when the upload is its files.
            !faces.iter().all(|upload| {
                let face = FontFace {
                    family: family.to_string(),
                    ..upload.face.clone()
                };
                installed.faces.contains(&face)
                    && catalog
                        .data(&face)
                        .is_some_and(|data| same_bytes(&data, &upload.data))
            })
        });
        if installed {
            return false;
        }
        let mut members = embedded
            .iter()
            .filter(|(face, _)| face.family == family)
            .peekable();
        if members.peek().is_none() {
            return true;
        }
        members.all(|(face, data)| {
            let from_original = ttf_parser::Face::parse(&data.bytes, data.index)
                .ok()
                .and_then(|parsed| font_file::original_family(&parsed))
                .is_some_and(|name| name == original);
            let clash = faces.iter().find(|upload| {
                upload.face.weight == face.weight && upload.face.italic == face.italic
            });
            from_original
                && clash.is_none_or(|upload| {
                    named(upload, family).is_some_and(|ours| same_bytes(&ours, data))
                })
        })
    };
    if fits(name) {
        return name.to_string();
    }
    // Renamed families of the same original, in order, then a new one.
    (2..)
        .map(|n| format!("{original} ({n})"))
        .find(|family| fits(family))
        .expect("a free family name")
}

fn same_bytes(a: &FontData, b: &FontData) -> bool {
    a.index == b.index && (Arc::ptr_eq(&a.bytes, &b.bytes) || a.bytes == b.bytes)
}

/// Why an embedded family cannot be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveFamilyError {
    /// No face of the family is embedded.
    Missing(String),
    /// The fallback family is used by this many texts.
    FallbackInUse(usize),
}

impl std::fmt::Display for RemoveFamilyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoveFamilyError::Missing(family) => write!(f, "font {family} is not embedded"),
            RemoveFamilyError::FallbackInUse(count) => write!(
                f,
                "{FALLBACK_FAMILY} is used by {count} text{} and is the fallback font",
                if *count == 1 { "" } else { "s" }
            ),
        }
    }
}

impl std::error::Error for RemoveFamilyError {}

/// The texts of the presentation whose font is of `family`.
pub fn texts_using(presentation: &Presentation, family: &str) -> Vec<(ElementId, FontFace)> {
    presentation
        .slides
        .iter()
        .flat_map(|slide| slide.walk())
        .filter_map(|node| {
            let text = node.element.as_text()?;
            (text.style.font.family == family).then(|| (node.element.id, text.style.font.clone()))
        })
        .collect()
}

/// Operations that remove every embedded face of `family`. Texts that use
/// it change to the closest face of [`FALLBACK_FAMILY`], which is embedded
/// first when needed.
pub fn remove_family(
    presentation: &Presentation,
    family: &str,
) -> Result<Vec<Operation>, RemoveFamilyError> {
    let faces: Vec<FontFace> = presentation
        .fonts
        .faces()
        .filter(|face| face.family == family)
        .cloned()
        .collect();
    if faces.is_empty() {
        return Err(RemoveFamilyError::Missing(family.to_string()));
    }
    let users = texts_using(presentation, family);
    if family == FALLBACK_FAMILY && !users.is_empty() {
        return Err(RemoveFamilyError::FallbackInUse(users.len()));
    }
    let fallback = bundled()
        .family(FALLBACK_FAMILY)
        .map(|family| family.faces.clone())
        .unwrap_or_default();
    let mut operations = Vec::new();
    let mut added: Vec<FontFace> = Vec::new();
    for (id, face) in users {
        let Some(to) = closest_face(&fallback, face.weight, face.italic) else {
            continue;
        };
        if !presentation.fonts.contains(to)
            && !added.contains(to)
            && let Some(data) = bundled().data(to)
        {
            added.push(to.clone());
            operations.push(Operation::AddFont {
                face: to.clone(),
                data,
            });
        }
        operations.push(Operation::SetTextStyle {
            id,
            patch: TextStylePatch {
                font: Some(to.clone()),
                ..Default::default()
            },
        });
    }
    operations.extend(faces.into_iter().map(|face| Operation::RemoveFont { face }));
    Ok(operations)
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

    fn inter_file(index: usize) -> Vec<u8> {
        crate::assets::fonts()[index].to_vec()
    }

    /// Embedded faces after applying the plan's operations.
    fn embed(embedded: &mut Vec<(FontFace, FontData)>, plan: &UploadPlan) {
        for operation in &plan.operations {
            if let Operation::AddFont { face, data } = operation {
                embedded.push((face.clone(), data.clone()));
            }
        }
    }

    #[test]
    fn font_files_are_told_by_their_extension() {
        assert!(is_font_path(Path::new("a/Roboto.TTF")));
        assert!(is_font_path(Path::new("b.otf")));
        assert!(is_font_path(Path::new("c.ttc")));
        assert!(!is_font_path(Path::new("d.woff2")));
        assert!(!is_font_path(Path::new("e.png")));
    }

    #[test]
    fn reading_a_file_gives_its_faces() {
        let faces = read_file(inter_file(0)).unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].face, FontFace::new("Inter", 400, false));
        assert_eq!(faces[0].original, "Inter");
        assert!(!faces[0].variable);
        assert_eq!(
            read_file(b"not a font".to_vec()).unwrap_err(),
            FontFileError::NotAFont
        );
        assert_eq!(
            read_file(b"wOF2 and more".to_vec()).unwrap_err(),
            FontFileError::WebFont
        );
    }

    #[test]
    fn an_upload_with_a_free_name_keeps_it() {
        let catalog = Catalog::from_db(fontdb::Database::new());
        let plan = plan_upload(&[], &catalog, read_file(inter_file(0)).unwrap());
        assert_eq!(plan.faces, [FontFace::new("Inter", 400, false)]);
        assert_eq!(plan.operations.len(), 1);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn an_upload_of_an_installed_family_is_renamed_unless_it_is_the_same_file() {
        let catalog = bundled();
        let same = plan_upload(&[], &catalog, read_file(inter_file(0)).unwrap());
        assert_eq!(same.faces, [FontFace::new("Inter", 400, false)]);

        // Other bytes under the name of an installed family.
        let other = font_file::rename(&inter_file(1), 0, "Inter").unwrap();
        let faces = read_file(other).unwrap();
        let plan = plan_upload(&[], &catalog, faces);
        assert_eq!(plan.faces, [FontFace::new("Inter (2)", 500, false)]);
        let Operation::AddFont { data, .. } = &plan.operations[0] else {
            panic!("an AddFont");
        };
        let read = read_file(data.bytes.to_vec()).unwrap();
        assert_eq!(read[0].face.family, "Inter (2)");
        assert_eq!(read[0].original, "Inter");
    }

    #[test]
    fn files_of_one_upload_share_the_new_name_and_later_ones_join_it() {
        let catalog = bundled();
        let foreign = |index| {
            let bytes = font_file::rename(&inter_file(index), 0, "Inter").unwrap();
            // Renamed files remember "Inter" as the original name.
            read_file(bytes).unwrap()
        };
        let mut uploaded = foreign(0);
        uploaded.extend(foreign(2));
        let mut embedded = Vec::new();
        let plan = plan_upload(&embedded, &catalog, uploaded);
        assert_eq!(
            plan.faces,
            [
                FontFace::new("Inter (2)", 400, false),
                FontFace::new("Inter (2)", 600, false)
            ]
        );
        embed(&mut embedded, &plan);

        // The same file again embeds nothing.
        let again = plan_upload(&embedded, &catalog, foreign(0));
        assert!(again.operations.is_empty());
        assert_eq!(again.faces, [FontFace::new("Inter (2)", 400, false)]);

        // A new weight of the same family joins it.
        let bold = plan_upload(&embedded, &catalog, foreign(3));
        assert_eq!(bold.faces, [FontFace::new("Inter (2)", 800, false)]);

        // Other bytes of an embedded weight go to a new family.
        let clash = read_file(font_file::rename(&inter_file(1), 0, "Inter").unwrap())
            .unwrap()
            .into_iter()
            .map(|mut upload| {
                upload.face.weight = 400;
                upload
            })
            .collect();
        let plan = plan_upload(&embedded, &catalog, clash);
        assert_eq!(plan.faces, [FontFace::new("Inter (3)", 400, false)]);
    }

    #[test]
    fn removing_a_family_moves_its_texts_to_the_fallback() {
        use crate::document::{Element, ElementKind, Frame, TextElement, TextSizing, TextStyle};
        let mut presentation = Presentation::new();
        let face = FontFace::new("Inter (2)", 600, true);
        let data = FontData {
            bytes: Arc::from(font_file::rename(&inter_file(6), 0, "Inter (2)").unwrap()),
            index: 0,
        };
        presentation
            .apply(Operation::AddFont {
                face: face.clone(),
                data,
            })
            .unwrap();
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        let text = TextElement {
            content: "Hi".into(),
            style: TextStyle {
                font: face.clone(),
                ..TextStyle::default()
            },
            sizing: TextSizing::AutoWidth,
        };
        presentation
            .apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element: Element::new(id, Frame::default(), ElementKind::Text(text)),
            })
            .unwrap();
        let before = presentation.clone();

        let operations = remove_family(&presentation, "Inter (2)").unwrap();
        let undo = presentation.apply(Operation::Batch(operations)).unwrap();
        assert!(!presentation.fonts.contains(&face));
        let font = &presentation
            .element(id)
            .unwrap()
            .as_text()
            .unwrap()
            .style
            .font;
        assert_eq!(*font, FontFace::new("Inter", 600, true));

        assert_eq!(
            remove_family(&presentation, "Inter"),
            Err(RemoveFamilyError::FallbackInUse(1))
        );
        presentation.apply(undo).unwrap();
        assert_eq!(presentation, before);
    }
}
