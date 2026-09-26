//! Renders a slide to an image on the CPU, without GPUI: the lines and glyph
//! positions come from `text_layout`, the same as in the editor, and each
//! glyph is filled from its outline in the embedded font. Shapes take their
//! outlines, heads and gradients from `shape`.
//!
//! Content past the slide edge is cut off, as in an export. The debug overlay
//! draws text frames, line boxes, baselines and overflow.

use std::sync::Arc;

use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Path, PathBuilder, Rect, SpreadMode, Stroke,
    StrokeDash, Transform,
};
pub use tiny_skia::{Paint, Pixmap};

use crate::document::{
    Element, ElementId, ElementKind, Fill, FontData, Frame, GradientStop, ImageId, Presentation,
    Rgb, SlideId,
};
use crate::shape::{self, Head, Seg};
use crate::text_layout::TextLayout;

/// Gives the decoded pixels of an embedded image, upright, or `None` while
/// they are not available.
pub type Images<'a> = &'a dyn Fn(ImageId) -> Option<Arc<Pixmap>>;

/// For renders that draw no image: image fills show as a placeholder.
pub fn no_images(_: ImageId) -> Option<Arc<Pixmap>> {
    None
}

/// Colors of the overlay, from the editor palette (`theme.rs`).
const ACCENT: Rgb = Rgb(0x1F4BFF);
const WARN: Rgb = Rgb(0xB26A00);
const GUIDE: Rgb = Rgb(0xF2376B);

#[derive(Debug, PartialEq)]
pub enum RenderError {
    UnknownSlide(SlideId),
    /// The requested image is empty or too large.
    InvalidSize,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::UnknownSlide(id) => write!(f, "no slide {}", id.0),
            RenderError::InvalidSize => write!(f, "invalid image size"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Renders the slide at `scale` (1.0 is one pixel per slide unit).
pub fn render_slide(
    presentation: &Presentation,
    slide: SlideId,
    scale: f32,
    overlay: bool,
) -> Result<Pixmap, RenderError> {
    let slide = presentation
        .slide(slide)
        .ok_or(RenderError::UnknownSlide(slide))?;
    let width = (presentation.size.width as f32 * scale).round() as u32;
    let height = (presentation.size.height as f32 * scale).round() as u32;
    if !scale.is_finite() || scale <= 0. {
        return Err(RenderError::InvalidSize);
    }
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::InvalidSize)?;
    pixmap.fill(Color::WHITE);
    let transform = Transform::from_scale(scale, scale);

    let leaves: Vec<(&Element, f32, Option<TextLayout>)> = slide
        .visible_leaves()
        .map(|node| {
            let layout = presentation.text_layout(node.element.id);
            (node.element, node.opacity, layout)
        })
        .collect();
    let texts = || {
        leaves
            .iter()
            .filter_map(|(element, _, layout)| Some((*element, layout.as_ref()?)))
    };

    if overlay {
        for (element, layout) in texts() {
            let transform = turned(transform, &element.frame);
            paint_overlay_under(&mut pixmap, &element.frame, layout, transform);
        }
    }
    let images = presentation_images(presentation);
    for (element, opacity, layout) in &leaves {
        let transform = turned(transform, &element.frame);
        match layout {
            Some(layout) => paint_text(
                &mut pixmap,
                presentation,
                element,
                *opacity,
                layout,
                transform,
            ),
            None => paint_shape(&mut pixmap, element, *opacity, transform, &images),
        }
    }
    if overlay {
        for (element, layout) in texts() {
            let transform = turned(transform, &element.frame);
            paint_overlay_over(&mut pixmap, &element.frame, layout, scale, transform);
        }
    }
    Ok(pixmap)
}

/// `transform` with the rotation of `frame` around its center applied first.
fn turned(transform: Transform, frame: &Frame) -> Transform {
    if frame.rotation == 0. {
        return transform;
    }
    let (cx, cy) = frame.center();
    transform.pre_concat(Transform::from_rotate_at(frame.rotation, cx, cy))
}

/// The images of the presentation, decoded when first drawn.
fn presentation_images(presentation: &Presentation) -> impl Fn(ImageId) -> Option<Arc<Pixmap>> {
    |id| crate::images::pixels(presentation.images.get(id)?)
}

/// Paints a rectangle, an ellipse or a line; other kinds are left alone.
/// `transform` maps slide units to pixels with the rotation of the frame
/// applied; `opacity` multiplies the opacities of the fill and the stroke.
pub fn paint_shape(
    pixmap: &mut Pixmap,
    element: &Element,
    opacity: f32,
    transform: Transform,
    images: Images,
) {
    let frame = &element.frame;
    if let ElementKind::Line(line) = &element.kind {
        // The line runs along the x axis of its local units.
        let local = transform.pre_translate(frame.x, frame.y + frame.height / 2.);
        let geometry = shape::line_geometry(frame.width, line);
        let stroke = &line.stroke;
        let color = paint(stroke.color, stroke.opacity * opacity);
        if let Some((from, to)) = geometry.segment {
            let mut builder = PathBuilder::new();
            builder.move_to(from.0, from.1);
            builder.line_to(to.0, to.1);
            if let Some(path) = builder.finish() {
                let style =
                    stroke_style(stroke.width, shape::dash_array(stroke.dash, stroke.width));
                pixmap.stroke_path(&path, &color, &style, local, None);
            }
        }
        for head in &geometry.heads {
            match head {
                Head::Filled(segs) => {
                    if let Some(path) = build_path(segs) {
                        pixmap.fill_path(&path, &color, FillRule::Winding, local, None);
                    }
                }
                Head::Open(points) => {
                    let mut builder = PathBuilder::new();
                    builder.move_to(points[0].0, points[0].1);
                    builder.line_to(points[1].0, points[1].1);
                    builder.line_to(points[2].0, points[2].1);
                    if let Some(path) = builder.finish() {
                        let style = stroke_style(stroke.width, None);
                        pixmap.stroke_path(&path, &color, &style, local, None);
                    }
                }
            }
        }
        return;
    }
    let local = transform.pre_translate(frame.x, frame.y);
    let Some(segs) = shape::outline(&element.kind, frame.width, frame.height) else {
        return;
    };
    let Some(path) = build_path(&segs) else {
        return;
    };
    if let Some(fill) = element.kind.fill() {
        fill_shape(pixmap, &path, fill, frame, opacity, local, images);
    }
    if let Some(stroke) = element.kind.stroke() {
        let color = paint(stroke.color, stroke.opacity * opacity);
        let style = stroke_style(stroke.width, shape::dash_array(stroke.dash, stroke.width));
        pixmap.stroke_path(&path, &color, &style, local, None);
    }
}

fn build_path(segs: &[Seg]) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for seg in segs {
        match *seg {
            Seg::Move((x, y)) => builder.move_to(x, y),
            Seg::Line((x, y)) => builder.line_to(x, y),
            Seg::Cubic((x1, y1), (x2, y2), (x, y)) => builder.cubic_to(x1, y1, x2, y2, x, y),
            Seg::Close => builder.close(),
        }
    }
    builder.finish()
}

fn stroke_style(width: f32, dash: Option<[f32; 2]>) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Butt,
        line_join: LineJoin::Miter,
        dash: dash.and_then(|dash| StrokeDash::new(dash.to_vec(), 0.)),
        ..Stroke::default()
    }
}

fn color(color: Rgb, alpha: f32) -> Color {
    let [_, r, g, b] = color.0.to_be_bytes();
    Color::from_rgba8(r, g, b, (alpha.clamp(0., 1.) * 255.).round() as u8)
}

fn gradient_stops(stops: &[GradientStop], opacity: f32) -> Vec<tiny_skia::GradientStop> {
    stops
        .iter()
        .map(|stop| {
            tiny_skia::GradientStop::new(stop.position, color(stop.color, stop.opacity * opacity))
        })
        .collect()
}

/// A light checker for an image fill whose pixels are not available.
const PLACEHOLDER: Rgb = Rgb(0xE4E4E4);

fn fill_shape(
    pixmap: &mut Pixmap,
    path: &Path,
    fill: &Fill,
    frame: &Frame,
    opacity: f32,
    local: Transform,
    images: Images,
) {
    let (width, height) = (frame.width, frame.height);
    let shader = match fill {
        Fill::None => return,
        Fill::Solid(solid) => {
            let color = paint(solid.color, solid.opacity * opacity);
            pixmap.fill_path(path, &color, FillRule::Winding, local, None);
            return;
        }
        Fill::LinearGradient(gradient) => {
            let (start, end) = shape::linear_gradient_line(gradient.angle, width, height);
            tiny_skia::LinearGradient::new(
                start.into(),
                end.into(),
                gradient_stops(&gradient.stops, opacity),
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
        Fill::RadialGradient(gradient) => {
            let (center, radius) =
                shape::radial_gradient_ellipse(gradient.center, gradient.radius, width, height);
            // A unit circle at the origin, stretched to the ellipse.
            tiny_skia::RadialGradient::new(
                (0., 0.).into(),
                (0., 0.).into(),
                1.,
                gradient_stops(&gradient.stops, opacity),
                SpreadMode::Pad,
                Transform::from_row(radius.0, 0., 0., radius.1, center.0, center.1),
            )
        }
        Fill::Image(image) => {
            let Some(pixels) = images(image.id) else {
                let color = paint(PLACEHOLDER, image.opacity * opacity);
                pixmap.fill_path(path, &color, FillRule::Winding, local, None);
                return;
            };
            let size = (pixels.width(), pixels.height());
            let (x, y, w, h) = shape::fit_rect(image.fit, width, height, size);
            let placed = Transform::from_row(w / size.0 as f32, 0., 0., h / size.1 as f32, x, y);
            let shader = tiny_skia::Pattern::new(
                pixels.as_ref().as_ref(),
                SpreadMode::Pad,
                FilterQuality::Bicubic,
                image.opacity * opacity,
                placed,
            );
            // With contain, the pattern stops at the image edges.
            let clip = Rect::from_xywh(x.max(0.), y.max(0.), w.min(width), h.min(height));
            let paint = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            match clip.and_then(|clip| {
                let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height())?;
                mask.fill_path(
                    &PathBuilder::from_rect(clip),
                    FillRule::Winding,
                    true,
                    local,
                );
                Some(mask)
            }) {
                Some(mask) => pixmap.fill_path(path, &paint, FillRule::Winding, local, Some(&mask)),
                None => pixmap.fill_path(path, &paint, FillRule::Winding, local, None),
            }
            return;
        }
    };
    let Some(shader) = shader else {
        return;
    };
    let paint = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    pixmap.fill_path(path, &paint, FillRule::Winding, local, None);
}

/// Renders one shape alone, turned by its rotation, at `scale` pixels per
/// slide unit. Returns the image and the area of the slide it covers: the
/// bounds of the frame grown by the stroke and the heads, and a pixel of
/// margin. The pixels are premultiplied.
pub fn render_shape_box(
    element: &Element,
    opacity: f32,
    scale: f32,
    images: Images,
) -> Option<(Pixmap, Frame)> {
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    let bounds = shape::visual_frame(element).bounds();
    let margin = 1. / scale;
    let area = Frame {
        x: bounds.x - margin,
        y: bounds.y - margin,
        width: bounds.width + 2. * margin,
        height: bounds.height + 2. * margin,
        rotation: 0.,
    };
    let width = (area.width * scale).ceil() as u32;
    let height = (area.height * scale).ceil() as u32;
    let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
    let transform = Transform::from_scale(scale, scale).pre_translate(-area.x, -area.y);
    paint_shape(
        &mut pixmap,
        element,
        opacity,
        turned(transform, &element.frame),
        images,
    );
    let area = Frame {
        width: width as f32 / scale,
        height: height as f32 / scale,
        ..area
    };
    Some((pixmap, area))
}

/// A solid paint of `color` at `alpha`, anti-aliased.
pub fn paint(color: Rgb, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    let [_, r, g, b] = color.0.to_be_bytes();
    paint.set_color_rgba8(r, g, b, (alpha.clamp(0., 1.) * 255.).round() as u8);
    paint.anti_alias = true;
    paint
}

fn fill_rect(pixmap: &mut Pixmap, rect: (f32, f32, f32, f32), paint: &Paint, transform: Transform) {
    if let Some(rect) = Rect::from_xywh(rect.0, rect.1, rect.2, rect.3) {
        pixmap.fill_rect(rect, paint, transform, None);
    }
}

/// Builds glyph outlines into one path, placed in slide units.
struct GlyphPath {
    builder: PathBuilder,
    scale: f32,
    x: f32,
    y: f32,
}

impl ttf_parser::OutlineBuilder for GlyphPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder
            .move_to(self.x + x * self.scale, self.y - y * self.scale);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder
            .line_to(self.x + x * self.scale, self.y - y * self.scale);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x2 * self.scale,
            self.y - y2 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

fn paint_text(
    pixmap: &mut Pixmap,
    presentation: &Presentation,
    element: &Element,
    opacity: f32,
    layout: &TextLayout,
    transform: Transform,
) {
    let Some(text) = element.as_text() else {
        return;
    };
    let style = &text.style;
    let Some(data) = presentation.fonts.get(&style.font) else {
        return;
    };
    let ink = TextInk {
        font: data,
        layout,
        color: paint(style.color, opacity),
        underline: style.underline,
        strikethrough: style.strikethrough,
    };
    draw_text(pixmap, &ink, &element.frame, transform);
}

/// What fills the text of one box.
pub struct TextInk<'a> {
    pub font: &'a FontData,
    pub layout: &'a TextLayout,
    pub color: Paint<'static>,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Fills the glyphs and decorations of a text box. `transform` maps slide
/// units to pixels; the rotation of the frame is not applied here.
fn draw_text(pixmap: &mut Pixmap, ink: &TextInk, frame: &Frame, transform: Transform) {
    let Ok(face) = ttf_parser::Face::parse(&ink.font.bytes, ink.font.index) else {
        return;
    };
    let layout = ink.layout;
    let color = &ink.color;
    let mut glyphs = GlyphPath {
        builder: PathBuilder::new(),
        scale: layout.font_size / face.units_per_em() as f32,
        x: 0.,
        y: 0.,
    };
    for line in &layout.lines {
        let baseline = frame.y + line.baseline;
        for glyph in &line.glyphs {
            glyphs.x = frame.x + glyph.x;
            glyphs.y = baseline + glyph.y;
            face.outline_glyph(ttf_parser::GlyphId(glyph.id), &mut glyphs);
        }
    }
    if let Some(path) = glyphs.builder.finish() {
        pixmap.fill_path(&path, color, FillRule::Winding, transform, None);
    }

    let decorations = layout.decorations;
    for line in &layout.lines {
        if line.right <= line.left {
            continue;
        }
        let baseline = frame.y + line.baseline;
        let mut stroke = |offset: f32, thickness: f32| {
            let rect = (
                frame.x + line.left,
                baseline + offset,
                line.right - line.left,
                thickness,
            );
            fill_rect(pixmap, rect, color, transform);
        };
        if ink.underline {
            stroke(
                decorations.underline_offset,
                decorations.underline_thickness,
            );
        }
        if ink.strikethrough {
            stroke(
                decorations.strikeout_offset,
                decorations.strikeout_thickness,
            );
        }
    }
}

/// Renders one text box alone, turned by its rotation, at `scale` pixels per
/// slide unit. Returns the image and the area of the slide it covers: the
/// bounds of the frame and a pixel of margin. The pixels are premultiplied.
pub fn render_text_box(ink: &TextInk, frame: &Frame, scale: f32) -> Option<(Pixmap, Frame)> {
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    let bounds = frame.bounds();
    let margin = 1. / scale;
    // Glyphs may reach past the frame, such as the overflow of a fixed box.
    let content_bottom = ink
        .layout
        .lines
        .last()
        .map_or(0., |line| line.top + line.height);
    let reach = Frame {
        height: frame.height.max(content_bottom),
        ..*frame
    }
    .bounds();
    let area = Frame {
        x: bounds.x.min(reach.x) - margin,
        y: bounds.y.min(reach.y) - margin,
        width: bounds.width.max(reach.width) + 2. * margin,
        height: bounds.height.max(reach.height) + 2. * margin,
        rotation: 0.,
    };
    let width = (area.width * scale).ceil() as u32;
    let height = (area.height * scale).ceil() as u32;
    let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
    let transform = Transform::from_scale(scale, scale).pre_translate(-area.x, -area.y);
    draw_text(&mut pixmap, ink, frame, turned(transform, frame));
    let area = Frame {
        width: width as f32 / scale,
        height: height as f32 / scale,
        ..area
    };
    Some((pixmap, area))
}

/// Overlay drawn below the text: line boxes and the overflow band.
fn paint_overlay_under(
    pixmap: &mut Pixmap,
    frame: &Frame,
    layout: &TextLayout,
    transform: Transform,
) {
    let line_box = paint(ACCENT, 0.08);
    for line in &layout.lines {
        let rect = (
            frame.x + line.left,
            frame.y + line.top,
            (line.right - line.left).max(0.),
            line.height,
        );
        fill_rect(pixmap, rect, &line_box, transform);
    }
    let overflow = layout.overflow();
    if overflow > 0. {
        let content_bottom = layout
            .lines
            .last()
            .map_or(frame.height, |line| line.top + line.height);
        let top = frame.y + frame.height;
        let rect = (frame.x, top, frame.width, frame.y + content_bottom - top);
        fill_rect(pixmap, rect, &paint(WARN, 0.15), transform);
    }
}

/// Overlay drawn above the text: the frame and the baselines, one device
/// pixel wide whatever the scale.
fn paint_overlay_over(
    pixmap: &mut Pixmap,
    frame: &Frame,
    layout: &TextLayout,
    scale: f32,
    transform: Transform,
) {
    let pixel = 1. / scale;
    for line in &layout.lines {
        let rect = (
            frame.x,
            frame.y + line.baseline,
            frame.width.max(pixel),
            pixel,
        );
        fill_rect(pixmap, rect, &paint(GUIDE, 0.7), transform);
    }
    let color = if layout.overflow() > 0. { WARN } else { ACCENT };
    if let Some(rect) = Rect::from_xywh(
        frame.x,
        frame.y,
        frame.width.max(pixel),
        frame.height.max(pixel),
    ) {
        let path = PathBuilder::from_rect(rect);
        let stroke = Stroke {
            width: 1.5 * pixel,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint(color, 1.), &stroke, transform, None);
    }
}

/// What a debug run prints about one text box.
pub struct TextReport {
    pub id: ElementId,
    pub frame: Frame,
    pub lines: usize,
    pub overflow: f32,
    pub missing_glyphs: usize,
}

impl std::fmt::Display for TextReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let frame = &self.frame;
        write!(
            f,
            "text {}: frame {:.1},{:.1} {:.1}×{:.1}{}, {} line{}, overflow {:.1} px, missing glyphs {}",
            self.id.0,
            frame.x,
            frame.y,
            frame.width,
            frame.height,
            if frame.rotation == 0. {
                String::new()
            } else {
                format!(" turned {:.1}°", frame.rotation)
            },
            self.lines,
            if self.lines == 1 { "" } else { "s" },
            self.overflow,
            self.missing_glyphs
        )
    }
}

/// Reports every visible text box of a slide, in paint order.
pub fn report(presentation: &Presentation, slide: SlideId) -> Vec<TextReport> {
    let Some(slide) = presentation.slide(slide) else {
        return Vec::new();
    };
    slide
        .visible_texts()
        .filter_map(|(element, _)| {
            let layout = presentation.text_layout(element.id)?;
            Some(TextReport {
                id: element.id,
                frame: element.frame,
                lines: layout.lines.len(),
                overflow: layout.overflow(),
                missing_glyphs: layout.missing_glyphs,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script;

    fn scene(json: &str) -> Presentation {
        let mut presentation = Presentation::new();
        script::apply(&mut presentation, script::parse(json).unwrap()).unwrap();
        presentation
    }

    /// Whether the pixel is visibly not white.
    fn inked(pixmap: &Pixmap, x: u32, y: u32) -> bool {
        let pixel = pixmap.pixel(x, y).unwrap();
        pixel.red() < 200 || pixel.green() < 200 || pixel.blue() < 200
    }

    /// Inked pixels inside and outside a rectangle (in pixels).
    fn ink(pixmap: &Pixmap, left: u32, top: u32, right: u32, bottom: u32) -> (usize, usize) {
        let (mut inside, mut outside) = (0, 0);
        for y in 0..pixmap.height() {
            for x in 0..pixmap.width() {
                if inked(pixmap, x, y) {
                    if (left..right).contains(&x) && (top..bottom).contains(&y) {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                }
            }
        }
        (inside, outside)
    }

    const TEXT: &str = r#"[
      {"op": "add_font", "face": {"family": "Inter"}},
      {"op": "add_element", "slide": 1, "element": {
        "id": 1, "frame": {"x": 100, "y": 100},
        "text": {"content": "Hello", "style": {"size": 64}}}}
    ]"#;

    #[test]
    fn text_is_drawn_inside_its_frame() {
        let presentation = scene(TEXT);
        let frame = presentation.element(ElementId(1)).unwrap().frame;
        let pixmap = render_slide(&presentation, SlideId(1), 1., false).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (1600, 900));
        let (inside, outside) = ink(
            &pixmap,
            frame.x as u32,
            frame.y as u32,
            (frame.x + frame.width).ceil() as u32 + 1,
            (frame.y + frame.height).ceil() as u32 + 1,
        );
        assert!(inside > 500, "the glyphs are filled: {inside}");
        assert_eq!(outside, 0);
    }

    #[test]
    fn rotated_text_is_drawn_turned_around_its_center() {
        let presentation = scene(
            r#"[
          {"op": "add_font", "face": {"family": "Inter"}},
          {"op": "add_element", "slide": 1, "element": {
            "id": 1, "frame": {"x": 400, "y": 400, "rotation": 90},
            "text": {"content": "Hello world", "style": {"size": 64}}}}
        ]"#,
        );
        let frame = presentation.element(ElementId(1)).unwrap().frame;
        assert!(frame.width > frame.height * 2., "a wide line: {frame:?}");
        let bounds = frame.bounds();
        let pixmap = render_slide(&presentation, SlideId(1), 1., false).unwrap();
        let (inside, outside) = ink(
            &pixmap,
            bounds.x as u32,
            bounds.y as u32,
            (bounds.x + bounds.width).ceil() as u32 + 1,
            (bounds.y + bounds.height).ceil() as u32 + 1,
        );
        assert!(inside > 500, "the glyphs are filled: {inside}");
        assert_eq!(outside, 0, "the text stands upright in its turned box");
        assert!(
            report(&presentation, SlideId(1))[0]
                .to_string()
                .contains("turned 90.0°")
        );
    }

    #[test]
    fn scale_changes_the_image_size() {
        let presentation = scene(TEXT);
        let pixmap = render_slide(&presentation, SlideId(1), 0.5, false).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (800, 450));
        assert_eq!(
            render_slide(&presentation, SlideId(9), 1., false).unwrap_err(),
            RenderError::UnknownSlide(SlideId(9))
        );
    }

    #[test]
    fn overflow_is_drawn_below_a_fixed_box_and_tinted_by_the_overlay() {
        let presentation = scene(
            r#"[
          {"op": "add_font", "face": {"family": "Inter"}},
          {"op": "add_element", "slide": 1, "element": {
            "id": 1, "frame": {"x": 100, "y": 100, "width": 300, "height": 40},
            "text": {"content": "One\nTwo\nThree", "sizing": "fixed"}}}
        ]"#,
        );
        let plain = render_slide(&presentation, SlideId(1), 1., false).unwrap();
        let below = (145..240).any(|y| (100..400).any(|x| inked(&plain, x, y)));
        assert!(below, "text runs past the bottom of the box");

        let overlay = render_slide(&presentation, SlideId(1), 1., true).unwrap();
        // The frame's left edge is stroked in the warning color.
        let edge = overlay.pixel(100, 120).unwrap();
        assert!(edge.red() > edge.blue(), "orange frame: {edge:?}");
        assert!(report(&presentation, SlideId(1))[0].overflow > 0.);
    }

    #[test]
    fn reports_read_like_a_summary() {
        let presentation = scene(TEXT);
        let line = report(&presentation, SlideId(1))[0].to_string();
        assert!(line.starts_with("text 1: frame 100.0,100.0 "), "{line}");
        assert!(
            line.ends_with("1 line, overflow 0.0 px, missing glyphs 0"),
            "{line}"
        );
    }

    fn rgb(pixmap: &Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
        let pixel = pixmap.pixel(x, y).unwrap();
        (pixel.red(), pixel.green(), pixel.blue())
    }

    fn shapes(elements: &str) -> Pixmap {
        let ops = format!(r#"[{{"op": "add_element", "slide": 1, "element": {elements}}}]"#);
        render_slide(&scene(&ops), SlideId(1), 1., false).unwrap()
    }

    #[test]
    fn rectangles_fill_their_frame_but_not_rounded_corners() {
        let pixmap = shapes(
            r#"{"frame": {"x": 100, "y": 100, "width": 200, "height": 100},
                "rectangle": {"corner_radius": 40, "fill": {"solid": {"color": "000000"}}}}"#,
        );
        assert_eq!(rgb(&pixmap, 200, 150), (0, 0, 0));
        assert!(!inked(&pixmap, 102, 102), "the corner is round");
        assert!(!inked(&pixmap, 305, 150), "nothing past the frame");
    }

    #[test]
    fn ellipses_leave_the_corners_of_their_frame_empty() {
        let pixmap = shapes(
            r#"{"frame": {"x": 100, "y": 100, "width": 200, "height": 200},
                "ellipse": {"fill": {"solid": {"color": "000000"}}}}"#,
        );
        assert!(inked(&pixmap, 200, 200));
        assert!(!inked(&pixmap, 110, 110));
    }

    #[test]
    fn dashed_strokes_have_gaps() {
        let pixmap = shapes(
            r#"{"line": {"from": {"x": 100, "y": 100}, "to": {"x": 500, "y": 100},
                "stroke": {"width": 4, "dash": "dashed"}}}"#,
        );
        let inked_run: Vec<bool> = (100..500).map(|x| inked(&pixmap, x, 100)).collect();
        assert!(inked_run.iter().any(|inked| *inked));
        assert!(inked_run.iter().any(|inked| !*inked), "dashes leave gaps");
    }

    #[test]
    fn linear_gradients_follow_their_angle() {
        let pixmap = shapes(
            r#"{"frame": {"x": 100, "y": 100, "width": 200, "height": 200},
                "rectangle": {"fill": {"linear_gradient": {"angle": 90, "stops": [
                  {"position": 0, "color": "000000"}, {"position": 1, "color": "FFFFFF"}]}}}}"#,
        );
        let top = rgb(&pixmap, 200, 105).0;
        let bottom = rgb(&pixmap, 200, 295).0;
        assert!(top < 30 && bottom > 225, "{top} {bottom}");
        let left = rgb(&pixmap, 105, 200).0;
        let right = rgb(&pixmap, 295, 200).0;
        assert!(left.abs_diff(right) < 3, "{left} {right}");
    }

    #[test]
    fn radial_gradients_start_at_their_center() {
        let pixmap = shapes(
            r#"{"frame": {"x": 100, "y": 100, "width": 200, "height": 100},
                "rectangle": {"fill": {"radial_gradient": {"stops": [
                  {"position": 0, "color": "FFFFFF"}, {"position": 1, "color": "000000"}]}}}}"#,
        );
        let center = rgb(&pixmap, 200, 150).0;
        let edge = rgb(&pixmap, 295, 150).0;
        let side = rgb(&pixmap, 200, 102).0;
        assert!(
            center > 240 && edge < 20 && side < 20,
            "{center} {edge} {side}"
        );
    }

    #[test]
    fn turned_lines_are_drawn_between_their_ends() {
        let pixmap =
            shapes(r#"{"line": {"from": {"x": 100, "y": 100}, "to": {"x": 300, "y": 300}}}"#);
        assert!(inked(&pixmap, 200, 200));
        assert!(!inked(&pixmap, 200, 120));
    }

    #[test]
    fn large_heads_are_wider_than_small_ones() {
        let head = |size: &str| {
            let pixmap = shapes(&format!(
                r#"{{"line": {{"from": {{"x": 100, "y": 100}}, "to": {{"x": 500, "y": 100}},
                    "stroke": {{"width": 4}}, "end": {{"kind": "triangle", "size": "{size}"}}}}}}"#
            ));
            // 15 units behind the tip: past a small head, inside a large one.
            (80..120).filter(|y| inked(&pixmap, 485, *y)).count()
        };
        let (small, large) = (head("small"), head("large"));
        assert!(large > small && small > 0, "{small} {large}");
    }

    #[test]
    fn element_opacity_fades_shapes_and_hidden_ones_are_not_drawn() {
        let faded = shapes(
            r#"{"opacity": 0.5, "frame": {"x": 100, "y": 100, "width": 100, "height": 100},
                "rectangle": {"fill": {"solid": {"color": "000000"}}}}"#,
        );
        let gray = rgb(&faded, 150, 150).0;
        assert!((120..136).contains(&gray), "{gray}");
        let hidden = shapes(
            r#"{"hidden": true, "frame": {"x": 100, "y": 100, "width": 100, "height": 100},
                "rectangle": {}}"#,
        );
        assert!(!inked(&hidden, 150, 150));
    }

    fn image_scene(fit: &str) -> Pixmap {
        use base64::Engine as _;
        let data =
            base64::engine::general_purpose::STANDARD.encode(crate::images::tests::png(200, 100));
        let ops = format!(
            r#"[{{"op": "add_image", "id": "$i", "data": "{data}"}},
                {{"op": "add_element", "slide": 1, "element": {{
                  "frame": {{"x": 100, "y": 100, "width": 200, "height": 200}},
                  "rectangle": {{"fill": {{"image": {{"id": "$i", "fit": "{fit}"}}}}}}}}}}]"#
        );
        render_slide(&scene(&ops), SlideId(1), 1., false).unwrap()
    }

    #[test]
    fn cover_fills_the_box_and_contain_leaves_bars() {
        let cover = image_scene("cover");
        assert_eq!(rgb(&cover, 110, 110), (255, 0, 0));
        assert_eq!(rgb(&cover, 290, 290), (0, 0, 255));
        let contain = image_scene("contain");
        assert_eq!(
            rgb(&contain, 200, 110),
            (255, 255, 255),
            "a bar above the image"
        );
        assert_eq!(rgb(&contain, 110, 200), (255, 0, 0));
        let stretch = image_scene("stretch");
        assert_eq!(rgb(&stretch, 110, 110), (255, 0, 0));
        assert_eq!(rgb(&stretch, 290, 110), (0, 0, 255));
    }
}
