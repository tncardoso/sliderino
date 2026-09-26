//! Videos embedded in a presentation, read, converted and played with
//! GStreamer.
//!
//! A video is kept as an MP4 with H.264 video and AAC audio (or no audio):
//! PowerPoint and browsers play that format. A video in another format is
//! converted when it is added. The first frame is the poster: PDF, PNG and
//! the editor canvas show it in place of the video.
//!
//! GStreamer reads files, so the bytes of a video are written once to a
//! file in the temporary directory, named by their hash.
//!
//! The GStreamer libraries are linked, but its plugins are found when a
//! video is first used. A missing plugin fails the operation that needs it
//! with [`VideoError::MissingPlugin`]; the rest of the editor works.

use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_pbutils as gst_pbutils;
use gstreamer_pbutils::prelude::*;
use tiny_skia::Pixmap;

pub use crate::style::VideoId;

/// Most bytes of an embedded video.
pub const MAX_BYTES: usize = 512 << 20;

/// Most pixels on a side of a decoded frame.
pub const DECODED_SIDE: u32 = 1920;

/// H.264 encoders, the first installed one is used. Software encoders come
/// first: they work on every machine and with every picture size.
const H264_ENCODERS: [&str; 6] = [
    "x264enc",
    "openh264enc",
    "vah264enc",
    "vah264lpenc",
    "nvh264enc",
    "vtenc_h264",
];

/// AAC encoders, the first installed one is used.
const AAC_ENCODERS: [&str; 4] = ["avenc_aac", "fdkaacenc", "voaacenc", "faac"];

/// Longest wait to read the header of a video.
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// Not a video, or a format GStreamer cannot read.
    Unsupported,
    /// Larger than [`MAX_BYTES`].
    TooLarge,
    /// A video that cannot be read.
    Unreadable,
    /// GStreamer does not start.
    NoGstreamer,
    /// A GStreamer plugin that the operation needs is not installed.
    MissingPlugin(String),
    /// A file of the temporary directory cannot be written or read.
    Io(String),
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoError::Unsupported => write!(f, "the file is not a video that can be read"),
            VideoError::TooLarge => {
                write!(f, "the video is too large: at most {} MiB", MAX_BYTES >> 20)
            }
            VideoError::Unreadable => write!(f, "the video cannot be read"),
            VideoError::NoGstreamer => write!(f, "GStreamer does not start"),
            VideoError::MissingPlugin(what) => write!(
                f,
                "a GStreamer plugin is missing: {what}. Install the GStreamer plugins \
                 (base, good, bad and ugly, or libav)"
            ),
            VideoError::Io(error) => write!(f, "cannot use the temporary video file: {error}"),
        }
    }
}

impl std::error::Error for VideoError {}

/// Starts GStreamer once.
fn init() -> Result<(), VideoError> {
    static INIT: OnceLock<bool> = OnceLock::new();
    if *INIT.get_or_init(|| gst::init().is_ok()) {
        Ok(())
    } else {
        Err(VideoError::NoGstreamer)
    }
}

fn element(factory: &str) -> Result<gst::Element, VideoError> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| VideoError::MissingPlugin(factory.into()))
}

/// The first of these element factories that is installed.
fn first_installed(factories: &[&'static str]) -> Option<&'static str> {
    factories
        .iter()
        .copied()
        .find(|name| gst::ElementFactory::find(name).is_some())
}

/// The plugins that adding, playing and exporting videos need and that are
/// missing; empty when all are there.
pub fn missing_plugins() -> Vec<String> {
    if init().is_err() {
        return vec!["GStreamer".into()];
    }
    let mut missing: Vec<String> = [
        "playbin",
        "uridecodebin",
        "h264parse",
        "aacparse",
        "appsrc",
        "appsink",
        "videoconvert",
        "videoscale",
        "audioconvert",
        "audioresample",
        "mp4mux",
    ]
    .into_iter()
    .filter(|name| gst::ElementFactory::find(name).is_none())
    .map(String::from)
    .collect();
    if first_installed(&H264_ENCODERS).is_none() {
        missing.push("an H.264 encoder (x264enc or openh264enc)".into());
    }
    if first_installed(&AAC_ENCODERS).is_none() {
        missing.push("an AAC encoder (avenc_aac, fdkaacenc or voaacenc)".into());
    }
    missing
}

/// The bytes of one embedded video and what its header says.
#[derive(Clone)]
pub struct VideoData {
    pub bytes: Arc<[u8]>,
    /// Size of the picture, in pixels.
    pub width: u32,
    pub height: u32,
    /// In seconds.
    pub duration: f32,
    pub has_audio: bool,
}

impl PartialEq for VideoData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes
    }
}

impl std::fmt::Debug for VideoData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoData")
            .field("bytes", &format_args!("{} bytes", self.bytes.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("duration", &self.duration)
            .field("has_audio", &self.has_audio)
            .finish()
    }
}

/// What the header of a video file says.
#[derive(Clone, Debug, PartialEq)]
struct Probe {
    width: u32,
    height: u32,
    duration: f32,
    has_audio: bool,
    /// An MP4 with H.264 video and AAC audio or no audio.
    embeddable: bool,
}

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn media_dir() -> PathBuf {
    std::env::temp_dir().join("sliderino-media")
}

/// The file in the temporary directory that holds these bytes, written when
/// it is not there yet.
fn media_file(bytes: &[u8]) -> Result<PathBuf, VideoError> {
    let dir = media_dir();
    let path = dir.join(format!("{:016x}-{}.video", hash(bytes), bytes.len()));
    let complete = std::fs::metadata(&path).is_ok_and(|meta| meta.len() == bytes.len() as u64);
    if !complete {
        std::fs::create_dir_all(&dir).map_err(|error| VideoError::Io(error.to_string()))?;
        // Write to a unique name, then rename, so a reader never sees half
        // a file.
        let partial = dir.join(format!(
            "{:016x}-{}.partial",
            hash(bytes),
            std::process::id()
        ));
        std::fs::write(&partial, bytes).map_err(|error| VideoError::Io(error.to_string()))?;
        std::fs::rename(&partial, &path).map_err(|error| VideoError::Io(error.to_string()))?;
    }
    Ok(path)
}

fn uri(path: &std::path::Path) -> Result<String, VideoError> {
    gst::glib::filename_to_uri(path, None)
        .map(|uri| uri.to_string())
        .map_err(|error| VideoError::Io(error.to_string()))
}

fn probe(path: &std::path::Path) -> Result<Probe, VideoError> {
    init()?;
    let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_nseconds(
        DISCOVER_TIMEOUT.as_nanos() as u64,
    ))
    .map_err(|_| VideoError::MissingPlugin("discoverer".into()))?;
    let info = discoverer
        .discover_uri(&uri(path)?)
        .map_err(|_| VideoError::Unsupported)?;
    match info.result() {
        gst_pbutils::DiscovererResult::Ok => {}
        gst_pbutils::DiscovererResult::MissingPlugins => {
            let details = info.missing_elements_installer_details();
            let what = details
                .iter()
                .map(|detail| detail.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(VideoError::MissingPlugin(what));
        }
        _ => return Err(VideoError::Unsupported),
    }
    let videos = info.video_streams();
    let video = videos.first().ok_or(VideoError::Unsupported)?;
    if video.is_image() {
        return Err(VideoError::Unsupported);
    }
    let (width, height) = (video.width(), video.height());
    if width == 0 || height == 0 {
        return Err(VideoError::Unreadable);
    }
    let caps_name = |stream: &gst_pbutils::DiscovererStreamInfo| {
        stream
            .caps()
            .and_then(|caps| caps.structure(0).map(|s| s.to_owned()))
    };
    let audios = info.audio_streams();
    let container = info
        .stream_info()
        .and_then(|stream| caps_name(&stream))
        .is_some_and(|s| {
            s.name() == "video/quicktime" && s.get::<&str>("variant").is_ok_and(|v| v == "iso")
        });
    let h264 = caps_name(video.upcast_ref()).is_some_and(|s| s.name() == "video/x-h264");
    let aac = audios.iter().all(|audio| {
        caps_name(audio.upcast_ref()).is_some_and(|s| {
            s.name() == "audio/mpeg" && s.get::<i32>("mpegversion").is_ok_and(|v| v == 4)
        })
    });
    Ok(Probe {
        width,
        height,
        duration: info
            .duration()
            .map_or(0., |duration| duration.nseconds() as f32 / 1e9),
        has_audio: !audios.is_empty(),
        embeddable: container && h264 && aac && videos.len() == 1 && audios.len() <= 1,
    })
}

impl VideoData {
    /// Reads the header of a video that is embeddable as it is: an MP4 with
    /// H.264 and AAC. Other videos fail with [`VideoError::Unsupported`];
    /// [`prepare`] converts them.
    pub fn read(bytes: Arc<[u8]>) -> Result<VideoData, VideoError> {
        if bytes.len() > MAX_BYTES {
            return Err(VideoError::TooLarge);
        }
        let probe = probe(&media_file(&bytes)?)?;
        if !probe.embeddable {
            return Err(VideoError::Unsupported);
        }
        Ok(VideoData {
            bytes,
            width: probe.width,
            height: probe.height,
            duration: probe.duration,
            has_audio: probe.has_audio,
        })
    }
}

/// Reads a video in any format GStreamer reads and converts it to MP4 with
/// H.264 and AAC when it is in another format. `progress` gets the done
/// share, 0 to 1, while it converts. Returns the video and whether it was
/// converted.
pub fn prepare(bytes: Arc<[u8]>, progress: &dyn Fn(f32)) -> Result<(VideoData, bool), VideoError> {
    if bytes.len() > MAX_BYTES {
        return Err(VideoError::TooLarge);
    }
    let path = media_file(&bytes)?;
    let found = probe(&path)?;
    if found.embeddable {
        return Ok((
            VideoData {
                bytes,
                width: found.width,
                height: found.height,
                duration: found.duration,
                has_audio: found.has_audio,
            },
            false,
        ));
    }
    let converted = transcode(&path, &found, progress)?;
    if converted.len() > MAX_BYTES {
        return Err(VideoError::TooLarge);
    }
    Ok((VideoData::read(Arc::from(converted))?, true))
}

/// The first installed H.264 encoder, set for `pixels_per_second`: the
/// default bit rates of some encoders are too low for slides.
fn h264_encoder(pixels_per_second: f64) -> Result<gst::Element, VideoError> {
    let name = first_installed(&H264_ENCODERS)
        .ok_or_else(|| VideoError::MissingPlugin("an H.264 encoder".into()))?;
    let encoder = element(name)?;
    // About 0.1 bit per pixel: 6 Mbit/s for 1080p at 30 frames per second.
    let bits = (pixels_per_second * 0.1).clamp(500_000., 20_000_000.);
    match name {
        "openh264enc" => encoder.set_property("bitrate", bits as u32),
        _ if encoder
            .find_property("bitrate")
            .is_some_and(|p| p.value_type() == u32::static_type()) =>
        {
            // x264enc and the hardware encoders count kbit/s.
            encoder.set_property("bitrate", (bits / 1000.) as u32)
        }
        _ => {}
    }
    Ok(encoder)
}

fn aac_encoder() -> Result<gst::Element, VideoError> {
    let name = first_installed(&AAC_ENCODERS)
        .ok_or_else(|| VideoError::MissingPlugin("an AAC encoder".into()))?;
    element(name)
}

/// A filter to the 4:2:0 pictures that players of H.264 all take.
fn i420() -> Result<gst::Element, VideoError> {
    let filter = element("capsfilter")?;
    filter.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("format", "I420")
            .build(),
    );
    Ok(filter)
}

/// Runs a pipeline to its end, calling `progress` with the done share.
fn run_to_end(
    pipeline: &gst::Pipeline,
    duration: f32,
    progress: &dyn Fn(f32),
) -> Result<(), VideoError> {
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| VideoError::Unreadable)?;
    let bus = pipeline.bus().ok_or(VideoError::Unreadable)?;
    let result = loop {
        let message = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(100),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
        match message.as_ref().map(|message| message.view()) {
            Some(gst::MessageView::Eos(_)) => break Ok(()),
            Some(gst::MessageView::Error(error)) => {
                let missing = error
                    .error()
                    .matches(gst::CoreError::MissingPlugin)
                    .then(|| error.error().to_string());
                break Err(missing.map_or(VideoError::Unreadable, VideoError::MissingPlugin));
            }
            _ => {
                if duration > 0.
                    && let Some(position) = pipeline.query_position::<gst::ClockTime>()
                {
                    progress((position.nseconds() as f32 / 1e9 / duration).clamp(0., 1.));
                }
            }
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    result
}

/// A unique file in the temporary directory for an output.
fn output_file(extension: &str) -> Result<PathBuf, VideoError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = media_dir();
    std::fs::create_dir_all(&dir).map_err(|error| VideoError::Io(error.to_string()))?;
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(dir.join(format!("out-{}-{next}.{extension}", std::process::id())))
}

fn read_output(path: &std::path::Path) -> Result<Vec<u8>, VideoError> {
    let bytes = std::fs::read(path).map_err(|error| VideoError::Io(error.to_string()));
    let _ = std::fs::remove_file(path);
    bytes
}

/// Adds elements linked in a chain to the pipeline, links the last one to a
/// new pad of the muxer and returns the sink pad of the first one.
fn chain(
    pipeline: &gst::Pipeline,
    muxer: &gst::Element,
    elements: Vec<gst::Element>,
    request: &str,
) -> Result<gst::Pad, VideoError> {
    pipeline
        .add_many(&elements)
        .map_err(|_| VideoError::Unreadable)?;
    gst::Element::link_many(&elements).map_err(|_| VideoError::Unreadable)?;
    let target = muxer
        .request_pad_simple(request)
        .ok_or(VideoError::Unreadable)?;
    elements
        .last()
        .and_then(|last| last.static_pad("src"))
        .ok_or(VideoError::Unreadable)?
        .link(&target)
        .map_err(|_| VideoError::Unreadable)?;
    elements[0].static_pad("sink").ok_or(VideoError::Unreadable)
}

/// The elements that encode raw video of `width` × `height` to H.264 for
/// the muxer, scaled to its [`encode_size`].
fn video_encoding(width: u32, height: u32, fps: f64) -> Result<Vec<gst::Element>, VideoError> {
    let (width, height) = encode_size(width, height);
    let size = element("capsfilter")?;
    size.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("width", width as i32)
            .field("height", height as i32)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build(),
    );
    Ok(vec![
        element("queue")?,
        element("videoconvert")?,
        element("videoscale")?,
        size,
        i420()?,
        h264_encoder(f64::from(width) * f64::from(height) * fps)?,
        element("h264parse")?,
    ])
}

/// Converts the video file at `path`, described by `found`, to MP4 with
/// H.264 and AAC.
fn transcode(
    path: &std::path::Path,
    found: &Probe,
    progress: &dyn Fn(f32),
) -> Result<Vec<u8>, VideoError> {
    let (has_audio, duration) = (found.has_audio, found.duration);
    let _span = crate::perf::span("transcode_video");
    init()?;
    let output = output_file("mp4")?;
    let pipeline = gst::Pipeline::new();
    let source = element("uridecodebin")?;
    source.set_property("uri", uri(path)?);
    // Hardware decoders can give frames in GPU memory that the converters
    // do not read.
    source.set_property("force-sw-decoders", true);
    let muxer = element("mp4mux")?;
    let sink = element("filesink")?;
    sink.set_property("location", output.to_string_lossy().as_ref());
    pipeline
        .add_many([&source, &muxer, &sink])
        .map_err(|_| VideoError::Unreadable)?;
    muxer.link(&sink).map_err(|_| VideoError::Unreadable)?;
    // The chains exist before the pipeline plays; the decoder pads link to
    // them as they appear.
    let video = chain(
        &pipeline,
        &muxer,
        video_encoding(found.width, found.height, 30.)?,
        "video_%u",
    )?;
    let audio = if has_audio {
        let elements = vec![
            element("queue")?,
            element("audioconvert")?,
            element("audioresample")?,
            aac_encoder()?,
            element("aacparse")?,
        ];
        Some(chain(&pipeline, &muxer, elements, "audio_%u")?)
    } else {
        None
    };
    source.connect_pad_added(move |_, pad| {
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let Some(name) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        let target = if name.starts_with("video/") {
            Some(&video)
        } else if name.starts_with("audio/") {
            audio.as_ref()
        } else {
            None
        };
        if let Some(target) = target
            && !target.is_linked()
        {
            let _ = pad.link(target);
        }
    });
    let result = run_to_end(&pipeline, duration, progress);
    let bytes = read_output(&output);
    result?;
    bytes
}

/// Smallest side of an encoded video: H.264 encoders stretch smaller
/// pictures to their own minimum, which changes the proportions.
const MIN_ENCODED_SIDE: u32 = 256;

/// The size of a video encoded for a picture of `width` × `height`: the
/// same proportions, at least [`MIN_ENCODED_SIDE`] on the smaller side,
/// and even.
pub fn encode_size(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (width.max(1), height.max(1));
    let smallest = width.min(height);
    let (width, height) = if smallest < MIN_ENCODED_SIDE {
        let scale = MIN_ENCODED_SIDE as f32 / smallest as f32;
        (
            (width as f32 * scale).round() as u32,
            (height as f32 * scale).round() as u32,
        )
    } else {
        (width, height)
    };
    (width.div_ceil(2) * 2, height.div_ceil(2) * 2)
}

/// Encodes frames to an MP4 with H.264 and no audio, at `fps` frames per
/// second. The video has the [`encode_size`] of `width` × `height`, and
/// `frame` gives frame number `n` at that size, as premultiplied pixels;
/// the video has `count` frames.
pub fn encode_frames(
    width: u32,
    height: u32,
    fps: u32,
    count: u32,
    frame: &mut dyn FnMut(u32) -> Option<Pixmap>,
) -> Result<Vec<u8>, VideoError> {
    let _span = crate::perf::span("encode_video");
    init()?;
    let (width, height) = encode_size(width, height);
    let fps = fps.max(1);
    let output = output_file("mp4")?;
    let pipeline = gst::Pipeline::new();
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", width as i32)
        .field("height", height as i32)
        .field("framerate", gst::Fraction::new(fps as i32, 1))
        .build();
    let source = gst_app::AppSrc::builder()
        .caps(&caps)
        .format(gst::Format::Time)
        .block(true)
        .build();
    let muxer = element("mp4mux")?;
    let sink = element("filesink")?;
    sink.set_property("location", output.to_string_lossy().as_ref());
    pipeline
        .add_many([source.upcast_ref(), &muxer, &sink])
        .map_err(|_| VideoError::Unreadable)?;
    muxer.link(&sink).map_err(|_| VideoError::Unreadable)?;
    let input = chain(
        &pipeline,
        &muxer,
        video_encoding(width, height, f64::from(fps))?,
        "video_%u",
    )?;
    source
        .static_pad("src")
        .ok_or(VideoError::Unreadable)?
        .link(&input)
        .map_err(|_| VideoError::Unreadable)?;

    let frames = std::thread::scope(|scope| {
        // The pipeline runs while this thread pushes frames.
        let running = scope.spawn(|| run_to_end(&pipeline, 0., &|_| {}));
        let frame_duration = gst::ClockTime::SECOND / u64::from(fps);
        for n in 0..count {
            let Some(pixels) = frame(n) else {
                let _ = source.end_of_stream();
                return running
                    .join()
                    .unwrap_or(Err(VideoError::Unreadable))
                    .and(Err(VideoError::Unreadable));
            };
            let data = straight_rgba(&pixels, width, height);
            let mut buffer = gst::Buffer::from_mut_slice(data);
            {
                let buffer = buffer.get_mut().expect("a new buffer is writable");
                buffer.set_pts(frame_duration * u64::from(n));
                buffer.set_duration(frame_duration);
            }
            if source.push_buffer(buffer).is_err() {
                break;
            }
        }
        let _ = source.end_of_stream();
        running.join().unwrap_or(Err(VideoError::Unreadable))
    });
    let bytes = read_output(&output);
    frames?;
    bytes
}

/// The pixels of a frame as straight RGBA, cropped or padded to the size.
fn straight_rgba(pixels: &Pixmap, width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    for y in 0..height.min(pixels.height()) {
        for x in 0..width.min(pixels.width()) {
            let pixel = pixels.pixel(x, y).expect("inside the pixmap").demultiply();
            let at = ((y * width + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&[
                pixel.red(),
                pixel.green(),
                pixel.blue(),
                pixel.alpha(),
            ]);
        }
    }
    data
}

/// The size of a decoded frame: the video size, scaled down to at most
/// `max_side` pixels on a side.
fn decoded_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let largest = width.max(height).max(1);
    if largest <= max_side {
        return (width.max(1), height.max(1));
    }
    let scale = max_side as f32 / largest as f32;
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

/// A playbin whose video goes to an app sink as RGBA frames of the size.
fn playbin(
    data: &VideoData,
    max_side: u32,
) -> Result<(gst::Element, gst_app::AppSink), VideoError> {
    init()?;
    let (width, height) = decoded_size(data.width, data.height, max_side);
    let playbin = element("playbin")?;
    playbin.set_property("uri", uri(&media_file(&data.bytes)?)?);
    // The defaults without subtitles, and software decoders: hardware ones
    // can give frames in GPU memory that the app sink does not read.
    playbin.set_property_from_str(
        "flags",
        "video+audio+soft-volume+soft-colorbalance+deinterlace+force-sw-decoders",
    );
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", width as i32)
                .field("height", height as i32)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                .build(),
        )
        .max_buffers(1)
        .drop(true)
        .build();
    let bin = gst::Bin::new();
    let convert = element("videoconvert")?;
    let scale = element("videoscale")?;
    bin.add_many([&convert, &scale, sink.upcast_ref()])
        .map_err(|_| VideoError::Unreadable)?;
    gst::Element::link_many([&convert, &scale, sink.upcast_ref()])
        .map_err(|_| VideoError::Unreadable)?;
    let pad = convert.static_pad("sink").ok_or(VideoError::Unreadable)?;
    let ghost = gst::GhostPad::with_target(&pad).map_err(|_| VideoError::Unreadable)?;
    bin.add_pad(&ghost).map_err(|_| VideoError::Unreadable)?;
    playbin.set_property("video-sink", &bin);
    Ok((playbin, sink))
}

/// The frame of a sample as premultiplied pixels.
fn sample_pixels(sample: &gst::Sample) -> Option<Pixmap> {
    let caps = sample.caps()?;
    let info = gstreamer_video::VideoInfo::from_caps(caps).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let (width, height) = (info.width(), info.height());
    let stride = info.stride()[0] as usize;
    let mut pixmap = Pixmap::new(width, height)?;
    let line = width as usize * 4;
    let data = pixmap.data_mut();
    let mut opaque = true;
    for y in 0..height as usize {
        let row = &map[y * stride..y * stride + line];
        data[y * line..(y + 1) * line].copy_from_slice(row);
        opaque &= row.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255);
    }
    if !opaque {
        for pixel in pixmap.pixels_mut() {
            let straight = tiny_skia::ColorU8::from_rgba(
                pixel.red(),
                pixel.green(),
                pixel.blue(),
                pixel.alpha(),
            );
            *pixel = straight.premultiply();
        }
    }
    Some(pixmap)
}

/// Decodes the first frame of a video.
fn decode_poster(data: &VideoData) -> Result<Pixmap, VideoError> {
    let _span = crate::perf::span("decode_poster");
    let (playbin, sink) = playbin(data, DECODED_SIDE)?;
    playbin.set_property("audio-sink", element("fakesink")?);
    playbin
        .set_state(gst::State::Paused)
        .map_err(|_| VideoError::Unreadable)?;
    let sample = sink.try_pull_preroll(gst::ClockTime::from_seconds(10));
    let _ = playbin.set_state(gst::State::Null);
    sample
        .as_ref()
        .and_then(sample_pixels)
        .ok_or(VideoError::Unreadable)
}

/// Decoded posters by the address of the bytes they came from, most
/// recently used last.
type Posters = Vec<(Arc<[u8]>, Arc<Pixmap>)>;

static POSTERS: Mutex<Posters> = Mutex::new(Vec::new());

/// Most posters kept decoded.
const POSTERS_KEPT: usize = 16;

/// The first frame of a video when the cache has it. Never decodes.
pub fn cached_poster(data: &VideoData) -> Option<Arc<Pixmap>> {
    let mut posters = POSTERS.lock().ok()?;
    let index = posters
        .iter()
        .position(|(key, _)| Arc::ptr_eq(key, &data.bytes))?;
    let entry = posters.remove(index);
    let pixels = entry.1.clone();
    posters.push(entry);
    Some(pixels)
}

/// The first frame of a video, decoding it when the cache does not have
/// it; `None` when it cannot be decoded.
pub fn poster(data: &VideoData) -> Option<Arc<Pixmap>> {
    if let Some(pixels) = cached_poster(data) {
        return Some(pixels);
    }
    let pixels = Arc::new(decode_poster(data).ok()?);
    if let Ok(mut posters) = POSTERS.lock() {
        posters.push((data.bytes.clone(), pixels.clone()));
        if posters.len() > POSTERS_KEPT {
            posters.remove(0);
        }
    }
    Some(pixels)
}

/// Plays one video: its frames for a fill, its sound on the default audio
/// output. Dropping the player stops it.
pub struct Player {
    playbin: gst::Element,
    /// The newest sample of the app sink, put there by its streaming
    /// thread.
    newest: Arc<Mutex<Option<gst::Sample>>>,
    looped: bool,
    /// The last frame pulled from the sink.
    frame: Option<Arc<Pixmap>>,
    finished: bool,
}

impl Player {
    /// A paused player at the start of the video, with frames of at most
    /// `max_side` pixels on a side. `silent` sends the sound nowhere, for
    /// tests.
    pub fn new(
        data: &VideoData,
        max_side: u32,
        looped: bool,
        muted: bool,
        silent: bool,
    ) -> Result<Player, VideoError> {
        let (playbin, sink) = playbin(data, max_side)?;
        let newest = Arc::new(Mutex::new(None));
        let keep = |newest: &Arc<Mutex<Option<gst::Sample>>>, sample| {
            if let Ok(mut newest) = newest.lock() {
                *newest = Some(sample);
            }
            Ok(gst::FlowSuccess::Ok)
        };
        let (on_preroll, on_sample) = (newest.clone(), newest.clone());
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| {
                    keep(
                        &on_preroll,
                        sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?,
                    )
                })
                .new_sample(move |sink| {
                    keep(
                        &on_sample,
                        sink.pull_sample().map_err(|_| gst::FlowError::Eos)?,
                    )
                })
                .build(),
        );
        playbin.set_property("mute", muted);
        if silent {
            playbin.set_property("audio-sink", element("fakesink")?);
        }
        playbin
            .set_state(gst::State::Paused)
            .map_err(|_| VideoError::Unreadable)?;
        Ok(Player {
            playbin,
            newest,
            looped,
            frame: None,
            finished: false,
        })
    }

    pub fn play(&mut self) {
        if self.finished {
            self.restart();
        }
        let _ = self.playbin.set_state(gst::State::Playing);
    }

    pub fn pause(&mut self) {
        let _ = self.playbin.set_state(gst::State::Paused);
    }

    pub fn playing(&self) -> bool {
        self.playbin.current_state() == gst::State::Playing && !self.finished
    }

    /// Goes back to the start.
    pub fn restart(&mut self) {
        self.finished = false;
        let _ = self.playbin.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
            gst::ClockTime::ZERO,
        );
    }

    pub fn set_muted(&self, muted: bool) {
        self.playbin.set_property("mute", muted);
    }

    /// Whether the video played to its end and does not loop.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Handles the end of the video and returns the newest frame. Call it
    /// on each animation frame.
    pub fn frame(&mut self) -> Option<Arc<Pixmap>> {
        if let Some(bus) = self.playbin.bus() {
            while let Some(message) =
                bus.pop_filtered(&[gst::MessageType::Eos, gst::MessageType::Error])
            {
                match message.view() {
                    gst::MessageView::Eos(_) if self.looped => self.restart(),
                    _ => self.finished = true,
                }
            }
        }
        let sample = self.newest.lock().ok().and_then(|mut newest| newest.take());
        if let Some(pixels) = sample.as_ref().and_then(sample_pixels) {
            self.frame = Some(Arc::new(pixels));
        }
        self.frame.clone()
    }

    /// Seconds from the start of the video.
    pub fn position(&self) -> f32 {
        self.playbin
            .query_position::<gst::ClockTime>()
            .map_or(0., |position| position.nseconds() as f32 / 1e9)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

/// Videos embedded in the presentation, so it opens anywhere with them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoLibrary {
    pub(crate) videos: BTreeMap<VideoId, VideoData>,
}

impl VideoLibrary {
    pub fn get(&self, id: VideoId) -> Option<&VideoData> {
        self.videos.get(&id)
    }

    pub fn contains(&self, id: VideoId) -> bool {
        self.videos.contains_key(&id)
    }

    /// The embedded videos, by id.
    pub fn iter(&self) -> impl Iterator<Item = (VideoId, &VideoData)> {
        self.videos.iter().map(|(id, data)| (*id, data))
    }

    /// The video holding exactly these bytes, if one does.
    pub fn find(&self, bytes: &[u8]) -> Option<VideoId> {
        self.videos
            .iter()
            .find(|(_, data)| &data.bytes[..] == bytes)
            .map(|(id, _)| *id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Whether the plugins the tests need are installed; prints why not.
    pub fn plugins_or_skip() -> bool {
        let missing = missing_plugins();
        if missing.is_empty() {
            true
        } else {
            eprintln!("missing GStreamer plugins {missing:?}: skipping");
            false
        }
    }

    /// Frames of `width` × `height`: red on the left half and blue on the
    /// right, for `count` frames at 10 frames per second.
    fn frames(width: u32, height: u32) -> impl FnMut(u32) -> Option<Pixmap> {
        move |_| {
            let mut pixmap = Pixmap::new(width, height)?;
            for y in 0..height {
                for x in 0..width {
                    let color = if x < width / 2 {
                        tiny_skia::ColorU8::from_rgba(255, 0, 0, 255)
                    } else {
                        tiny_skia::ColorU8::from_rgba(0, 0, 255, 255)
                    };
                    pixmap.pixels_mut()[(y * width + x) as usize] = color.premultiply();
                }
            }
            Some(pixmap)
        }
    }

    /// An MP4 with H.264 of the [`encode_size`] of `width` × `height`, one
    /// second long.
    pub fn mp4(width: u32, height: u32) -> Arc<[u8]> {
        let (w, h) = encode_size(width, height);
        Arc::from(encode_frames(width, height, 10, 10, &mut frames(w, h)).unwrap())
    }

    /// A WebM with VP8 video and Vorbis audio, from GStreamer test sources.
    fn webm() -> Option<Arc<[u8]>> {
        init().ok()?;
        let output = output_file("webm").ok()?;
        let pipeline = gst::parse::launch(&format!(
            "videotestsrc num-buffers=10 ! video/x-raw,width=64,height=48,framerate=10/1 \
             ! vp8enc ! webmmux name=mux ! filesink location={} \
             audiotestsrc num-buffers=10 ! vorbisenc ! mux.",
            output.display()
        ))
        .ok()?
        .downcast::<gst::Pipeline>()
        .ok()?;
        run_to_end(&pipeline, 0., &|_| {}).ok()?;
        read_output(&output).ok().map(Arc::from)
    }

    #[test]
    fn encodes_reads_and_decodes_the_poster() {
        if !plugins_or_skip() {
            return;
        }
        let data = VideoData::read(mp4(512, 256)).unwrap();
        assert_eq!((data.width, data.height), (512, 256));
        assert!((data.duration - 1.).abs() < 0.2, "{data:?}");
        assert!(!data.has_audio);
        let poster = poster(&data).unwrap();
        assert_eq!((poster.width(), poster.height()), (512, 256));
        let left = poster.pixel(64, 128).unwrap();
        let right = poster.pixel(448, 128).unwrap();
        assert!(left.red() > 200 && left.blue() < 60, "{left:?}");
        assert!(right.blue() > 200 && right.red() < 60, "{right:?}");
        assert!(Arc::ptr_eq(&poster, &cached_poster(&data).unwrap()));
    }

    #[test]
    fn converts_other_formats_and_keeps_embeddable_ones() {
        if !plugins_or_skip() {
            return;
        }
        let Some(webm) = webm() else {
            eprintln!("no VP8 or Vorbis encoder: skipping");
            return;
        };
        assert_eq!(
            VideoData::read(webm.clone()).unwrap_err(),
            VideoError::Unsupported
        );
        let reported = Mutex::new(Vec::new());
        let (data, converted) = prepare(webm, &|done| reported.lock().unwrap().push(done)).unwrap();
        assert!(converted);
        assert_eq!((data.width, data.height), encode_size(64, 48));
        assert!(data.has_audio);
        let mp4 = mp4(16, 16);
        let (kept, converted) = prepare(mp4.clone(), &|_| {}).unwrap();
        assert!(!converted);
        assert!(Arc::ptr_eq(&kept.bytes, &mp4));
    }

    #[test]
    fn encoded_sizes_keep_their_proportions() {
        assert_eq!(encode_size(64, 32), (512, 256));
        assert_eq!(encode_size(1921, 1080), (1922, 1080));
        assert_eq!(encode_size(300, 1000), (300, 1000));
    }

    #[test]
    fn rejects_files_that_are_not_videos() {
        if !plugins_or_skip() {
            return;
        }
        let text: Arc<[u8]> = Arc::from(&b"not a video at all"[..]);
        assert!(prepare(text, &|_| {}).is_err());
        let png = crate::images::tests::png(8, 8);
        assert!(prepare(png, &|_| {}).is_err());
    }

    #[test]
    fn plays_to_the_end_and_loops() {
        if !plugins_or_skip() {
            return;
        }
        let data = VideoData::read(mp4(32, 16)).unwrap();
        let mut once = Player::new(&data, 64, false, true, true).unwrap();
        let start = std::time::Instant::now();
        while once.frame().is_none() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(once.frame().is_some(), "the preroll shows before it plays");
        once.play();
        let start = std::time::Instant::now();
        while !once.finished() && start.elapsed() < Duration::from_secs(5) {
            once.frame();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(once.finished());
        let mut looped = Player::new(&data, 64, true, true, true).unwrap();
        looped.play();
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(1500) {
            looped.frame();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!looped.finished());
        assert!(looped.playing());
    }
}
