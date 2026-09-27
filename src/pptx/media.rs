//! The media parts of the deck: images, and the videos and posters of
//! video and shader fills. Each is written once and shared by the slides.

use std::collections::HashMap;
use std::io::Cursor;

use crate::document::{ImageId, Presentation, ShaderFill, VideoId};
use crate::images::{ImageData, ImageFormat};

use super::package::Package;

/// The media parts, and the name each image, video and shader has among
/// them.
#[derive(Default)]
pub struct Media {
    parts: Vec<(String, Vec<u8>)>,
    images: HashMap<ImageId, String>,
    videos: HashMap<VideoId, Clip>,
    shaders: Vec<(ShaderKey, Clip)>,
}

/// A video and its poster frame, as targets from a slide part.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub video: String,
    pub poster: String,
    /// Pixels of the poster.
    pub size: (u32, u32),
    /// Seconds.
    pub duration: f32,
}

/// What makes two shader videos the same.
#[derive(PartialEq)]
struct ShaderKey {
    source: std::sync::Arc<str>,
    channel0: Option<ImageId>,
    size: (u32, u32),
    duration: f32,
}

/// Frames per second of the videos of shader fills.
pub const SHADER_FPS: u32 = 30;

impl Media {
    /// The target of image `id` from a slide part, such as
    /// `../media/image1.png`. The image is added on first use.
    pub fn image(&mut self, presentation: &Presentation, id: ImageId) -> Result<String, String> {
        if let Some(target) = self.images.get(&id) {
            return Ok(target.clone());
        }
        let data = presentation
            .images
            .get(id)
            .ok_or_else(|| format!("image {} is not in the presentation", id.0))?;
        let (bytes, extension) =
            upright(data).ok_or_else(|| format!("image {} cannot be read", id.0))?;
        let target = self.add("image", extension, bytes);
        self.images.insert(id, target.clone());
        Ok(target)
    }

    fn add(&mut self, stem: &str, extension: &str, bytes: Vec<u8>) -> String {
        let name = format!("{stem}{}.{extension}", self.parts.len() + 1);
        self.parts.push((format!("ppt/media/{name}"), bytes));
        format!("../media/{name}")
    }

    /// Video `id` and its first frame. Each video is added once.
    pub fn video(&mut self, presentation: &Presentation, id: VideoId) -> Result<Clip, String> {
        if let Some(clip) = self.videos.get(&id) {
            return Ok(clip.clone());
        }
        let data = presentation
            .videos
            .get(id)
            .ok_or_else(|| format!("video {} is not in the presentation", id.0))?;
        let poster = crate::videos::poster(data)
            .ok_or_else(|| format!("the first frame of video {} cannot be read", id.0))?;
        let png = poster
            .encode_png()
            .map_err(|error| format!("the first frame of video {}: {error}", id.0))?;
        let clip = Clip {
            video: self.add("media", "mp4", data.bytes.to_vec()),
            poster: self.add("image", "png", png),
            size: (poster.width(), poster.height()),
            duration: data.duration,
        };
        self.videos.insert(id, clip.clone());
        Ok(clip)
    }

    /// The video of a shader fill on a box of `width` × `height` units, from
    /// time 0 to its duration, and its frame at time 0. The same shader on
    /// a box of the same size is rendered once.
    pub fn shader(
        &mut self,
        presentation: &Presentation,
        shader: &ShaderFill,
        width: f32,
        height: f32,
    ) -> Result<Clip, String> {
        let size =
            crate::shaders::clamp_size(width.round().max(1.) as u32, height.round().max(1.) as u32);
        let key = ShaderKey {
            source: shader.source.clone(),
            channel0: shader.channel0,
            size,
            duration: shader.duration,
        };
        if let Some((_, clip)) = self.shaders.iter().find(|(known, _)| *known == key) {
            return Ok(clip.clone());
        }
        if !crate::shaders::available() {
            return Err("no GPU renders the shader".to_string());
        }
        let channel0 = match shader.channel0 {
            Some(image) => Some(
                presentation
                    .images
                    .get(image)
                    .and_then(crate::images::pixels)
                    .ok_or_else(|| format!("image {} cannot be read", image.0))?,
            ),
            None => None,
        };
        let poster = crate::shaders::render(
            &shader.source,
            channel0.as_ref(),
            size.0,
            size.1,
            crate::shaders::Inputs::at(0.),
        )
        .map_err(|error| format!("the shader does not render: {error}"))?;
        let job = crate::shaders::VideoJob {
            source: shader.source.clone(),
            channel0,
            width: size.0,
            height: size.1,
            fps: SHADER_FPS,
            duration: shader.duration,
        };
        let video = job
            .encode()
            .map_err(|error| format!("the shader video cannot be made: {error}"))?;
        let png = poster
            .encode_png()
            .map_err(|error| format!("the shader frame: {error}"))?;
        let clip = Clip {
            video: self.add("media", "mp4", video.to_vec()),
            poster: self.add("image", "png", png),
            size: (poster.width(), poster.height()),
            duration: shader.duration,
        };
        self.shaders.push((key, clip.clone()));
        Ok(clip)
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
