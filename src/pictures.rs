//! The still pictures that fills paint in the editor: the decoded image of
//! an image fill, the first frame of a video fill and the frame at time 0
//! of a shader fill.
//!
//! The canvas and the thumbnails only read the caches
//! ([`Picture::cached`]); a picture that is not there yet is loaded in the
//! background ([`Load::run`]) and the editor renders again when it is
//! ready.

use std::sync::{Arc, Mutex};

use tiny_skia::Pixmap;

use crate::document::{Fill, Frame, ImageData, ImageId, Presentation, VideoData, VideoId};

/// A still picture of a fill.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Picture {
    Image(ImageId),
    /// The first frame of a video.
    Poster(VideoId),
    /// A shader at time 0, at a size in pixels.
    Shader {
        source: Arc<str>,
        channel0: Option<ImageId>,
        width: u32,
        height: u32,
    },
}

/// Pixels of a shader still per slide unit: slides are about 1920 units
/// wide, so a still is sharp at 100 % zoom.
const SHADER_SCALE: f32 = 1.;

/// Shader stills are rendered at sizes rounded up to this many pixels, so
/// that a resize does not render a new still at each step.
const SHADER_STEP: u32 = 64;

impl Picture {
    /// The still picture that a fill paints in a frame; `None` for fills
    /// without one.
    pub fn of(fill: &Fill, frame: &Frame) -> Option<Picture> {
        match fill {
            Fill::Image(image) => Some(Picture::Image(image.id)),
            Fill::Video(video) => Some(Picture::Poster(video.id)),
            Fill::Shader(shader) => {
                let step = |value: f32| {
                    let pixels = (value * SHADER_SCALE).max(1.).ceil() as u32;
                    pixels.div_ceil(SHADER_STEP) * SHADER_STEP
                };
                let (width, height) =
                    crate::shaders::clamp_size(step(frame.width), step(frame.height));
                Some(Picture::Shader {
                    source: shader.source.clone(),
                    channel0: shader.channel0,
                    width,
                    height,
                })
            }
            _ => None,
        }
    }

    /// The picture when a cache has it. Never decodes or renders. A shader
    /// still of another size stands in for a missing one.
    pub fn cached(&self, presentation: &Presentation) -> Option<Arc<Pixmap>> {
        match self {
            Picture::Image(id) => crate::images::cached_pixels(presentation.images.get(*id)?),
            Picture::Poster(id) => crate::videos::cached_poster(presentation.videos.get(*id)?),
            Picture::Shader {
                source,
                channel0,
                width,
                height,
            } => {
                let channel = match channel0 {
                    Some(id) => Some(crate::images::cached_pixels(presentation.images.get(*id)?)?),
                    None => None,
                };
                cached_still(source, channel.as_ref(), (*width, *height))
            }
        }
    }

    /// What loading the picture needs, owned so that it can move to a
    /// background thread; `None` when the presentation does not embed it.
    pub fn load(&self, presentation: &Presentation) -> Option<Load> {
        Some(match self {
            Picture::Image(id) => Load::Image(presentation.images.get(*id)?.clone()),
            Picture::Poster(id) => Load::Poster(presentation.videos.get(*id)?.clone()),
            Picture::Shader {
                source,
                channel0,
                width,
                height,
            } => Load::Shader {
                source: source.clone(),
                channel0: match channel0 {
                    Some(id) => Some(presentation.images.get(*id)?.clone()),
                    None => None,
                },
                size: (*width, *height),
            },
        })
    }
}

/// The data to load a [`Picture`].
pub enum Load {
    Image(ImageData),
    Poster(VideoData),
    Shader {
        source: Arc<str>,
        channel0: Option<ImageData>,
        size: (u32, u32),
    },
}

impl Load {
    /// Decodes or renders the picture into its cache. Returns whether it
    /// could. Slow: call it off the UI thread.
    pub fn run(self) -> bool {
        match self {
            Load::Image(data) => crate::images::pixels(&data).is_some(),
            Load::Poster(data) => crate::videos::poster(&data).is_some(),
            Load::Shader {
                source,
                channel0,
                size,
            } => {
                let channel = match &channel0 {
                    Some(data) => match crate::images::pixels(data) {
                        Some(pixels) => Some(pixels),
                        None => return false,
                    },
                    None => None,
                };
                still(&source, channel.as_ref(), size).is_some()
            }
        }
    }
}

/// A shader still: the hash of the source, the address of the channel
/// pixels (0 for none), the size, and the pixels. Each entry keeps the
/// channel pixels alive so that the address stays a valid key.
type Still = (u64, Option<Arc<Pixmap>>, (u32, u32), Arc<Pixmap>);

static STILLS: Mutex<Vec<Still>> = Mutex::new(Vec::new());

/// Most shader stills kept.
const STILLS_KEPT: usize = 32;

fn source_hash(source: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn same_channel(a: Option<&Arc<Pixmap>>, b: Option<&Arc<Pixmap>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// The still of a shader at `size` when the cache has it, or else one at
/// another size.
fn cached_still(
    source: &str,
    channel: Option<&Arc<Pixmap>>,
    size: (u32, u32),
) -> Option<Arc<Pixmap>> {
    let hash = source_hash(source);
    let stills = STILLS.lock().ok()?;
    let mut matching = stills.iter().rev().filter(|(key, key_channel, _, _)| {
        *key == hash && same_channel(key_channel.as_ref(), channel)
    });
    let exact = matching
        .clone()
        .find(|(_, _, key_size, _)| *key_size == size);
    exact
        .or_else(|| matching.next())
        .map(|(_, _, _, pixels)| pixels.clone())
}

/// Renders the still of a shader at time 0 into the cache.
fn still(source: &str, channel: Option<&Arc<Pixmap>>, size: (u32, u32)) -> Option<Arc<Pixmap>> {
    let hash = source_hash(source);
    if let Ok(stills) = STILLS.lock()
        && let Some(found) = stills.iter().find(|(key, key_channel, key_size, _)| {
            *key == hash && same_channel(key_channel.as_ref(), channel) && *key_size == size
        })
    {
        return Some(found.3.clone());
    }
    let inputs = crate::shaders::Inputs::at(0.);
    let pixels = Arc::new(crate::shaders::render(source, channel, size.0, size.1, inputs).ok()?);
    if let Ok(mut stills) = STILLS.lock() {
        stills.push((hash, channel.cloned(), size, pixels.clone()));
        if stills.len() > STILLS_KEPT {
            stills.remove(0);
        }
    }
    Some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ImageFill, ImageFit, ShaderFill};

    #[test]
    fn shader_stills_round_their_size_up() {
        let fill = Fill::Shader(ShaderFill::default());
        let frame = Frame {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 130.,
            rotation: 0.,
        };
        match Picture::of(&fill, &frame) {
            Some(Picture::Shader { width, height, .. }) => assert_eq!((width, height), (128, 192)),
            other => panic!("{other:?}"),
        }
        let image = Fill::Image(ImageFill {
            id: ImageId(3),
            fit: ImageFit::Cover,
            opacity: 1.,
        });
        assert_eq!(
            Picture::of(&image, &frame),
            Some(Picture::Image(ImageId(3)))
        );
        assert_eq!(Picture::of(&Fill::None, &frame), None);
    }

    #[test]
    fn a_shader_still_of_another_size_stands_in() {
        if !crate::shaders::available() {
            eprintln!("no GPU adapter: skipping");
            return;
        }
        let source = "void mainImage(out vec4 c, in vec2 p) { c = vec4(0.25, 0.5, 0.75, 1.0); }\n";
        assert!(cached_still(source, None, (64, 64)).is_none());
        let rendered = still(source, None, (64, 64)).unwrap();
        assert!(Arc::ptr_eq(
            &rendered,
            &cached_still(source, None, (64, 64)).unwrap()
        ));
        let other = cached_still(source, None, (128, 64)).unwrap();
        assert_eq!((other.width(), other.height()), (64, 64));
    }
}
