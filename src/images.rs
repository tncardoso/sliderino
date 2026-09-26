//! Images embedded in a presentation: their bytes as the author gave them,
//! checked once when added, and their decoded pixels, kept in a small cache
//! shared by the renderers.
//!
//! Only PNG and JPEG are embedded: PDF, PPTX and HTML all take them as they
//! are. The EXIF orientation of a JPEG is kept with its bytes and applied
//! when the image is decoded, so the sizes given here are those of the
//! upright image.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use image::{ImageDecoder as _, ImageReader, metadata::Orientation};
use tiny_skia::Pixmap;

pub use crate::style::ImageId;

/// Most pixels on a side of an embedded image.
pub const MAX_SIDE: u32 = 16384;

/// Most bytes of an embedded image.
pub const MAX_BYTES: usize = 64 << 20;

/// Most pixels on a side of a decoded image: larger ones are scaled down,
/// which is more than a slide needs.
pub const DECODED_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG or a JPEG.
    Unsupported,
    /// Larger than [`MAX_SIDE`] or [`MAX_BYTES`].
    TooLarge,
    /// A PNG or JPEG that cannot be read.
    Unreadable,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Unsupported => write!(f, "only PNG and JPEG images are supported"),
            ImageError::TooLarge => write!(
                f,
                "the image is too large: at most {MAX_SIDE} pixels on a side and {} MiB",
                MAX_BYTES >> 20
            ),
            ImageError::Unreadable => write!(f, "the image cannot be read"),
        }
    }
}

impl std::error::Error for ImageError {}

/// The bytes of one embedded image and what its header says.
#[derive(Clone)]
pub struct ImageData {
    pub bytes: Arc<[u8]>,
    pub format: ImageFormat,
    /// Size of the upright image, in pixels.
    pub width: u32,
    pub height: u32,
    /// EXIF orientation, 1 to 8; 1 is upright.
    pub orientation: u8,
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes
    }
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageData")
            .field("bytes", &format_args!("{} bytes", self.bytes.len()))
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("orientation", &self.orientation)
            .finish()
    }
}

impl ImageData {
    /// Checks the bytes of a PNG or a JPEG from its header, without
    /// decoding its pixels.
    pub fn read(bytes: Arc<[u8]>) -> Result<ImageData, ImageError> {
        if bytes.len() > MAX_BYTES {
            return Err(ImageError::TooLarge);
        }
        let (format, (width, height), orientation) = {
            let reader = ImageReader::new(Cursor::new(&bytes[..]))
                .with_guessed_format()
                .map_err(|_| ImageError::Unreadable)?;
            let format = match reader.format() {
                Some(image::ImageFormat::Png) => ImageFormat::Png,
                Some(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
                _ => return Err(ImageError::Unsupported),
            };
            let mut decoder = reader.into_decoder().map_err(|_| ImageError::Unreadable)?;
            let orientation = decoder
                .orientation()
                .unwrap_or(Orientation::NoTransforms)
                .to_exif();
            (format, decoder.dimensions(), orientation)
        };
        if width == 0 || height == 0 {
            return Err(ImageError::Unreadable);
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(ImageError::TooLarge);
        }
        // Orientations 5 to 8 turn the image by a quarter.
        let (width, height) = if orientation >= 5 {
            (height, width)
        } else {
            (width, height)
        };
        Ok(ImageData {
            bytes,
            format,
            width,
            height,
            orientation,
        })
    }

    /// Decodes the upright image, scaled down to at most `max_side` pixels
    /// on a side, as premultiplied pixels.
    pub fn decode(&self, max_side: u32) -> Option<Pixmap> {
        let _span = crate::perf::span("decode_image");
        let format = match self.format {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        };
        let mut image = image::load_from_memory_with_format(&self.bytes, format).ok()?;
        if let Some(orientation) = Orientation::from_exif(self.orientation) {
            image.apply_orientation(orientation);
        }
        let (width, height) = (image.width(), image.height());
        let largest = width.max(height);
        let image = if largest > max_side {
            let scale = max_side as f32 / largest as f32;
            image.resize(
                ((width as f32 * scale).round() as u32).max(1),
                ((height as f32 * scale).round() as u32).max(1),
                image::imageops::FilterType::Triangle,
            )
        } else {
            image
        };
        let rgba = image.into_rgba8();
        let (width, height) = rgba.dimensions();
        let mut pixmap = Pixmap::new(width, height)?;
        for (pixel, rgba) in pixmap.pixels_mut().iter_mut().zip(rgba.pixels()) {
            let [r, g, b, a] = rgba.0;
            *pixel = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
        }
        Some(pixmap)
    }
}

/// Images embedded in the presentation, so it opens anywhere with them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageLibrary {
    pub(crate) images: BTreeMap<ImageId, ImageData>,
}

impl ImageLibrary {
    pub fn get(&self, id: ImageId) -> Option<&ImageData> {
        self.images.get(&id)
    }

    pub fn contains(&self, id: ImageId) -> bool {
        self.images.contains_key(&id)
    }

    /// The embedded images, by id.
    pub fn iter(&self) -> impl Iterator<Item = (ImageId, &ImageData)> {
        self.images.iter().map(|(id, data)| (*id, data))
    }

    /// The image holding exactly these bytes, if one does.
    pub fn find(&self, bytes: &[u8]) -> Option<ImageId> {
        self.images
            .iter()
            .find(|(_, data)| &data.bytes[..] == bytes)
            .map(|(id, _)| *id)
    }
}

/// Decoded images, most recently used last. Each entry holds the bytes it
/// came from, so their address stays a valid key.
type Decoded = Vec<(Arc<[u8]>, Arc<Pixmap>)>;

static DECODED: Mutex<Decoded> = Mutex::new(Vec::new());

/// Most bytes of decoded pixels kept: sixteen of the largest images.
const DECODED_BYTES: usize = 16 * (DECODED_SIDE as usize * DECODED_SIDE as usize * 4);

fn lookup(bytes: &Arc<[u8]>) -> Option<Arc<Pixmap>> {
    let mut decoded = DECODED.lock().ok()?;
    let index = decoded
        .iter()
        .position(|(key, _)| Arc::ptr_eq(key, bytes))?;
    let entry = decoded.remove(index);
    let pixels = entry.1.clone();
    decoded.push(entry);
    Some(pixels)
}

/// The decoded pixels of an image when the cache has them. Never decodes.
pub fn cached_pixels(data: &ImageData) -> Option<Arc<Pixmap>> {
    lookup(&data.bytes)
}

/// The decoded pixels of an image, decoding it when the cache does not
/// have them; `None` when it cannot be decoded.
pub fn pixels(data: &ImageData) -> Option<Arc<Pixmap>> {
    if let Some(pixels) = lookup(&data.bytes) {
        return Some(pixels);
    }
    let pixels = Arc::new(data.decode(DECODED_SIDE)?);
    if let Ok(mut decoded) = DECODED.lock() {
        decoded.push((data.bytes.clone(), pixels.clone()));
        let size = |pixels: &Pixmap| pixels.data().len();
        let mut total: usize = decoded.iter().map(|(_, pixels)| size(pixels)).sum();
        while total > DECODED_BYTES && decoded.len() > 1 {
            total -= size(&decoded.remove(0).1);
        }
    }
    Some(pixels)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG of `width` × `height` pixels, red on the left half and blue on
    /// the right.
    pub fn png(width: u32, height: u32) -> Arc<[u8]> {
        let image = image::RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        Arc::from(bytes)
    }

    pub fn jpeg(width: u32, height: u32) -> Arc<[u8]> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([0, 128, 0]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
            .unwrap();
        Arc::from(bytes)
    }

    #[test]
    fn reads_png_and_jpeg_headers() {
        let png = ImageData::read(png(40, 20)).unwrap();
        assert_eq!(
            (png.format, png.width, png.height, png.orientation),
            (ImageFormat::Png, 40, 20, 1)
        );
        let jpeg = ImageData::read(jpeg(8, 6)).unwrap();
        assert_eq!(
            (jpeg.format, jpeg.width, jpeg.height),
            (ImageFormat::Jpeg, 8, 6)
        );
    }

    #[test]
    fn rejects_other_formats_and_garbage() {
        let gif: Arc<[u8]> = Arc::from(&b"GIF89a\x01\x00\x01\x00\x00\x00\x00;"[..]);
        assert_eq!(ImageData::read(gif).unwrap_err(), ImageError::Unsupported);
        let garbage: Arc<[u8]> = Arc::from(&b"not an image at all"[..]);
        assert!(ImageData::read(garbage).is_err());
        let truncated: Arc<[u8]> = Arc::from(&png(10, 10)[..20]);
        assert!(ImageData::read(truncated).is_err());
    }

    #[test]
    fn decodes_upright_and_scaled_down() {
        let mut data = ImageData::read(png(40, 20)).unwrap();
        let pixmap = data.decode(DECODED_SIDE).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (40, 20));
        assert_eq!(pixmap.pixel(0, 0).unwrap().red(), 255);
        let small = data.decode(10).unwrap();
        assert_eq!((small.width(), small.height()), (10, 5));
        // A quarter turn clockwise puts the left half on top.
        data.orientation = 6;
        let turned = data.decode(DECODED_SIDE).unwrap();
        assert_eq!((turned.width(), turned.height()), (20, 40));
        assert_eq!(turned.pixel(10, 0).unwrap().red(), 255);
        assert_eq!(turned.pixel(10, 39).unwrap().blue(), 255);
    }

    #[test]
    fn decoded_pixels_are_cached() {
        let data = ImageData::read(png(4, 4)).unwrap();
        assert!(cached_pixels(&data).is_none());
        let first = pixels(&data).unwrap();
        assert!(Arc::ptr_eq(&first, &cached_pixels(&data).unwrap()));
    }
}
