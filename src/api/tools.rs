//! The tools of the agent API: one registry for the MCP server and the CLI,
//! and the handlers that run them inside the editor.
//!
//! Handlers work on a [`Host`], so they run the same on the editor and on a
//! bare presentation in tests. Every tool that reads or writes the document
//! returns its `revision`; writes accept a `base_revision` and fail with
//! `stale_revision` when the document changed since.

use std::ops::Range;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::API_VERSION;
use crate::api::ops::{self, Applied, Op, OpError, Options};
use crate::api::protocol::{ApiError, Image, ToolOutput};
use crate::document::{ApplyError, ElementId, Fill, Frame, Presentation, SlideId};
use crate::history::History;
use crate::{fonts, render};

/// Where a tool runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// In an editor instance, reached through its socket.
    Instance,
    /// In the client process: finding and starting instances.
    Local,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments, without the `instance` argument that
    /// every instance tool also takes.
    pub schema: Value,
    pub target: Target,
    pub read_only: bool,
}

impl ToolSpec {
    /// The schema a client advertises: instance tools also take `instance`.
    pub fn input_schema(&self) -> Value {
        let mut schema = self.schema.clone();
        if self.target == Target::Instance {
            schema["properties"]["instance"] = json!({
                "type": "integer",
                "description": "Process id of the Sliderino instance. Needed only when several instances are open; see list_instances."
            });
        }
        schema
    }
}

const OPS_FORMAT: &str = "Each op is an object tagged by \"op\":
- add_slide {slide?: {id?, elements?}, index?}
- remove_slide {id}; move_slide {id, index}
- add_element {slide, parent?, element, index?}. element: {id?, name?, hidden?, locked?, opacity?, frame: {x, y, width, height, rotation}} plus one kind key: text: {content, sizing, style} | group: {children: [element]} | rectangle: {fill?, stroke?, corner_radius?} | ellipse: {fill?, stroke?} | line: {stroke?, start?, end?, from?, to?}
- remove_element {id}; set_frame {id, frame}; set_text_sizing {id, sizing}
- set_text_style {id, patch}; replace_text {id, range: {start, end} (bytes), text}
- set_shape_style {id, patch: {fill?, stroke?, corner_radius?, start?, end?}}: stroke holds only the fields to change, or null to remove the stroke of a rectangle or ellipse
- set_line_points {id, from: {x, y}, to: {x, y}}: moves the ends of a line
- move_element {id, parent?, index?}: into a group of the same slide, or to the slide without parent
- group {id?, children: [id]}; ungroup {id}
- set_layer {id, patch: {name?, hidden?, locked?, opacity?}}; \"name\": null clears the name
- add_font {face} embeds an installed face. add_font {path | data, face?} embeds the faces of a TTF, OTF or TTC file (face picks one); path is read by the client, data is base64. A family name already in use by other fonts becomes \"Name (2)\"; the result lists the embedded faces in fonts. remove_font {face} removes an unused face; remove_font {family} removes all faces of the family and changes its texts to Inter. batch {ops}
- add_image {id?, path | data}: embeds a PNG or JPEG; path is read by the client (relative to its working folder), data is base64. The same bytes are not added twice: the reference names the embedded image. remove_image {id}: removes an image no fill uses.
- add_video {id?, path | data}: embeds a video. An MP4 with H.264 and AAC is kept as it is; other formats are converted to it first, which can take long. remove_video {id}: removes a video no fill uses.
Elements form a tree: a group holds children in paint order (last on top). All frames are in slide units, children too. frame.rotation is the final angle in degrees, clockwise around the frame center, kept in (-180, 180]; x, y, width and height are the frame before it turns. get_elements and find_elements add bounds {x, y, width, height}, the unrotated box around a rotated element. The frame of a group is the box around its children in the axes of the group rotation. set_frame on a group moves it, resizes it by scaling the positions and boxes of its children (not their fonts, stroke widths or corner radii), or turns it and its children to the given angle; a new group has rotation 0, and the rotation of an added group only sets the axes of its box. A locked element, or one inside a locked group, rejects every op but set_layer. Hidden elements are not drawn, exported or reported. opacity (0-1, default 1) applies to the whole element; a group multiplies the opacity of its children. group and ungroup read the document as the earlier ops of the call left it: inside a batch they cannot use elements the same batch creates.
Shapes: a line has a frame of height 0; it goes from the left end to the right end of the frame, turned by the rotation. A frame with a height becomes its horizontal center axis. Give a new line from and to instead of a frame to place it by its ends; get_elements adds points {from, to} for lines. fill: \"none\" | {\"solid\": {color, opacity?}} | {\"linear_gradient\": {angle (degrees clockwise, 0 = left to right, turns with the shape), stops}} | {\"radial_gradient\": {center?: {x, y}, radius?: {x, y} (fractions of the box, default 0.5), stops}} | {\"image\": {id (or \"$name\" of add_image), fit?: cover | contain | stretch, opacity?}} | {\"video\": {id (or \"$name\" of add_video), fit?, opacity?, start?: auto | on_click, loop? (default true), muted?}} | {\"shader\": {source? (GLSL as on Shadertoy: void mainImage(out vec4 fragColor, in vec2 fragCoord) with iResolution, iTime, iTimeDelta, iFrame, iFrameRate, iChannel0, iChannelResolution; default: the Shadertoy default shader), channel0? (image id or \"$name\"), opacity?, start?, loop?, duration? (seconds 1-60, default 10: the loop of the video PPTX gets)}}. In a presentation, start auto plays when the slide shows; on_click fills start one per click, in layer order, before the next slide. Screenshots and PDF show the first video frame and the shader at time 0. get_basic_info lists the images with their upright pixel size, and the videos. stops: 2 to 10 of {position 0-1 in increasing order, color, opacity?}. A new rectangle or ellipse has a light gray fill and no stroke. stroke: {color?, opacity?, width? (default 4), dash?: solid | dashed | dotted}; the stroke is centered on the outline. corner_radius (rectangles) is in slide units. start and end (lines): none | triangle | arrow | diamond | circle, or {kind, size: small | medium | large}.
Ids of new slides and elements are optional; write \"$name\" to name a new id and use \"$name\" in later ops of the same call. Slides are 1600x900 units. Colors are hex strings like \"1A1A1A\". sizing: auto_width | auto_height | fixed. style/patch fields (all optional): font {family, weight 100-900, italic}, size, line_height (\"auto\" or {\"percent\": 120}), letter_spacing (% of size), align (left|center|right|justify), vertical_align (top|middle|bottom), paragraph_spacing, underline, strikethrough, case (original|upper), color. Fonts are embedded automatically from the bundled and system fonts.";

fn empty_schema() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn base_revision() -> Value {
    json!({
        "type": "integer",
        "description": "Fail with stale_revision unless the document is still at this revision."
    })
}

fn slide_arg() -> Value {
    json!({"type": "integer", "description": "Slide id. Default: the slide shown in the editor."})
}

/// Every tool, in the order clients list them.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_instances",
            description: "List the open Sliderino editors: process id, title and API version.",
            schema: empty_schema(),
            target: Target::Local,
            read_only: true,
        },
        ToolSpec {
            name: "open_editor",
            description: "Start a Sliderino editor with a new presentation and wait until it accepts calls. Returns its process id.",
            schema: empty_schema(),
            target: Target::Local,
            read_only: false,
        },
        ToolSpec {
            name: "get_basic_info",
            description: "Overview of the open presentation: revision, slide size, slides with their element counts, embedded fonts and images, the slide shown in the editor and the undo state. Call it first.",
            schema: empty_schema(),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "get_selection",
            description: "What the person sees and edits: the slide shown, the selected elements (a list) and the text selection when a text box is being edited.",
            schema: empty_schema(),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "get_slide",
            description: "Elements of a slide in paint order (last on top), with frame and the content and style of each kind: text, group, rectangle, ellipse or line.",
            schema: json!({
                "type": "object",
                "properties": {"slide": slide_arg()},
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "get_elements",
            description: "Full data of elements by id, with the resolved text layout: line count, content size, overflow past a fixed box and characters the font does not have.",
            schema: json!({
                "type": "object",
                "properties": {"ids": {"type": "array", "items": {"type": "integer"}}},
                "required": ["ids"],
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "find_elements",
            description: "Find text elements whose content contains a string (case-insensitive), on all slides or on one.",
            schema: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"},
                    "slide": {"type": "integer", "description": "Slide id. Default: all slides."}
                },
                "required": ["text"],
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "get_screenshot",
            description: "Render a slide to PNG as it exports, without the editor UI. Use overlay to see text frames, line boxes, baselines and overflow. Videos show their first frame; shaders show their frame at time (default 0).",
            schema: json!({
                "type": "object",
                "properties": {
                    "slide": slide_arg(),
                    "scale": {"type": "number", "description": "Pixels per slide unit. Default 0.5 (800x450)."},
                    "overlay": {"type": "boolean"},
                    "time": {"type": "number", "description": "Seconds after the shaders start. Default 0."},
                    "path": {"type": "string", "description": "Write the PNG to this file instead of returning it."}
                },
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "get_diagnostics",
            description: "Problems, on all slides or on one: text that overflows its fixed box, characters missing from the font (overflow, missing_glyphs), shaders that do not compile (shader_error: {line, message}) and videos or shaders that cannot play on this machine (media_error).",
            schema: json!({
                "type": "object",
                "properties": {"slide": {"type": "integer", "description": "Slide id. Default: all slides."}},
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "render_shader_video",
            description: "Render the shader fill of an element to an MP4 file (H.264), from time 0 to the duration of the fill: the video that PPTX export uses for it. Use it to check the animation of a shader.",
            schema: json!({
                "type": "object",
                "properties": {
                    "element": {"type": "integer", "description": "Id of a rectangle or ellipse with a shader fill."},
                    "path": {"type": "string", "description": "MP4 file to write."},
                    "width": {"type": "integer", "description": "Pixels. Default: the width of the element in slide units, at most 1920."},
                    "height": {"type": "integer", "description": "Pixels. Default: the height of the element in slide units, at most 1920."},
                    "fps": {"type": "integer", "description": "Frames per second, 1 to 60. Default 30."}
                },
                "required": ["element", "path"],
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "list_fonts",
            description: "Font faces embedded in the presentation, and the families available to embed (bundled and installed). Filter families with query.",
            schema: json!({
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Case-insensitive part of a family name."}},
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: true,
        },
        ToolSpec {
            name: "apply_operations",
            description: OPS_FORMAT,
            schema: json!({
                "type": "object",
                "properties": {
                    "ops": {"type": "array", "items": {"type": "object"}, "description": "Operations applied in order, all or none, as one undo step."},
                    "label": {"type": "string", "description": "Name of the undo step shown in the editor's history."},
                    "base_revision": base_revision()
                },
                "required": ["ops"],
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: false,
        },
        ToolSpec {
            name: "undo",
            description: "Undo the latest step of the shared history, whether a person or an agent made it. Pass base_revision to avoid undoing an edit you have not seen.",
            schema: json!({
                "type": "object",
                "properties": {"base_revision": base_revision()},
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: false,
        },
        ToolSpec {
            name: "redo",
            description: "Redo the latest undone step.",
            schema: json!({
                "type": "object",
                "properties": {"base_revision": base_revision()},
                "additionalProperties": false
            }),
            target: Target::Instance,
            read_only: false,
        },
    ]
}

pub fn spec(name: &str) -> Option<ToolSpec> {
    specs().into_iter().find(|spec| spec.name == name)
}

/// The editor state the handlers read, besides the document.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub current_slide: SlideId,
    pub selection: Vec<ElementId>,
    /// Element being edited and its selected byte range.
    pub text_edit: Option<(ElementId, Range<usize>)>,
}

/// Where the handlers run: the editor, or a bare presentation in tests.
pub trait Host {
    fn presentation(&self) -> &Presentation;
    fn history(&self) -> &History;
    fn view(&self) -> ViewState;
    /// Applies the ops all or nothing as one undo step named `label`.
    fn apply_ops(&mut self, label: &str, ops: Vec<Op>) -> Result<Applied, OpError>;
    /// `None` when there is nothing to undo.
    fn undo(&mut self) -> Option<Result<(), ApplyError>>;
    fn redo(&mut self) -> Option<Result<(), ApplyError>>;
}

/// The options agent ops use: automatic fonts, all or nothing.
pub const AGENT_OPTIONS: Options = Options {
    auto_fonts: true,
    all_or_nothing: true,
};

/// Applies agent ops to a presentation and records them as one step; the
/// shared part of [`Host::apply_ops`].
pub fn apply_and_record(
    presentation: &mut Presentation,
    history: &mut History,
    selection: Vec<ElementId>,
    label: &str,
    ops: Vec<Op>,
) -> Result<Applied, OpError> {
    let applied = ops::apply(presentation, ops, AGENT_OPTIONS)?;
    if !applied.steps.is_empty() {
        history.record(label, applied.inverse(), selection.clone(), selection);
    }
    Ok(applied)
}

/// Runs an instance tool.
pub fn handle(host: &mut dyn Host, tool: &str, args: Value) -> Result<ToolOutput, ApiError> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "get_basic_info" => {
            parse::<NoArgs>(args)?;
            Ok(basic_info(host).into())
        }
        "get_selection" => {
            parse::<NoArgs>(args)?;
            Ok(selection(host).into())
        }
        "get_slide" => get_slide(host, parse(args)?),
        "get_elements" => get_elements(host, parse(args)?),
        "find_elements" => find_elements(host, parse(args)?),
        "get_screenshot" => screenshot(host, parse(args)?),
        "get_diagnostics" => diagnostics(host, parse(args)?),
        "render_shader_video" => {
            let (job, value) = shader_video_job(host, args)?;
            shader_video_output(&job, value)
        }
        "list_fonts" => list_fonts(host, parse(args)?),
        "apply_operations" => apply_operations(host, parse(args)?),
        "undo" => step(host, parse(args)?, true),
        "redo" => step(host, parse(args)?, false),
        _ => Err(ApiError::new(
            "unknown_tool",
            format!("no tool {tool:?} in the editor"),
        )),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, ApiError> {
    serde_json::from_value(args).map_err(ApiError::invalid_args)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlideArgs {
    slide: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementsArgs {
    ids: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindArgs {
    text: String,
    slide: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenshotArgs {
    slide: Option<u64>,
    scale: Option<f32>,
    #[serde(default)]
    overlay: bool,
    #[serde(default)]
    time: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShaderVideoArgs {
    element: u64,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FontsArgs {
    query: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyArgs {
    ops: Vec<Op>,
    label: Option<String>,
    base_revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepArgs {
    base_revision: Option<u64>,
}

fn check_revision(host: &dyn Host, base: Option<u64>) -> Result<(), ApiError> {
    let current = host.presentation().revision();
    match base {
        Some(base) if base != current => Err(ApiError::new(
            "stale_revision",
            format!(
                "the document is at revision {current}, not {base}: read it again before changing it"
            ),
        )
        .with_data(json!({"revision": current}))),
        _ => Ok(()),
    }
}

/// The slide the arguments name, or the one the editor shows.
fn slide_id(host: &dyn Host, slide: Option<u64>) -> Result<SlideId, ApiError> {
    let id = slide.map_or(host.view().current_slide, SlideId);
    match host.presentation().slide(id) {
        Some(_) => Ok(id),
        None => Err(ApiError::new("unknown_slide", format!("no slide {}", id.0))),
    }
}

fn basic_info(host: &dyn Host) -> Value {
    let presentation = host.presentation();
    let history = host.history();
    let slides: Vec<Value> = presentation
        .slides
        .iter()
        .map(|slide| json!({"id": slide.id, "elements": slide.walk().len()}))
        .collect();
    let fonts: Vec<_> = presentation.fonts.faces().collect();
    let images: Vec<Value> = presentation
        .images
        .iter()
        .map(|(id, data)| {
            json!({
                "id": id,
                "format": data.format,
                "width": data.width,
                "height": data.height,
                "bytes": data.bytes.len(),
                "used": presentation.image_in_use(id),
            })
        })
        .collect();
    let videos: Vec<Value> = presentation
        .videos
        .iter()
        .map(|(id, data)| {
            json!({
                "id": id,
                "width": data.width,
                "height": data.height,
                "duration": data.duration,
                "audio": data.has_audio,
                "bytes": data.bytes.len(),
                "used": presentation.video_in_use(id),
            })
        })
        .collect();
    json!({
        "api_version": API_VERSION,
        "revision": presentation.revision(),
        "slide_size": {"width": presentation.size.width, "height": presentation.size.height},
        "slides": slides,
        "fonts": fonts,
        "images": images,
        "videos": videos,
        "current_slide": host.view().current_slide,
        "undo": history.done().last(),
        "redo": history.undone().next(),
    })
}

fn selection(host: &dyn Host) -> Value {
    let view = host.view();
    let text_edit = view.text_edit.map(
        |(id, range)| json!({"id": id, "selection": {"start": range.start, "end": range.end}}),
    );
    json!({
        "revision": host.presentation().revision(),
        "current_slide": view.current_slide,
        "selection": view.selection,
        "text_edit": text_edit,
    })
}

fn get_slide(host: &dyn Host, args: SlideArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    let id = slide_id(host, args.slide)?;
    let slide = presentation.slide(id).expect("checked slide");
    Ok(json!({
        "revision": presentation.revision(),
        "slide": id,
        "index": presentation.index_of(id),
        "elements": slide.elements,
    })
    .into())
}

fn get_elements(host: &dyn Host, args: ElementsArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    let mut elements = Vec::with_capacity(args.ids.len());
    for id in args.ids.into_iter().map(ElementId) {
        let Some(element) = presentation.element(id) else {
            return Err(ApiError::new(
                "unknown_element",
                format!("no element {}", id.0),
            ));
        };
        let mut value = serde_json::to_value(element).expect("elements serialize");
        value["slide"] = json!(presentation.locate(id).map(|location| location.slide));
        if element.frame.rotation != 0. {
            value["bounds"] = bounds_json(&element.frame);
        }
        if element.is_line() {
            let (from, to) = element.frame.line_ends();
            value["points"] = json!({
                "from": {"x": from.0, "y": from.1},
                "to": {"x": to.0, "y": to.1},
            });
        }
        if let Some(layout) = presentation.text_layout(id) {
            value["layout"] = json!({
                "lines": layout.lines.len(),
                "content_width": layout.content_width,
                "content_height": layout.content_height,
                "overflow": layout.overflow(),
                "missing_glyphs": layout.missing_glyphs,
            });
        }
        elements.push(value);
    }
    Ok(json!({"revision": presentation.revision(), "elements": elements}).into())
}

/// The unrotated box around a rotated frame, for agents that check overlap
/// or the slide edges.
fn bounds_json(frame: &Frame) -> Value {
    let bounds = frame.bounds();
    json!({"x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height})
}

fn find_elements(host: &dyn Host, args: FindArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    if let Some(slide) = args.slide {
        slide_id(host, Some(slide))?;
    }
    let needle = args.text.to_lowercase();
    let found: Vec<Value> = presentation
        .slides
        .iter()
        .filter(|slide| args.slide.is_none_or(|id| slide.id.0 == id))
        .flat_map(|slide| {
            slide
                .walk()
                .into_iter()
                .map(move |node| (slide.id, node.element))
        })
        .filter_map(|(slide, element)| {
            let text = element.as_text()?;
            text.content.to_lowercase().contains(&needle).then(|| {
                let mut value = json!({
                    "id": element.id,
                    "slide": slide,
                    "content": text.content,
                    "frame": element.frame,
                });
                if element.frame.rotation != 0. {
                    value["bounds"] = bounds_json(&element.frame);
                }
                value
            })
        })
        .collect();
    Ok(json!({"revision": presentation.revision(), "elements": found}).into())
}

fn screenshot(host: &dyn Host, args: ScreenshotArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    let id = slide_id(host, args.slide)?;
    let scale = args.scale.unwrap_or(0.5);
    if !args.time.is_finite() || args.time < 0. {
        return Err(ApiError::invalid_args("time must be 0 or more seconds"));
    }
    let pixmap = render::render_slide_at(presentation, id, scale, args.overlay, args.time)
        .map_err(|error| ApiError::new("render_failed", error.to_string()))?;
    let png = pixmap
        .encode_png()
        .map_err(|error| ApiError::new("render_failed", error.to_string()))?;
    Ok(ToolOutput {
        value: json!({
            "revision": presentation.revision(),
            "slide": id,
            "width": pixmap.width(),
            "height": pixmap.height(),
        }),
        image: Some(Image {
            mime: "image/png".into(),
            data: base64::engine::general_purpose::STANDARD.encode(png),
        }),
    })
}

fn diagnostics(host: &dyn Host, args: SlideArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    if let Some(slide) = args.slide {
        slide_id(host, Some(slide))?;
    }
    let problems: Vec<Value> = presentation
        .slides
        .iter()
        .filter(|slide| args.slide.is_none_or(|id| slide.id.0 == id))
        .flat_map(|slide| {
            render::report(presentation, slide.id)
                .into_iter()
                .map(move |report| (slide.id, report))
        })
        .filter(|(_, report)| report.overflow > 0. || report.missing_glyphs > 0)
        .map(|(slide, report)| {
            json!({
                "slide": slide,
                "element": report.id,
                "frame": report.frame,
                "lines": report.lines,
                "overflow": report.overflow,
                "missing_glyphs": report.missing_glyphs,
            })
        })
        .chain(media_problems(presentation, args.slide))
        .collect();
    Ok(json!({"revision": presentation.revision(), "problems": problems}).into())
}

/// Shaders that do not compile, and videos and shaders that cannot play
/// on this machine, on all slides or on one.
fn media_problems(presentation: &Presentation, slide: Option<u64>) -> Vec<Value> {
    let mut problems = Vec::new();
    let mut missing_plugins = None;
    let mut gpu = None;
    for slide in presentation
        .slides
        .iter()
        .filter(|found| slide.is_none_or(|id| found.id.0 == id))
    {
        for node in slide.walk() {
            let element = node.element;
            let problem = match element.kind.fill() {
                Some(Fill::Shader(shader)) => match crate::shaders::check(&shader.source) {
                    Err(error) => Some(json!({
                        "shader_error": {"line": error.line, "message": error.message}
                    })),
                    Ok(()) if !*gpu.get_or_insert_with(crate::shaders::available) => {
                        Some(json!({"media_error": "no GPU is available to render shaders"}))
                    }
                    Ok(()) => None,
                },
                Some(Fill::Video(_)) => {
                    let missing =
                        missing_plugins.get_or_insert_with(crate::videos::missing_plugins);
                    (!missing.is_empty()).then(|| {
                        json!({"media_error": format!(
                            "missing GStreamer plugins: {}", missing.join(", ")
                        )})
                    })
                }
                _ => None,
            };
            if let Some(mut problem) = problem {
                problem["slide"] = json!(slide.id);
                problem["element"] = json!(element.id);
                problems.push(problem);
            }
        }
    }
    problems
}

/// The first half of `render_shader_video`, which reads the document: what
/// to render. The editor runs it on its UI thread, then
/// [`shader_video_output`] off it.
pub fn shader_video_job(
    host: &dyn Host,
    args: Value,
) -> Result<(crate::shaders::VideoJob, Value), ApiError> {
    let args: ShaderVideoArgs = parse(args)?;
    let presentation = host.presentation();
    let id = ElementId(args.element);
    let element = presentation
        .element(id)
        .ok_or_else(|| ApiError::new("unknown_element", format!("no element {}", id.0)))?;
    let Some(Fill::Shader(shader)) = element.kind.fill() else {
        return Err(ApiError::invalid_args(format!(
            "element {} has no shader fill",
            id.0
        )));
    };
    let size = |value: Option<u32>, frame: f32| value.unwrap_or(frame.round().max(1.) as u32);
    let (width, height) = crate::shaders::clamp_size(
        size(args.width, element.frame.width),
        size(args.height, element.frame.height),
    );
    let channel0 = match shader.channel0 {
        Some(image) => Some(
            presentation
                .images
                .get(image)
                .and_then(crate::images::pixels)
                .ok_or_else(|| {
                    ApiError::new("render_failed", format!("image {} cannot be read", image.0))
                })?,
        ),
        None => None,
    };
    let job = crate::shaders::VideoJob {
        source: shader.source.clone(),
        channel0,
        width,
        height,
        fps: args.fps.unwrap_or(30).clamp(1, 60),
        duration: shader.duration,
    };
    let value = json!({"revision": presentation.revision(), "element": id});
    Ok((job, value))
}

/// The second half of `render_shader_video`: renders and encodes the video.
/// Slow.
pub fn shader_video_output(
    job: &crate::shaders::VideoJob,
    mut value: Value,
) -> Result<ToolOutput, ApiError> {
    let bytes = job
        .encode()
        .map_err(|error| ApiError::new("render_failed", error.to_string()))?;
    let (width, height) = crate::videos::encode_size(job.width, job.height);
    value["width"] = json!(width);
    value["height"] = json!(height);
    value["fps"] = json!(job.fps);
    value["duration"] = json!(job.duration);
    value["bytes"] = json!(bytes.len());
    Ok(ToolOutput {
        value,
        image: Some(Image {
            mime: "video/mp4".into(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        }),
    })
}

fn list_fonts(host: &dyn Host, args: FontsArgs) -> Result<ToolOutput, ApiError> {
    let presentation = host.presentation();
    let embedded: Vec<_> = presentation.fonts.faces().collect();
    let mut value = json!({"revision": presentation.revision(), "embedded": embedded});
    // The system scan blocks; the editor starts it in the background.
    match fonts::catalog_ready() {
        None => value["loading"] = json!(true),
        Some(catalog) => {
            let query = args.query.map(|query| query.to_lowercase());
            let families: Vec<Value> = catalog
                .families()
                .iter()
                .filter(|family| {
                    query
                        .as_ref()
                        .is_none_or(|query| family.name.to_lowercase().contains(query))
                })
                .map(|family| {
                    let faces: Vec<Value> = family
                        .faces
                        .iter()
                        .map(|face| {
                            let mut value = json!({"weight": face.weight, "italic": face.italic});
                            // Reading the license loads the face: only for
                            // a filtered list.
                            if query.is_some() && catalog.is_restricted(face) {
                                value["restricted"] = json!(true);
                            }
                            value
                        })
                        .collect();
                    json!({"family": family.name, "faces": faces})
                })
                .collect();
            value["families"] = json!(families);
        }
    }
    Ok(value.into())
}

fn apply_operations(host: &mut dyn Host, args: ApplyArgs) -> Result<ToolOutput, ApiError> {
    check_revision(host, args.base_revision)?;
    let label = match (&args.label, args.ops.as_slice()) {
        (Some(label), _) => label.clone(),
        (None, [op]) => op.label().to_string(),
        (None, _) => "Agent edit".to_string(),
    };
    let applied = host.apply_ops(&label, args.ops).map_err(|error| {
        ApiError::new("operation_failed", error.to_string()).with_data(json!({"op": error.index}))
    })?;
    Ok(json!({
        "revision": host.presentation().revision(),
        "refs": applied.refs,
        "warnings": applied.warnings,
        "fonts": applied.fonts,
    })
    .into())
}

fn step(host: &mut dyn Host, args: StepArgs, undo: bool) -> Result<ToolOutput, ApiError> {
    check_revision(host, args.base_revision)?;
    let label = if undo {
        host.history().done().last().map(str::to_string)
    } else {
        host.history().undone().next().map(str::to_string)
    };
    let result = if undo { host.undo() } else { host.redo() };
    match result {
        Some(Err(error)) => Err(ApiError::new("operation_failed", error.to_string())),
        Some(Ok(())) | None => {
            let key = if undo { "undone" } else { "redone" };
            let mut value = json!({"revision": host.presentation().revision()});
            value[key] = json!(label.filter(|_| result.is_some()));
            Ok(value.into())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;

    /// A presentation and its history, without an editor.
    #[derive(Default)]
    pub struct TestHost {
        pub presentation: Presentation,
        pub history: History,
    }

    impl Host for TestHost {
        fn presentation(&self) -> &Presentation {
            &self.presentation
        }

        fn history(&self) -> &History {
            &self.history
        }

        fn view(&self) -> ViewState {
            ViewState {
                current_slide: self.presentation.slides[0].id,
                selection: Vec::new(),
                text_edit: None,
            }
        }

        fn apply_ops(&mut self, label: &str, ops: Vec<Op>) -> Result<Applied, OpError> {
            apply_and_record(
                &mut self.presentation,
                &mut self.history,
                vec![],
                label,
                ops,
            )
        }

        fn undo(&mut self) -> Option<Result<(), ApplyError>> {
            self.history
                .undo(&mut self.presentation)
                .map(|result| result.map(|_| ()))
        }

        fn redo(&mut self) -> Option<Result<(), ApplyError>> {
            self.history
                .redo(&mut self.presentation)
                .map(|result| result.map(|_| ()))
        }
    }

    fn call(host: &mut TestHost, tool: &str, args: Value) -> Result<Value, ApiError> {
        handle(host, tool, args).map(|output| output.value)
    }

    fn add_title(host: &mut TestHost) -> Value {
        call(
            host,
            "apply_operations",
            json!({"ops": [
                {"op": "add_element", "slide": 1, "element": {
                    "id": "$title", "frame": {"x": 100, "y": 100, "width": 300, "height": 40},
                    "text": {"content": "A title far too long for its box", "sizing": "fixed"}}},
                {"op": "set_text_style", "id": "$title", "patch": {"size": 48}}
            ], "label": "Add title"}),
        )
        .unwrap()
    }

    #[test]
    fn a_call_is_one_undo_step() {
        let mut host = TestHost::default();
        let result = add_title(&mut host);
        assert_eq!(result["refs"]["$title"], 1);
        assert_eq!(host.history.done().collect::<Vec<_>>(), ["Add title"]);
        let undone = call(&mut host, "undo", json!({})).unwrap();
        assert_eq!(undone["undone"], "Add title");
        assert!(host.presentation.slides[0].elements.is_empty());
    }

    #[test]
    fn a_stale_revision_changes_nothing() {
        let mut host = TestHost::default();
        let revision = add_title(&mut host)["revision"].as_u64().unwrap();
        call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "remove_element", "id": 1}], "base_revision": revision - 1}),
        )
        .map(|_| ())
        .map_err(|error| {
            assert_eq!(error.code, "stale_revision");
            assert_eq!(error.data["revision"], revision);
        })
        .unwrap_err();
        assert!(host.presentation.element(ElementId(1)).is_some());
        let undo = call(&mut host, "undo", json!({"base_revision": revision - 1})).unwrap_err();
        assert_eq!(undo.code, "stale_revision");
        call(&mut host, "undo", json!({"base_revision": revision})).unwrap();
    }

    #[test]
    fn a_failed_call_names_the_op_and_changes_nothing() {
        let mut host = TestHost::default();
        let error = call(
            &mut host,
            "apply_operations",
            json!({"ops": [
                {"op": "add_element", "slide": 1, "element": {"text": {"content": "a"}}},
                {"op": "remove_slide", "id": 1}
            ]}),
        )
        .unwrap_err();
        assert_eq!(error.code, "operation_failed");
        assert_eq!(error.data["op"], 1);
        assert!(host.presentation.slides[0].elements.is_empty());
        assert_eq!(host.history.done().count(), 0);
    }

    #[test]
    fn diagnostics_report_overflow() {
        let mut host = TestHost::default();
        add_title(&mut host);
        let problems = call(&mut host, "get_diagnostics", json!({})).unwrap()["problems"].clone();
        assert_eq!(problems[0]["element"], 1);
        assert!(problems[0]["overflow"].as_f64().unwrap() > 0.);
    }

    #[test]
    fn reads_describe_the_document() {
        let mut host = TestHost::default();
        add_title(&mut host);
        let info = call(&mut host, "get_basic_info", Value::Null).unwrap();
        assert_eq!(info["api_version"], API_VERSION);
        assert_eq!(info["slides"][0]["elements"], 1);
        assert_eq!(info["undo"], "Add title");
        assert_eq!(info["fonts"][0]["family"], "Inter");

        let slide = call(&mut host, "get_slide", json!({})).unwrap();
        assert_eq!(slide["elements"][0]["text"]["style"]["size"], 48.0);

        let found = call(&mut host, "find_elements", json!({"text": "TITLE"})).unwrap();
        assert_eq!(found["elements"][0]["id"], 1);

        let elements = call(&mut host, "get_elements", json!({"ids": [1]})).unwrap();
        assert!(elements["elements"][0]["layout"]["lines"].as_u64().unwrap() > 1);

        let error = call(&mut host, "get_elements", json!({"ids": [9]})).unwrap_err();
        assert_eq!(error.code, "unknown_element");
        let error = call(&mut host, "get_slide", json!({"slide": 1, "extra": 1})).unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
    }

    #[test]
    fn screenshots_are_png() {
        let mut host = TestHost::default();
        let output = handle(&mut host, "get_screenshot", json!({})).unwrap();
        assert_eq!(output.value["width"], 800);
        let image = output.image.unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image.data)
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
    }

    /// A shader that paints red while time is under 1 second, then blue.
    const TIMED: &str = "void mainImage(out vec4 c, in vec2 p) { c = iTime < 1.0 ? vec4(1, 0, 0, 1) : vec4(0, 0, 1, 1); }";

    fn add_shader(host: &mut TestHost, source: &str) {
        call(
            host,
            "apply_operations",
            json!({"ops": [{"op": "add_element", "slide": 1, "element": {
                "id": "$s", "frame": {"x": 0, "y": 0, "width": 400, "height": 200},
                "rectangle": {"fill": {"shader": {"source": source, "duration": 2}}}}}]}),
        )
        .unwrap();
    }

    fn pixel(output: &ToolOutput, x: u32, y: u32) -> (u8, u8, u8) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&output.image.as_ref().unwrap().data)
            .unwrap();
        let pixmap = render::Pixmap::decode_png(&bytes).unwrap();
        let pixel = pixmap.pixel(x, y).unwrap();
        (pixel.red(), pixel.green(), pixel.blue())
    }

    #[test]
    fn screenshots_show_shaders_at_a_time() {
        if !crate::shaders::available() {
            eprintln!("no GPU adapter: skipping");
            return;
        }
        let mut host = TestHost::default();
        add_shader(&mut host, TIMED);
        let start = handle(&mut host, "get_screenshot", json!({})).unwrap();
        assert_eq!(pixel(&start, 100, 50), (255, 0, 0));
        let later = handle(&mut host, "get_screenshot", json!({"time": 1.5})).unwrap();
        assert_eq!(pixel(&later, 100, 50), (0, 0, 255));
        let error = call(&mut host, "get_screenshot", json!({"time": -1})).unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
    }

    #[test]
    fn diagnostics_report_shader_errors_with_their_line() {
        let mut host = TestHost::default();
        add_shader(
            &mut host,
            "void mainImage(out vec4 c, in vec2 p)\n{\n    c = vec4(nope);\n}\n",
        );
        let problems = call(&mut host, "get_diagnostics", json!({})).unwrap()["problems"].clone();
        assert_eq!(problems[0]["element"], 1);
        assert_eq!(problems[0]["shader_error"]["line"], 3);
    }

    #[test]
    fn shader_videos_last_the_duration_of_the_fill() {
        if !crate::shaders::available() || !crate::videos::tests::plugins_or_skip() {
            return;
        }
        let mut host = TestHost::default();
        add_shader(&mut host, TIMED);
        let output = handle(
            &mut host,
            "render_shader_video",
            json!({"element": 1, "fps": 10}),
        )
        .unwrap();
        assert_eq!(output.value["duration"], 2.0);
        let video = output.image.unwrap();
        assert_eq!(video.mime, "video/mp4");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(video.data)
            .unwrap();
        let data = crate::videos::VideoData::read(Arc::from(bytes)).unwrap();
        assert!((data.duration - 2.).abs() < 0.2, "{data:?}");
        assert_eq!((data.width, data.height), (512, 256));
        let text = add_title(&mut host)["refs"]["$title"].clone();
        let error = call(&mut host, "render_shader_video", json!({"element": text})).unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
    }

    #[test]
    fn every_instance_tool_has_a_handler() {
        let mut host = TestHost::default();
        for spec in specs() {
            let result = handle(&mut host, spec.name, json!({"no_such_argument": 1}));
            let code = result.err().map(|error| error.code);
            match spec.target {
                Target::Instance => assert_eq!(code.as_deref(), Some("invalid_arguments")),
                Target::Local => assert_eq!(code.as_deref(), Some("unknown_tool")),
            }
        }
    }

    #[test]
    fn agents_group_move_lock_and_hide_elements() {
        let mut host = TestHost::default();
        let text = |id: &str, x: i32| {
            json!({"op": "add_element", "slide": 1, "element": {
                "id": id, "frame": {"x": x, "y": 100},
                "text": {"content": id, "sizing": "auto_width"}}})
        };
        let result = call(
            &mut host,
            "apply_operations",
            json!({"ops": [
                text("$a", 100),
                text("$b", 400),
                {"op": "group", "id": "$g", "children": ["$a", "$b"]},
                {"op": "set_layer", "id": "$g", "patch": {"name": "Header"}}
            ]}),
        )
        .unwrap();
        let group = result["refs"]["$g"].as_u64().unwrap();
        let slide = call(&mut host, "get_slide", json!({})).unwrap();
        let element = &slide["elements"][0];
        assert_eq!(element["id"], group);
        assert_eq!(element["name"], "Header");
        assert_eq!(element["group"]["children"].as_array().unwrap().len(), 2);
        let info = call(&mut host, "get_basic_info", json!({})).unwrap();
        assert_eq!(info["slides"][0]["elements"], 3);

        let found = call(&mut host, "find_elements", json!({"text": "$b"})).unwrap();
        assert_eq!(found["elements"].as_array().unwrap().len(), 1);

        call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "set_layer", "id": group, "patch": {"locked": true}}]}),
        )
        .unwrap();
        let a = result["refs"]["$a"].as_u64().unwrap();
        let error = call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "move_element", "id": a}]}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("locked"), "{error}");
        call(&mut host, "undo", json!({})).unwrap();

        call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "ungroup", "id": group}]}),
        )
        .unwrap();
        assert_eq!(host.presentation.slides[0].elements.len(), 2);
    }

    #[test]
    fn agents_create_groups_with_children_in_one_op() {
        let mut host = TestHost::default();
        let result = call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "add_element", "slide": 1, "element": {
            "id": "$g", "hidden": true, "group": {"children": [
                {"id": "$t", "frame": {"x": 10, "y": 20},
                 "text": {"content": "Hi", "sizing": "auto_width"}}
            ]}}}]}),
        )
        .unwrap();
        let group = result["refs"]["$g"].as_u64().unwrap();
        let text = result["refs"]["$t"].as_u64().unwrap();
        let element = host.presentation.element(ElementId(group)).unwrap();
        assert!(element.hidden);
        assert_eq!(element.frame.x, 10.);
        assert_eq!(element.children()[0].id, ElementId(text));
        assert_eq!(host.history.done().last(), Some("Create group"));
    }

    #[test]
    fn hidden_texts_are_not_reported() {
        let mut host = TestHost::default();
        add_title(&mut host);
        call(
            &mut host,
            "apply_operations",
            json!({"ops": [{"op": "set_layer", "id": 1, "patch": {"hidden": true}}]}),
        )
        .unwrap();
        let problems = call(&mut host, "get_diagnostics", json!({})).unwrap()["problems"].clone();
        assert_eq!(problems, json!([]));
    }
}
