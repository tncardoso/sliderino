//! The media parts of the deck: images, and the videos and posters of
//! video and shader fills. Each is written once and shared by the slides.

use std::collections::HashMap;
use std::io::Cursor;

use crate::document::{ImageId, Presentation};
use crate::images::{ImageData, ImageFormat};

use super::package::Package;

/// The media parts, and the name each image has among them.
#[derive(Default)]
pub struct Media {
    parts: Vec<(String, Vec<u8>)>,
    images: HashMap<ImageId, String>,
}

impl Media {
    /// The target of image `id` from a slide part, such as
    /// `../media/image1.png`. The image is added on first use.
    pub fn image(&mut self, presentation: &Presentation, id: ImageId) -> Result<String, String> {
        if let Some(name) = self.images.get(&id) {
            return Ok(format!("../media/{name}"));
        }
        let data = presentation
            .images
            .get(id)
            .ok_or_else(|| format!("image {} is not in the presentation", id.0))?;
        let (bytes, extension) =
            upright(data).ok_or_else(|| format!("image {} cannot be read", id.0))?;
        let name = format!("image{}.{extension}", self.parts.len() + 1);
        self.parts.push((format!("ppt/media/{name}"), bytes));
        self.images.insert(id, name.clone());
        Ok(format!("../media/{name}"))
    }

    pub fn write(self, package: &mut Package) {
        for (name, bytes) in self.parts {
            package.binary(name, bytes, false);
        }
    }
}

/// The bytes of an image as PowerPoint shows it upright, and their file
/// extension. PowerPoint does not read the EXIF orientation of a JPEG, so a
/// turned image is decoded, turned and encoded again.
fn upright(data: &ImageData) -> Option<(Vec<u8>, &'static str)> {
    let extension = match data.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
    };
    if data.orientation <= 1 {
        return Some((data.bytes.to_vec(), extension));
    }
    let pixmap = data.decode(u32::MAX)?;
    match data.format {
        ImageFormat::Png => Some((pixmap.encode_png().ok()?, "png")),
        ImageFormat::Jpeg => {
            let rgb: Vec<u8> = pixmap
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let color = pixel.demultiply();
                    [color.red(), color.green(), color.blue()]
                })
                .collect();
            let mut out = Cursor::new(Vec::new());
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
                .encode(
                    &rgb,
                    pixmap.width(),
                    pixmap.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .ok()?;
            Some((out.into_inner(), "jpeg"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn an_upright_image_keeps_its_bytes() {
        let bytes: Arc<[u8]> = crate::images::tests::png(4, 2);
        let data = ImageData::read(bytes.clone()).unwrap();
        let (out, extension) = upright(&data).unwrap();
        assert_eq!(out, bytes.to_vec());
        assert_eq!(extension, "png");
    }

    #[test]
    fn a_turned_jpeg_is_encoded_upright() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("debug/scenes/assets/rotated.jpg");
        let data = ImageData::read(std::fs::read(path).unwrap().into()).unwrap();
        assert!(data.orientation > 1);
        let (out, extension) = upright(&data).unwrap();
        assert_eq!(extension, "jpeg");
        let again = ImageData::read(out.into()).unwrap();
        assert_eq!(again.orientation, 1);
        assert_eq!((again.width, again.height), (data.width, data.height));
    }
}
