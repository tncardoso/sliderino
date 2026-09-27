//! The `.sldr` file: a presentation with its fonts, images and videos.
//!
//! A `.sldr` file is a zip archive. `presentation.json` holds the slides and
//! the list of embedded files; the fonts, images and videos are next to it
//! as they were embedded, in `fonts/`, `images/` and `videos/`. Loading goes
//! through [`Presentation::from_saved`], so a file cannot hold what an edit
//! could not make.

use std::io::{Read as _, Seek, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::document::{
    FontData, FontFace, ImageData, ImageId, NextIds, Presentation, Slide, SlideSize, VideoData,
    VideoId,
};

/// The extension of Sliderino files, without the dot.
pub const EXTENSION: &str = "sldr";

/// The format that [`save`] writes. [`load`] reads it and older ones.
pub const FORMAT: u32 = 1;

const MANIFEST: &str = "presentation.json";

#[derive(Debug)]
pub enum FileError {
    Io(std::io::Error),
    /// Not a zip archive with a `presentation.json`.
    NotSliderino,
    /// Made by a newer Sliderino.
    NewerFormat(u32),
    /// `presentation.json` is not valid.
    Invalid(String),
    /// An entry that `presentation.json` lists is not in the archive.
    MissingEntry(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Io(error) => write!(f, "{error}"),
            FileError::NotSliderino => write!(f, "not a Sliderino file"),
            FileError::NewerFormat(format) => write!(
                f,
                "the file has format {format}, made by a newer Sliderino; this one reads up to format {FORMAT}"
            ),
            FileError::Invalid(message) => write!(f, "the file is damaged: {message}"),
            FileError::MissingEntry(name) => write!(f, "the file is damaged: {name} is missing"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<std::io::Error> for FileError {
    fn from(error: std::io::Error) -> Self {
        FileError::Io(error)
    }
}

impl From<zip::result::ZipError> for FileError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => FileError::Io(error),
            _ => FileError::NotSliderino,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    size: Size,
    next_ids: NextIds,
    #[serde(default)]
    fonts: Vec<FontEntry>,
    #[serde(default)]
    images: Vec<ImageEntry>,
    #[serde(default)]
    videos: Vec<VideoEntry>,
    slides: Vec<Slide>,
}

#[derive(Serialize, Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize)]
struct FontEntry {
    face: FontFace,
    /// Face index inside a font collection.
    #[serde(default)]
    index: u32,
    file: String,
}

#[derive(Serialize, Deserialize)]
struct ImageEntry {
    id: ImageId,
    file: String,
}

#[derive(Serialize, Deserialize)]
struct VideoEntry {
    id: VideoId,
    width: u32,
    height: u32,
    duration: f32,
    has_audio: bool,
    file: String,
}

/// `path` with the `.sldr` extension added when it has none.
pub fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(EXTENSION)
    }
}

/// Writes the presentation to `path`. The file is written next to it
/// first, then renamed, so a failed save leaves the old file as it was.
pub fn save(presentation: &Presentation, path: &Path) -> Result<(), FileError> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let result = std::fs::File::create(&partial)
        .map_err(FileError::from)
        .and_then(|file| write(presentation, file))
        .and_then(|()| std::fs::rename(&partial, path).map_err(FileError::from));
    if result.is_err() {
        std::fs::remove_file(&partial).ok();
    }
    result
}

fn write(presentation: &Presentation, file: std::fs::File) -> Result<(), FileError> {
    let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(false);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // Faces of one collection share their bytes: write them once.
    let mut font_files: Vec<(Arc<[u8]>, String)> = Vec::new();
    let mut fonts = Vec::new();
    for (face, data) in presentation.fonts.iter() {
        let known = font_files
            .iter()
            .find(|(bytes, _)| Arc::ptr_eq(bytes, &data.bytes) || **bytes == *data.bytes);
        let file = match known {
            Some((_, file)) => file.clone(),
            None => {
                let file = format!(
                    "fonts/{}.{}",
                    font_files.len() + 1,
                    font_extension(&data.bytes)
                );
                zip.start_file(&file, deflated)?;
                zip.write_all(&data.bytes)?;
                font_files.push((data.bytes.clone(), file.clone()));
                file
            }
        };
        fonts.push(FontEntry {
            face: face.clone(),
            index: data.index,
            file,
        });
    }

    let mut images = Vec::new();
    for (id, data) in presentation.images.iter() {
        let extension = match data.format {
            crate::images::ImageFormat::Png => "png",
            crate::images::ImageFormat::Jpeg => "jpg",
        };
        let file = format!("images/{}.{extension}", id.0);
        zip.start_file(&file, stored)?;
        zip.write_all(&data.bytes)?;
        images.push(ImageEntry { id, file });
    }

    let mut videos = Vec::new();
    for (id, data) in presentation.videos.iter() {
        let file = format!("videos/{}.mp4", id.0);
        zip.start_file(
            &file,
            stored.large_file(data.bytes.len() >= u32::MAX as usize),
        )?;
        zip.write_all(&data.bytes)?;
        videos.push(VideoEntry {
            id,
            width: data.width,
            height: data.height,
            duration: data.duration,
            has_audio: data.has_audio,
            file,
        });
    }

    let manifest = Manifest {
        format: FORMAT,
        size: Size {
            width: presentation.size.width,
            height: presentation.size.height,
        },
        next_ids: presentation.next_ids(),
        fonts,
        images,
        videos,
        slides: presentation.slides.clone(),
    };
    zip.start_file(MANIFEST, deflated)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)
        .map_err(|error| FileError::Invalid(error.to_string()))?;
    let mut writer = zip.finish()?;
    writer.flush()?;
    writer
        .into_inner()
        .map_err(|error| FileError::Io(error.into_error()))?
        .sync_all()?;
    Ok(())
}

fn font_extension(bytes: &[u8]) -> &'static str {
    match bytes.get(..4) {
        Some(b"OTTO") => "otf",
        Some(b"ttcf") => "ttc",
        _ => "ttf",
    }
}

/// Reads a presentation from `path`.
pub fn load(path: &Path) -> Result<Presentation, FileError> {
    let file = std::fs::File::open(path)?;
    read(std::io::BufReader::new(file))
}

fn read<R: std::io::Read + Seek>(reader: R) -> Result<Presentation, FileError> {
    let mut zip = ZipArchive::new(reader)?;
    let manifest: Manifest = {
        let entry = zip.by_name(MANIFEST).map_err(|error| match error {
            zip::result::ZipError::FileNotFound => FileError::NotSliderino,
            error => error.into(),
        })?;
        let value: serde_json::Value = serde_json::from_reader(entry)
            .map_err(|error| FileError::Invalid(error.to_string()))?;
        let format = value
            .get("format")
            .and_then(serde_json::Value::as_u64)
            .ok_or(FileError::NotSliderino)?;
        if format > u64::from(FORMAT) {
            return Err(FileError::NewerFormat(
                u32::try_from(format).unwrap_or(u32::MAX),
            ));
        }
        serde_json::from_value(value).map_err(|error| FileError::Invalid(error.to_string()))?
    };

    let mut entry_bytes = |name: &str| -> Result<Arc<[u8]>, FileError> {
        let mut entry = zip.by_name(name).map_err(|error| match error {
            zip::result::ZipError::FileNotFound => FileError::MissingEntry(name.into()),
            error => error.into(),
        })?;
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        entry.read_to_end(&mut bytes)?;
        Ok(Arc::from(bytes))
    };

    let mut font_files: Vec<(String, Arc<[u8]>)> = Vec::new();
    let mut fonts = Vec::new();
    for font in manifest.fonts {
        let bytes = match font_files.iter().find(|(file, _)| *file == font.file) {
            Some((_, bytes)) => bytes.clone(),
            None => {
                let bytes = entry_bytes(&font.file)?;
                font_files.push((font.file, bytes.clone()));
                bytes
            }
        };
        fonts.push((
            font.face,
            FontData {
                bytes,
                index: font.index,
            },
        ));
    }

    let mut images = Vec::new();
    for image in manifest.images {
        let data = ImageData::read(entry_bytes(&image.file)?)
            .map_err(|error| FileError::Invalid(format!("{}: {error:?}", image.file)))?;
        images.push((image.id, data));
    }

    let mut videos = Vec::new();
    for video in manifest.videos {
        let bytes = entry_bytes(&video.file)?;
        videos.push((
            video.id,
            VideoData {
                bytes,
                width: video.width,
                height: video.height,
                duration: video.duration,
                has_audio: video.has_audio,
            },
        ));
    }

    let size = SlideSize {
        width: manifest.size.width,
        height: manifest.size.height,
    };
    Presentation::from_saved(
        size,
        fonts,
        images,
        videos,
        manifest.slides,
        manifest.next_ids,
    )
    .map_err(|error| FileError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{add_text, with_inter};
    use crate::document::{
        Element, ElementKind, Fill, Frame, ImageFill, Operation, RectangleElement, TextSizing,
    };

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sliderino-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn png() -> Arc<[u8]> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        Arc::from(bytes)
    }

    /// A presentation with a text, a rectangle filled with an image, an
    /// embedded video and a second slide.
    fn sample() -> Presentation {
        let mut presentation = with_inter();
        add_text(
            &mut presentation,
            "Hello",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let image = presentation.new_image_id();
        presentation
            .apply(Operation::AddImage {
                id: image,
                data: ImageData::read(png()).unwrap(),
            })
            .unwrap();
        let video = presentation.new_video_id();
        presentation
            .apply(Operation::AddVideo {
                id: video,
                data: VideoData {
                    bytes: Arc::from(&b"not really an mp4"[..]),
                    width: 640,
                    height: 360,
                    duration: 2.5,
                    has_audio: true,
                },
            })
            .unwrap();
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        let rectangle = RectangleElement {
            fill: Fill::Image(ImageFill {
                id: image,
                fit: Default::default(),
                opacity: 1.,
            }),
            ..RectangleElement::default()
        };
        presentation
            .apply(Operation::AddElement {
                slide,
                parent: None,
                index: usize::MAX,
                element: Element::new(
                    id,
                    Frame {
                        x: 100.,
                        y: 100.,
                        width: 300.,
                        height: 200.,
                        rotation: 0.,
                    },
                    ElementKind::Rectangle(rectangle),
                ),
            })
            .unwrap();
        let second = presentation.new_slide_id();
        presentation
            .apply(Operation::AddSlide {
                index: usize::MAX,
                slide: Slide::new(second),
            })
            .unwrap();
        presentation
    }

    #[test]
    fn a_saved_presentation_loads_the_same() {
        let presentation = sample();
        let path = temp_path("round-trip.sldr");
        save(&presentation, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, presentation);
        assert_eq!(loaded.next_ids(), presentation.next_ids());
        assert_eq!(loaded.revision(), 0);
        let (_, video) = loaded.videos.iter().next().unwrap();
        assert_eq!((video.width, video.height, video.duration), (640, 360, 2.5));
        assert!(video.has_audio);
    }

    #[test]
    fn a_saved_table_loads_the_same() {
        let mut presentation = with_inter();
        let mut table = crate::table::TableElement::new(2, 3);
        table.rows[0][0].content = "Name".into();
        table.rows[1][2].fill = Some(crate::document::Fill::None);
        table.width = crate::table::TableSizing::Fixed(500.);
        table
            .merge(&crate::table::CellRange::new(0..1, 1..3))
            .unwrap();
        table
            .set_borders(
                &table.all(),
                crate::table::Sides::Outside,
                crate::table::Edge::None,
            )
            .unwrap();
        crate::document::tests::add_shape(
            &mut presentation,
            Frame::default(),
            crate::document::ElementKind::Table(Box::new(table)),
        );
        let path = temp_path("table.sldr");
        save(&presentation, &path).unwrap();
        assert_eq!(load(&path).unwrap(), presentation);
    }

    #[test]
    fn ids_are_not_reused_after_a_load() {
        let mut presentation = sample();
        // Removed ids stay used.
        let id = presentation.new_element_id();
        let path = temp_path("ids.sldr");
        save(&presentation, &path).unwrap();
        let mut loaded = load(&path).unwrap();
        assert!(loaded.new_element_id().0 > id.0);
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_refused() {
        let path = temp_path("text.sldr");
        std::fs::write(&path, "hello").unwrap();
        assert!(matches!(load(&path), Err(FileError::NotSliderino)));
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> std::io::Cursor<Vec<u8>> {
        let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut cursor = zip.finish().unwrap();
        cursor.set_position(0);
        cursor
    }

    fn manifest_of(presentation: &Presentation, name: &str) -> serde_json::Value {
        let path = temp_path(name);
        save(presentation, &path).unwrap();
        let mut zip = ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        serde_json::from_reader(zip.by_name(MANIFEST).unwrap()).unwrap()
    }

    #[test]
    fn a_newer_format_is_refused() {
        let mut manifest = manifest_of(&Presentation::new(), "newer.sldr");
        manifest["format"] = serde_json::json!(FORMAT + 1);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(matches!(
            read(zip_with(&[(MANIFEST, &bytes)])),
            Err(FileError::NewerFormat(format)) if format == FORMAT + 1
        ));
    }

    #[test]
    fn a_missing_entry_is_reported() {
        let manifest = manifest_of(&with_inter(), "missing-entry.sldr");
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let error = read(zip_with(&[(MANIFEST, &bytes)])).unwrap_err();
        assert!(matches!(error, FileError::MissingEntry(name) if name.starts_with("fonts/")));
    }

    #[test]
    fn an_element_that_uses_a_missing_image_is_refused() {
        let presentation = sample();
        let mut manifest = manifest_of(&presentation, "missing-image.sldr");
        manifest["images"] = serde_json::json!([]);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let inter = presentation.fonts.iter().next().unwrap().1.bytes.clone();
        let error = read(zip_with(&[
            (MANIFEST, &bytes),
            ("fonts/1.ttf", &inter),
            ("videos/1.mp4", b"not really an mp4"),
        ]))
        .unwrap_err();
        assert!(matches!(error, FileError::Invalid(_)), "{error:?}");
    }

    #[test]
    fn with_extension_adds_sldr_when_missing() {
        assert_eq!(
            with_extension(Path::new("/a/deck")),
            Path::new("/a/deck.sldr")
        );
        assert_eq!(
            with_extension(Path::new("/a/deck.sldr")),
            Path::new("/a/deck.sldr")
        );
    }
}
