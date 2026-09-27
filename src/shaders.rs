//! Shader fills: fragment shaders written as on Shadertoy, rendered off
//! screen on the GPU and read back as pixels for the renderers.
//!
//! The author writes `void mainImage(out vec4 fragColor, in vec2 fragCoord)`
//! in GLSL. A prelude declares the Shadertoy inputs (`iResolution`, `iTime`,
//! `iTimeDelta`, `iFrame`, `iFrameRate`, `iMouse`, `iDate`,
//! `iChannelResolution` and `iChannel0`) and a `main` calls `mainImage` with
//! the origin at the bottom left, as Shadertoy does. The output is opaque:
//! Shadertoy ignores the alpha of `fragColor`, and so does Sliderino.
//!
//! The GPU starts the first time a shader is compiled, not when the editor
//! starts. Without an adapter, shaders do not render and the fills show a
//! placeholder.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::Pixmap;

/// The shader a new shader fill starts with: the default of Shadertoy.
pub const DEFAULT_SOURCE: &str = "\
void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    // Normalized pixel coordinates (from 0 to 1)
    vec2 uv = fragCoord / iResolution.xy;

    // Time varying pixel color
    vec3 col = 0.5 + 0.5 * cos(iTime + uv.xyx + vec3(0, 2, 4));

    // Output to screen
    fragColor = vec4(col, 1.0);
}
";

/// Most pixels on a side of a rendered frame; larger boxes are scaled up
/// from a smaller frame.
pub const MAX_SIDE: u32 = 1920;

/// Frames per second of a playing shader.
pub const FRAME_RATE: f32 = 30.;

const PRELUDE: &str = "\
#version 450
layout(set = 0, binding = 0, std140) uniform SliderinoInputs {
    vec3 iResolution;
    float iTime;
    float iTimeDelta;
    int iFrame;
    float iFrameRate;
    float iSampleRate;
    vec4 iMouse;
    vec4 iDate;
    vec3 iChannelResolution[4];
};
layout(set = 0, binding = 1) uniform texture2D sliderino_channel0;
layout(set = 0, binding = 2) uniform sampler sliderino_sampler;
#define iChannel0 sampler2D(sliderino_channel0, sliderino_sampler)
layout(location = 0) out vec4 sliderino_color;
";

const EPILOGUE: &str = "
void main() {
    vec4 color = vec4(0.0);
    mainImage(color, vec2(gl_FragCoord.x, iResolution.y - gl_FragCoord.y));
    sliderino_color = vec4(clamp(color.rgb, 0.0, 1.0), 1.0);
}
";

/// A triangle that covers the target.
const VERTEX: &str = "
@vertex
fn main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
";

/// Bytes of the uniform block of the prelude, with the std140 layout.
const INPUTS_BYTES: usize = 128;

/// A problem in the source of a shader, at a line of the author's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderError {
    /// 1-based; 0 when the problem has no line.
    pub line: u32,
    pub message: String,
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for ShaderError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// No GPU adapter is available.
    NoGpu,
    Shader(ShaderError),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::NoGpu => write!(f, "no GPU is available to render shaders"),
            RenderError::Shader(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RenderError {}

fn prelude_lines() -> u32 {
    PRELUDE.lines().count() as u32
}

/// The full GLSL of a fragment shader: prelude, the author's source and the
/// entry point.
fn full_source(source: &str) -> String {
    let mut full = String::with_capacity(PRELUDE.len() + source.len() + EPILOGUE.len() + 1);
    full.push_str(PRELUDE);
    full.push_str(source);
    full.push('\n');
    full.push_str(EPILOGUE);
    full
}

/// The line of the author's source for a span of the full source.
fn author_line(span: naga::Span, full: &str) -> u32 {
    if span == naga::Span::default() {
        return 0;
    }
    let line = span.location(full).line_number;
    line.saturating_sub(prelude_lines())
}

/// Parses and validates a shader on the CPU. The GPU is not needed. The
/// results of the last sources are kept, so that the editor can check the
/// shaders it shows on each render.
pub fn check(source: &str) -> Result<(), ShaderError> {
    static CHECKED: Mutex<Vec<(u64, Result<(), ShaderError>)>> = Mutex::new(Vec::new());
    const KEPT: usize = 64;
    let key = hash(source);
    if let Ok(checked) = CHECKED.lock()
        && let Some((_, result)) = checked.iter().find(|(found, _)| *found == key)
    {
        return result.clone();
    }
    let result = parse(source).map(|_| ());
    if let Ok(mut checked) = CHECKED.lock() {
        checked.push((key, result.clone()));
        if checked.len() > KEPT {
            drop(checked.remove(0));
        }
    }
    result
}

fn parse(source: &str) -> Result<(naga::Module, naga::valid::ModuleInfo), ShaderError> {
    let full = full_source(source);
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let module = frontend.parse(&options, &full).map_err(|errors| {
        let first = errors.errors.first();
        ShaderError {
            line: first.map_or(0, |error| author_line(error.meta, &full)),
            message: first.map_or_else(
                || "the shader cannot be read".into(),
                |error| error.kind.to_string(),
            ),
        }
    })?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| {
        let line = error
            .spans()
            .next()
            .map_or(0, |(span, _)| author_line(*span, &full));
        ShaderError {
            line,
            message: error.as_inner().to_string(),
        }
    })?;
    Ok((module, info))
}

/// The inputs of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inputs {
    /// Seconds since the shader started.
    pub time: f32,
    /// Seconds since the previous frame.
    pub time_delta: f32,
    /// Frames since the shader started.
    pub frame: i32,
}

impl Inputs {
    /// The inputs of the frame at `time`, at [`FRAME_RATE`].
    pub fn at(time: f32) -> Self {
        let time = time.max(0.);
        Inputs {
            time,
            time_delta: 1. / FRAME_RATE,
            frame: (time * FRAME_RATE) as i32,
        }
    }
}

fn inputs_bytes(
    width: u32,
    height: u32,
    inputs: Inputs,
    channel: (u32, u32),
) -> [u8; INPUTS_BYTES] {
    let mut bytes = [0u8; INPUTS_BYTES];
    let mut put = |offset: usize, value: [u8; 4]| bytes[offset..offset + 4].copy_from_slice(&value);
    put(0, (width as f32).to_ne_bytes());
    put(4, (height as f32).to_ne_bytes());
    put(8, 1f32.to_ne_bytes());
    put(12, inputs.time.to_ne_bytes());
    put(16, inputs.time_delta.to_ne_bytes());
    put(20, inputs.frame.to_ne_bytes());
    put(24, FRAME_RATE.to_ne_bytes());
    put(28, 44100f32.to_ne_bytes());
    // iMouse and iDate stay 0: a slide has no mouse, and an export must not
    // change with the day it is made.
    put(64, (channel.0 as f32).to_ne_bytes());
    put(68, (channel.1 as f32).to_ne_bytes());
    put(72, 1f32.to_ne_bytes());
    bytes
}

/// The hash of a shader's source and the format of the target it compiled
/// for: a live [`ShaderPlayer`] can target a different format than
/// [`FORMAT`], so the same source can have a pipeline per format.
type ProgramKey = (u64, wgpu::TextureFormat);
type Programs = HashMap<ProgramKey, Result<Arc<wgpu::RenderPipeline>, ShaderError>>;

/// What to render and where, for [`Gpu::render_to_buffer`].
struct RenderRequest<'a> {
    source: &'a str,
    channel0: Option<&'a Arc<Pixmap>>,
    target: &'a wgpu::Texture,
    uniforms: &'a wgpu::Buffer,
    width: u32,
    height: u32,
    inputs: Inputs,
    format: wgpu::TextureFormat,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    vertex: wgpu::ShaderModule,
    sampler: wgpu::Sampler,
    /// Compiled shaders by [`ProgramKey`].
    programs: Mutex<Programs>,
    /// Uploaded channel images, by the address of their pixels; each entry
    /// keeps the pixels alive so the address stays a valid key.
    channels: Mutex<Vec<(Arc<Pixmap>, Arc<wgpu::Texture>)>>,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Most channel textures kept on the GPU.
const CHANNELS_KEPT: usize = 8;

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(|| pollster::block_on(Gpu::new())).as_ref()
}

impl Gpu {
    async fn new() -> Option<Gpu> {
        let _span = crate::perf::span("shader_gpu");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sliderino-shaders"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await
            .ok()?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader-inputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader-vertex"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX)),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shader-channel"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        Some(Gpu {
            device,
            queue,
            layout,
            pipeline_layout,
            vertex,
            sampler,
            programs: Mutex::new(HashMap::new()),
            channels: Mutex::new(Vec::new()),
        })
    }

    /// The compiled pipeline of `source`, rendering to `format`. Compiled
    /// once per source and format, then kept.
    fn program(
        &self,
        source: &str,
        format: wgpu::TextureFormat,
    ) -> Result<Arc<wgpu::RenderPipeline>, ShaderError> {
        let key = (hash(source), format);
        if let Some(program) = self.programs.lock().ok().and_then(|p| p.get(&key).cloned()) {
            return program;
        }
        let program = parse(source).map(|(module, _)| {
            let _span = crate::perf::span("shader_compile");
            let fragment = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("shader-fragment"),
                    source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
                });
            Arc::new(
                self.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("shader"),
                        layout: Some(&self.pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: &self.vertex,
                            entry_point: Some("main"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &fragment,
                            entry_point: Some("main"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    }),
            )
        });
        if let Ok(mut programs) = self.programs.lock() {
            programs.insert(key, program.clone());
        }
        program
    }

    /// The texture of a channel image, uploaded once. A 1×1 black texture
    /// stands for no image.
    fn channel(&self, pixels: Option<&Arc<Pixmap>>) -> Arc<wgpu::Texture> {
        static BLACK: OnceLock<Arc<Pixmap>> = OnceLock::new();
        let pixels = pixels.unwrap_or_else(|| {
            BLACK.get_or_init(|| {
                let mut black = Pixmap::new(1, 1).expect("1×1 pixmap");
                black.fill(tiny_skia::Color::BLACK);
                Arc::new(black)
            })
        });
        let mut channels = self.channels.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = channels
            .iter()
            .position(|(key, _)| Arc::ptr_eq(key, pixels))
        {
            let entry = channels.remove(index);
            let texture = entry.1.clone();
            channels.push(entry);
            return texture;
        }
        let size = wgpu::Extent3d {
            width: pixels.width(),
            height: pixels.height(),
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader-channel"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Shadertoy samples straight colors, not premultiplied ones, and
        // flips images: texture coordinate y = 0 is the bottom row, so that
        // texture(iChannel0, fragCoord / iResolution.xy) shows the image
        // upright.
        let width = pixels.width() as usize;
        let straight: Vec<u8> = pixels
            .pixels()
            .chunks(width)
            .rev()
            .flatten()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &straight,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * pixels.width()),
                rows_per_image: None,
            },
            size,
        );
        let texture = Arc::new(texture);
        channels.push((pixels.clone(), texture.clone()));
        if channels.len() > CHANNELS_KEPT {
            channels.remove(0);
        }
        texture
    }

    /// A render target of `width` × `height`, for repeated renders at that
    /// size.
    fn make_target(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// A uniform buffer of the shader inputs, for repeated renders.
    fn make_uniforms(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader-inputs"),
            size: INPUTS_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Renders one frame of `source` into `target` (`width` × `height`, the
    /// size `target` was made at) and queues a copy of it to a new readback
    /// buffer. Submitted to the queue but not waited on: the caller either
    /// blocks on it ([`Gpu::render`]) or polls it ([`ShaderPlayer`]).
    /// Returns the readback buffer and the byte stride of its rows, padded
    /// to `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`.
    fn render_to_buffer(
        &self,
        request: RenderRequest<'_>,
    ) -> Result<(wgpu::Buffer, u32), RenderError> {
        let RenderRequest {
            source,
            channel0,
            target,
            uniforms,
            width,
            height,
            inputs,
            format,
        } = request;
        let pipeline = self.program(source, format).map_err(RenderError::Shader)?;
        let channel = self.channel(channel0);
        let channel_size = (channel.width(), channel.height());
        let channel_size = if channel0.is_some() {
            channel_size
        } else {
            (0, 0)
        };
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let inputs = inputs_bytes(width, height, inputs, channel_size);
        self.queue.write_buffer(uniforms, 0, &inputs);
        let channel_view = channel.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader-inputs"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&channel_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        // Rows of a copy to a buffer are aligned to 256 bytes.
        let row = (4 * width).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader-readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        Ok((readback, row))
    }

    /// Renders one frame, blocking until it is done, and returns it as a
    /// premultiplied [`Pixmap`] (the output is opaque, so straight and
    /// premultiplied are the same). Used for stills, thumbnails, export and
    /// [`VideoJob`]; a live shader uses [`ShaderPlayer`] instead, which does
    /// not block the calling thread.
    fn render(
        &self,
        source: &str,
        channel0: Option<&Arc<Pixmap>>,
        width: u32,
        height: u32,
        inputs: Inputs,
    ) -> Result<Pixmap, RenderError> {
        let _span = crate::perf::span("shader_render");
        let target = self.make_target(width, height, FORMAT);
        let uniforms = self.make_uniforms();
        let (readback, row) = self.render_to_buffer(RenderRequest {
            source,
            channel0,
            target: &target,
            uniforms: &uniforms,
            width,
            height,
            inputs,
            format: FORMAT,
        })?;
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|_| RenderError::NoGpu)?;
        let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::NoGpu)?;
        {
            let mapped = slice.get_mapped_range();
            let data = pixmap.data_mut();
            let line = 4 * width as usize;
            for y in 0..height as usize {
                let from = y * row as usize;
                data[y * line..(y + 1) * line].copy_from_slice(&mapped[from..from + line]);
            }
        }
        readback.unmap();
        Ok(pixmap)
    }
}

fn hash(source: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

/// Whether a GPU can render shaders. Starts the GPU on the first call.
pub fn available() -> bool {
    gpu().is_some()
}

/// Renders one frame of a shader at `width` × `height` pixels. The sizes are
/// clamped to 1..=[`MAX_SIDE`], keeping their proportions.
pub fn render(
    source: &str,
    channel0: Option<&Arc<Pixmap>>,
    width: u32,
    height: u32,
    inputs: Inputs,
) -> Result<Pixmap, RenderError> {
    let (width, height) = clamp_size(width, height);
    gpu()
        .ok_or(RenderError::NoGpu)?
        .render(source, channel0, width, height, inputs)
}

/// A size no larger than [`MAX_SIDE`] on a side, with the same
/// proportions, and at least 1 pixel.
pub fn clamp_size(width: u32, height: u32) -> (u32, u32) {
    let largest = width.max(height).max(1);
    if largest <= MAX_SIDE {
        return (width.max(1), height.max(1));
    }
    let scale = MAX_SIDE as f32 / largest as f32;
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

/// The pixel layout a [`ShaderPlayer`] renders and reads back: BGRA for the
/// presenter's direct path, which hands the bytes straight to GPUI without a
/// conversion; RGBA for the CPU renderer, which wraps them in a [`Pixmap`].
/// The shader output is opaque, so straight and premultiplied alpha are the
/// same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra,
    Rgba,
}

impl PixelFormat {
    fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            PixelFormat::Bgra => wgpu::TextureFormat::Bgra8Unorm,
            PixelFormat::Rgba => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

/// One frame a [`ShaderPlayer`] read back from the GPU, shaped like
/// [`crate::videos::VideoFrame`]: packed rows with no row padding, in the
/// [`PixelFormat`] the player was made with.
#[derive(Clone)]
pub struct ShaderFrame {
    pub width: u32,
    pub height: u32,
    pub bytes: Arc<[u8]>,
    /// Counts the frames a player delivered, so that a view knows a new one.
    pub serial: u64,
    /// The shader time (seconds) this frame shows.
    pub time: f32,
}

impl ShaderFrame {
    /// The frame as a premultiplied [`Pixmap`], for a player made with
    /// [`PixelFormat::Rgba`].
    pub fn pixmap(&self) -> Option<Pixmap> {
        Pixmap::from_vec(
            self.bytes.to_vec(),
            tiny_skia::IntSize::from_wh(self.width, self.height)?,
        )
    }
}

/// Whether a readback slot of a [`ShaderPlayer`] is free to submit into, has
/// a render in flight, or holds a mapped frame waiting to be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Free,
    Pending,
    Ready,
}

/// What the `map_async` callback of a [`Slot`] found: still waiting,
/// mapped and ready to read, or the mapping failed (for example, the
/// device was lost). Kept in an atomic so the callback, which can run on
/// another thread, can set it.
const MAP_PENDING: u8 = 0;
const MAP_READY: u8 = 1;
const MAP_FAILED: u8 = 2;

/// One readback buffer of the ring a [`ShaderPlayer`] submits into.
struct Slot {
    buffer: wgpu::Buffer,
    state: SlotState,
    /// One of [`MAP_PENDING`], [`MAP_READY`] or [`MAP_FAILED`]: a failed
    /// mapping means `buffer` was never actually mapped, so it must be
    /// freed without a call to `get_mapped_range` or `unmap`, both of which
    /// would panic on it.
    mapped: Arc<AtomicU8>,
    serial: u64,
    time: f32,
    width: u32,
    height: u32,
    /// Byte stride of a row in `buffer`, padded to
    /// `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`.
    row: u32,
}

/// Readback buffers kept per live shader: enough that a submit rarely finds
/// every slot busy, without holding much GPU memory.
const RING: usize = 3;

/// Plays a shader live: renders ahead into a small ring of readback
/// buffers and polls them, so the calling thread never waits for the GPU.
/// Used by the presenter and by the editor's Preview; see
/// [`crate::ui::playback::Playback`].
pub struct ShaderPlayer {
    format: PixelFormat,
    size: (u32, u32),
    target: Option<wgpu::Texture>,
    uniforms: Option<wgpu::Buffer>,
    ring: [Option<Slot>; RING],
    serial: u64,
    skipped: u64,
    /// The last frame delivered, returned again while none is newer.
    last: Option<ShaderFrame>,
}

impl ShaderPlayer {
    pub fn new(format: PixelFormat) -> Self {
        ShaderPlayer {
            format,
            size: (0, 0),
            target: None,
            uniforms: None,
            ring: [const { None }; RING],
            serial: 0,
            skipped: 0,
            last: None,
        }
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Frames rendered since the player started, whether they were taken by
    /// [`ShaderPlayer::latest`] or not.
    pub fn delivered(&self) -> u64 {
        self.serial
    }

    /// Submits a render that skipped because every slot of the ring was
    /// busy.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Whether a render is in flight: the view should ask for another frame
    /// soon.
    pub fn pending(&self) -> bool {
        self.ring
            .iter()
            .flatten()
            .any(|slot| slot.state == SlotState::Pending)
    }

    /// Renders one frame at `size` (clamped to [`MAX_SIDE`], keeping its
    /// proportions) into a free slot of the ring; does nothing and counts a
    /// skip when every slot is busy. Returns whether it rendered.
    pub fn submit(
        &mut self,
        source: &str,
        channel0: Option<&Arc<Pixmap>>,
        size: (u32, u32),
        inputs: Inputs,
    ) -> bool {
        let Some(gpu) = gpu() else { return false };
        let (width, height) = clamp_size(size.0, size.1);
        if (width, height) != self.size {
            self.size = (width, height);
            self.target = None;
            self.uniforms = None;
            self.ring = [const { None }; RING];
            // Otherwise `latest` would keep handing out the old size until
            // a frame of the new one actually maps.
            self.last = None;
        }
        let Some(index) = self.ring.iter().position(|slot| {
            slot.as_ref()
                .is_none_or(|slot| slot.state == SlotState::Free)
        }) else {
            self.skipped += 1;
            return false;
        };
        let format = self.format.wgpu();
        let target = self
            .target
            .get_or_insert_with(|| gpu.make_target(width, height, format));
        let uniforms = self.uniforms.get_or_insert_with(|| gpu.make_uniforms());
        let Ok((buffer, row)) = gpu.render_to_buffer(RenderRequest {
            source,
            channel0,
            target,
            uniforms,
            width,
            height,
            inputs,
            format,
        }) else {
            return false;
        };
        self.serial += 1;
        let mapped = Arc::new(AtomicU8::new(MAP_PENDING));
        let callback_mapped = mapped.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let state = if result.is_ok() {
                    MAP_READY
                } else {
                    MAP_FAILED
                };
                callback_mapped.store(state, Ordering::Release);
            });
        self.ring[index] = Some(Slot {
            buffer,
            state: SlotState::Pending,
            mapped,
            serial: self.serial,
            time: inputs.time,
            width,
            height,
            row,
        });
        true
    }

    /// The newest frame that finished mapping since the last call, or the
    /// last one delivered when none is newer. Frees every slot that was
    /// ready: the older ones are dropped unread.
    pub fn latest(&mut self) -> Option<ShaderFrame> {
        let gpu = gpu()?;
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        for slot in self.ring.iter_mut().flatten() {
            if slot.state != SlotState::Pending {
                continue;
            }
            match slot.mapped.load(Ordering::Acquire) {
                MAP_READY => slot.state = SlotState::Ready,
                // The mapping failed: `buffer` was never mapped, so it is
                // freed here directly, without the read and the `unmap` a
                // ready slot gets below.
                MAP_FAILED => slot.state = SlotState::Free,
                _ => {}
            }
        }
        let newest = self
            .ring
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| match slot {
                Some(slot) if slot.state == SlotState::Ready => Some((index, slot.serial)),
                _ => None,
            })
            .max_by_key(|(_, serial)| *serial)
            .map(|(index, _)| index);
        let Some(newest) = newest else {
            return self.last.clone();
        };
        let mut frame = None;
        for (index, slot) in self.ring.iter_mut().enumerate() {
            let Some(slot) = slot else { continue };
            if slot.state != SlotState::Ready {
                continue;
            }
            if index == newest {
                let line = 4 * slot.width as usize;
                let mut bytes = vec![0u8; line * slot.height as usize];
                {
                    let mapped = slot.buffer.slice(..).get_mapped_range();
                    for y in 0..slot.height as usize {
                        let from = y * slot.row as usize;
                        bytes[y * line..(y + 1) * line].copy_from_slice(&mapped[from..from + line]);
                    }
                }
                slot.buffer.unmap();
                frame = Some(ShaderFrame {
                    width: slot.width,
                    height: slot.height,
                    bytes: Arc::from(bytes),
                    serial: slot.serial,
                    time: slot.time,
                });
            } else {
                slot.buffer.unmap();
            }
            slot.state = SlotState::Free;
        }
        self.last = frame.clone();
        frame
    }
}

/// What a shader video is made of.
#[derive(Clone, Debug)]
pub struct VideoJob {
    pub source: Arc<str>,
    pub channel0: Option<Arc<Pixmap>>,
    /// Size of the picture; the video has the [`crate::videos::encode_size`]
    /// of it.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Seconds.
    pub duration: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoJobError {
    Render(RenderError),
    Video(crate::videos::VideoError),
}

impl std::fmt::Display for VideoJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoJobError::Render(error) => error.fmt(f),
            VideoJobError::Video(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for VideoJobError {}

/// Encoded shader videos by the hash of their job, most recently used last.
static VIDEOS: Mutex<Vec<(u64, Arc<[u8]>)>> = Mutex::new(Vec::new());

/// Most shader videos kept encoded.
const VIDEOS_KEPT: usize = 4;

impl VideoJob {
    fn key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.source.hash(&mut hasher);
        if let Some(channel) = &self.channel0 {
            channel.data().hash(&mut hasher);
        }
        (self.width, self.height, self.fps, self.duration.to_bits()).hash(&mut hasher);
        hasher.finish()
    }

    /// Renders the shader from time 0 to `duration` and encodes the frames
    /// to an MP4 with H.264: the video that PPTX plays for a shader fill.
    /// The last job results are kept, so an export does not render the same
    /// shader twice. Slow: call it off the UI thread.
    pub fn encode(&self) -> Result<Arc<[u8]>, VideoJobError> {
        let key = self.key();
        if let Ok(mut videos) = VIDEOS.lock()
            && let Some(index) = videos.iter().position(|(found, _)| *found == key)
        {
            let entry = videos.remove(index);
            let bytes = entry.1.clone();
            videos.push(entry);
            return Ok(bytes);
        }
        // Checks the shader and the GPU before GStreamer starts.
        let gpu = gpu().ok_or(VideoJobError::Render(RenderError::NoGpu))?;
        gpu.program(&self.source, FORMAT)
            .map_err(|error| VideoJobError::Render(RenderError::Shader(error)))?;
        let fps = self.fps.clamp(1, 60);
        let (width, height) = crate::videos::encode_size(self.width, self.height);
        let count = ((self.duration * fps as f32).round() as u32).max(1);
        let mut failed = None;
        let bytes = crate::videos::encode_frames(width, height, fps, count, &mut |n| {
            let time = n as f32 / fps as f32;
            let inputs = Inputs {
                time,
                time_delta: 1. / fps as f32,
                frame: n as i32,
            };
            gpu.render(&self.source, self.channel0.as_ref(), width, height, inputs)
                .map_err(|error| failed = Some(error))
                .ok()
        });
        if let Some(error) = failed {
            return Err(VideoJobError::Render(error));
        }
        let bytes: Arc<[u8]> = Arc::from(bytes.map_err(VideoJobError::Video)?);
        if let Ok(mut videos) = VIDEOS.lock() {
            videos.push((key, bytes.clone()));
            if videos.len() > VIDEOS_KEPT {
                videos.remove(0);
            }
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skips a test when the machine has no GPU adapter.
    fn gpu_or_skip() -> bool {
        if available() {
            true
        } else {
            eprintln!("no GPU adapter: skipping");
            false
        }
    }

    #[test]
    fn the_default_shader_compiles() {
        assert_eq!(check(DEFAULT_SOURCE), Ok(()));
    }

    #[test]
    fn errors_give_the_line_of_the_author() {
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord)\n{\n    fragColor = vec4(nope, 1.0);\n}\n";
        let error = check(source).unwrap_err();
        assert_eq!(error.line, 3, "{error}");
        let missing = check("void other() {}\n").unwrap_err();
        assert!(missing.message.contains("mainImage"), "{missing}");
    }

    #[test]
    fn renders_with_the_origin_at_the_bottom_left() {
        if !gpu_or_skip() {
            return;
        }
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord)\n{\n    fragColor = fragCoord.y < iResolution.y * 0.5 ? vec4(1, 0, 0, 1) : vec4(0, 0, 1, 1);\n}\n";
        let pixmap = render(source, None, 8, 8, Inputs::at(0.)).unwrap();
        let top = pixmap.pixel(4, 1).unwrap();
        let bottom = pixmap.pixel(4, 6).unwrap();
        assert_eq!((top.red(), top.blue(), top.alpha()), (0, 255, 255));
        assert_eq!((bottom.red(), bottom.blue()), (255, 0));
    }

    #[test]
    fn time_and_channel_reach_the_shader() {
        if !gpu_or_skip() {
            return;
        }
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord)\n{\n    fragColor = vec4(iTime / 10.0, texture(iChannel0, vec2(0.25, 0.5)).b, iChannelResolution[0].x / 100.0, 1.0);\n}\n";
        let mut channel = Pixmap::new(40, 20).unwrap();
        channel.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        let pixmap = render(source, Some(&Arc::new(channel)), 4, 4, Inputs::at(5.)).unwrap();
        let pixel = pixmap.pixel(1, 1).unwrap();
        assert!((126..=129).contains(&pixel.red()), "{pixel:?}");
        assert_eq!(pixel.green(), 255);
        assert!((100..=103).contains(&pixel.blue()), "{pixel:?}");
    }

    #[test]
    fn encodes_a_video_of_the_duration_once() {
        if !gpu_or_skip() || !crate::videos::tests::plugins_or_skip() {
            return;
        }
        let job = VideoJob {
            source: Arc::from(DEFAULT_SOURCE),
            channel0: None,
            width: 320,
            height: 180,
            fps: 10,
            duration: 1.,
        };
        let bytes = job.encode().unwrap();
        assert!(
            Arc::ptr_eq(&bytes, &job.encode().unwrap()),
            "the second is cached"
        );
        let data = crate::videos::VideoData::read(bytes).unwrap();
        assert_eq!(
            (data.width, data.height),
            crate::videos::encode_size(320, 180)
        );
        assert!((data.duration - 1.).abs() < 0.2, "{data:?}");
        let broken = VideoJob {
            source: Arc::from("void mainImage(out vec4 c, in vec2 p) { nope; }\n"),
            ..job
        };
        assert!(matches!(
            broken.encode(),
            Err(VideoJobError::Render(RenderError::Shader(_)))
        ));
    }

    #[test]
    fn channels_show_upright_as_on_shadertoy() {
        if !gpu_or_skip() {
            return;
        }
        // Red on top, blue at the bottom.
        let mut channel = Pixmap::new(4, 4).unwrap();
        for (index, pixel) in channel.pixels_mut().iter_mut().enumerate() {
            let color = if index < 8 { (255, 0, 0) } else { (0, 0, 255) };
            *pixel = tiny_skia::ColorU8::from_rgba(color.0, color.1, color.2, 255).premultiply();
        }
        let source = "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p / iResolution.xy); }\n";
        let pixmap = render(source, Some(&Arc::new(channel)), 4, 4, Inputs::at(0.)).unwrap();
        assert_eq!(pixmap.pixel(1, 0).unwrap().red(), 255, "red on top");
        assert_eq!(
            pixmap.pixel(1, 3).unwrap().blue(),
            255,
            "blue at the bottom"
        );
    }

    #[test]
    fn large_sizes_are_clamped_with_their_proportions() {
        assert_eq!(clamp_size(3840, 1920), (1920, 960));
        assert_eq!(clamp_size(0, 10), (1, 10));
    }

    /// Polls a player until it delivers a frame, or panics after 5 seconds.
    fn wait_for_frame(player: &mut ShaderPlayer) -> ShaderFrame {
        let start = std::time::Instant::now();
        loop {
            if let Some(frame) = player.latest() {
                return frame;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "no frame in time"
            );
        }
    }

    #[test]
    fn a_player_delivers_a_frame_after_submit() {
        if !gpu_or_skip() {
            return;
        }
        let mut player = ShaderPlayer::new(PixelFormat::Rgba);
        assert!(player.latest().is_none());
        assert!(player.submit(DEFAULT_SOURCE, None, (8, 8), Inputs::at(0.)));
        let frame = wait_for_frame(&mut player);
        assert_eq!((frame.width, frame.height), (8, 8));
    }

    #[test]
    fn a_player_renders_red_in_its_format() {
        if !gpu_or_skip() {
            return;
        }
        let source = "void mainImage(out vec4 fragColor, in vec2 fragCoord)\n{\n    fragColor = vec4(1.0, 0.0, 0.0, 1.0);\n}\n";
        let mut bgra = ShaderPlayer::new(PixelFormat::Bgra);
        assert!(bgra.submit(source, None, (2, 2), Inputs::at(0.)));
        let frame = wait_for_frame(&mut bgra);
        assert_eq!(&frame.bytes[0..4], &[0, 0, 255, 255]);

        let mut rgba = ShaderPlayer::new(PixelFormat::Rgba);
        assert!(rgba.submit(source, None, (2, 2), Inputs::at(0.)));
        let frame = wait_for_frame(&mut rgba);
        assert_eq!(&frame.bytes[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn a_full_ring_skips_the_submit() {
        if !gpu_or_skip() {
            return;
        }
        let mut player = ShaderPlayer::new(PixelFormat::Rgba);
        for _ in 0..RING {
            assert!(player.submit(DEFAULT_SOURCE, None, (8, 8), Inputs::at(0.)));
        }
        assert_eq!(player.skipped(), 0);
        assert!(!player.submit(DEFAULT_SOURCE, None, (8, 8), Inputs::at(0.)));
        assert_eq!(player.skipped(), 1);
    }

    #[test]
    fn a_failed_map_frees_its_slot_without_reading_it() {
        // A real device loss is not something this test can force. Instead
        // it plants a slot whose `mapped` atomic already reads
        // `MAP_FAILED`, without ever calling `map_async` on its buffer: had
        // it, the real callback could fire on `latest`'s poll and overwrite
        // the forced value. `latest` must free this slot without calling
        // `get_mapped_range` or `unmap` on it: either would panic, since
        // its buffer was never actually mapped.
        let Some(gpu) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let mut player = ShaderPlayer::new(PixelFormat::Rgba);
        player.ring[0] = Some(Slot {
            buffer: gpu.make_uniforms(),
            state: SlotState::Pending,
            mapped: Arc::new(AtomicU8::new(MAP_FAILED)),
            serial: 1,
            time: 0.,
            width: 8,
            height: 8,
            row: 32,
        });
        assert!(player.latest().is_none());
        assert!(!player.pending());
        // The slot is free again: a submit finds room for it.
        assert!(player.submit(DEFAULT_SOURCE, None, (8, 8), Inputs::at(0.)));
    }

    #[test]
    fn a_size_change_gives_frames_of_the_new_size() {
        if !gpu_or_skip() {
            return;
        }
        let mut player = ShaderPlayer::new(PixelFormat::Rgba);
        assert!(player.submit(DEFAULT_SOURCE, None, (8, 8), Inputs::at(0.)));
        let first = wait_for_frame(&mut player);
        assert_eq!((first.width, first.height), (8, 8));
        assert!(player.submit(DEFAULT_SOURCE, None, (16, 12), Inputs::at(0.)));
        let second = wait_for_frame(&mut player);
        assert_eq!((second.width, second.height), (16, 12));
    }
}
