//! Paints rectangles, ellipses and lines on the canvas and in the
//! thumbnails, with the geometry of `shape`.
//!
//! Outlines, strokes, arrowheads, solid fills and the linear gradients that
//! GPUI draws exactly are tessellated GPUI paths, cached without their
//! position so that a move or a pan reuses them. Other fills (radial
//! gradients, linear gradients at other angles, images) come from the CPU
//! renderer as cached images, drawn at a zoom rounded up to a power of √2
//! so that a zoom gesture does not render them again at every frame.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{
    Background, Hsla, PathBuilder, PathStyle, Pixels, Point, RenderImage, StrokeOptions, Window,
    linear_color_stop, linear_gradient, point, px, size,
};

use crate::document::{Element, ElementId, ElementKind, Fill, Frame, Rgb};
use crate::render::{self, Pixmap};
use crate::shape::{self, Head, Seg};

/// A shape ready to paint: a copy of the element with the frame the canvas
/// shows, its opacity with its groups', and the picture of its image, video
/// or shader fill when it is ready.
#[derive(Clone, Debug)]
pub struct PaintShape {
    pub element: Element,
    pub opacity: f32,
    pub picture: Option<Arc<Pixmap>>,
}

fn hsla(color: Rgb, alpha: f32) -> Hsla {
    let color: Hsla = gpui_kit::rgb(color.0).into();
    color.opacity(alpha.clamp(0., 1.))
}

/// The paths of a shape, in window pixels from the center of its frame.
#[derive(Default)]
struct ShapePaths {
    fill: Option<gpui_kit::Path<Pixels>>,
    stroke: Option<gpui_kit::Path<Pixels>>,
    /// Filled arrowheads.
    heads: Option<gpui_kit::Path<Pixels>>,
    /// Open arrowheads, stroked.
    open_heads: Option<gpui_kit::Path<Pixels>>,
}

/// What a tessellation depends on; the position is left out.
#[derive(Clone, PartialEq)]
struct PathKey {
    kind: ElementKind,
    width: f32,
    height: f32,
    rotation: f32,
    zoom: f32,
}

/// What a rendered fill depends on; the position is left out.
#[derive(Clone, PartialEq)]
struct RasterKey {
    kind: ElementKind,
    width: f32,
    height: f32,
    rotation: f32,
    /// Device pixels per slide unit, rounded up to a power of √2.
    scale: f32,
    opacity: f32,
    /// Address of the picture of the fill; 0 for the placeholder.
    picture: usize,
}

/// Where a cache entry lives: the element, the zoom it is drawn at and
/// whether the image holds the whole shape, so that the canvas and the
/// thumbnails keep their own entries.
type Slot = (ElementId, u32, bool);

/// Tessellations and rendered fills of shapes. A slot holds the last one
/// made for it; its key tells whether it still fits the shape.
#[derive(Default)]
struct ShapeCache {
    paths: HashMap<Slot, (PathKey, Box<ShapePaths>, u64)>,
    /// The image and the area it covers, in slide units from the center of
    /// the frame.
    rasters: HashMap<Slot, (RasterKey, Arc<RenderImage>, Frame, u64)>,
    /// Counts the calls to [`paint_shape`]; entries unused for
    /// [`CACHE_KEEP`] calls go when a map grows past [`CACHE_ENTRIES`].
    clock: u64,
}

const CACHE_ENTRIES: usize = 2048;
const CACHE_KEEP: u64 = 8192;

thread_local! {
    static CACHE: std::cell::RefCell<ShapeCache> = std::cell::RefCell::default();
}

impl ShapeCache {
    /// Drops old entries when a map is full; returns the images that leave.
    fn trim(&mut self) -> Vec<Arc<RenderImage>> {
        let clock = self.clock;
        let recent = |used: u64| clock - used < CACHE_KEEP;
        if self.paths.len() > CACHE_ENTRIES {
            self.paths.retain(|_, (_, _, used)| recent(*used));
            if self.paths.len() > CACHE_ENTRIES {
                self.paths.clear();
            }
        }
        let mut evicted = Vec::new();
        if self.rasters.len() > CACHE_ENTRIES {
            let old: Vec<Slot> = self
                .rasters
                .iter()
                .filter(|(_, (_, _, _, used))| !recent(*used))
                .map(|(slot, _)| *slot)
                .collect();
            for slot in old {
                evicted.extend(self.rasters.remove(&slot).map(|entry| entry.1));
            }
            if self.rasters.len() > CACHE_ENTRIES {
                evicted.extend(self.rasters.drain().map(|(_, entry)| entry.1));
            }
        }
        evicted
    }
}

/// Whether GPUI draws the fill exactly as a path background.
fn native_fill(fill: &Fill, rotation: f32) -> bool {
    let square = |degrees: f32| degrees.rem_euclid(90.) == 0.;
    match fill {
        Fill::None | Fill::Solid(_) => true,
        // GPUI stretches the gradient to the bounds of the path, which is
        // exact only along an axis of the screen.
        Fill::LinearGradient(gradient) => {
            gradient.stops.len() == 2 && square(gradient.angle) && square(rotation)
        }
        Fill::RadialGradient(_) | Fill::Image(_) | Fill::Video(_) | Fill::Shader(_) => false,
    }
}

/// The background of a fill that [`native_fill`] accepts.
fn background(fill: &Fill, rotation: f32, opacity: f32) -> Option<Background> {
    match fill {
        Fill::None
        | Fill::RadialGradient(_)
        | Fill::Image(_)
        | Fill::Video(_)
        | Fill::Shader(_) => None,
        Fill::Solid(solid) => Some(hsla(solid.color, solid.opacity * opacity).into()),
        Fill::LinearGradient(gradient) => {
            let [from, to] = [gradient.stops[0], gradient.stops[1]];
            let stop = |stop: crate::document::GradientStop| {
                linear_color_stop(hsla(stop.color, stop.opacity * opacity), stop.position)
            };
            // CSS angles start at the top; ours start at the left.
            let angle = (gradient.angle + rotation + 90.).rem_euclid(360.);
            Some(linear_gradient(angle, stop(from), stop(to)))
        }
    }
}

/// Builds the outline steps into a path, from local units through `place`.
fn add_segs(builder: &mut PathBuilder, segs: &[Seg], place: &dyn Fn(f32, f32) -> Point<Pixels>) {
    for seg in segs {
        match *seg {
            Seg::Move((x, y)) => builder.move_to(place(x, y)),
            Seg::Line((x, y)) => builder.line_to(place(x, y)),
            Seg::Cubic((x1, y1), (x2, y2), (x, y)) => {
                builder.cubic_bezier_to(place(x, y), place(x1, y1), place(x2, y2))
            }
            Seg::Close => builder.close(),
        }
    }
}

fn stroke_builder(width: f32, dash: Option<[f32; 2]>, zoom: f32) -> PathBuilder {
    // At least one screen pixel, so that a thin stroke seen from afar does
    // not vanish between two rows of pixels.
    let shown = (width * zoom).max(1.);
    // Butt caps and miter joins, as in every export.
    let options = StrokeOptions::default().with_line_width(shown);
    let builder = PathBuilder::stroke(px(shown)).with_style(PathStyle::Stroke(options));
    match dash {
        Some([on, off]) => builder.dash_array(&[px(on * zoom), px(off * zoom)]),
        None => builder,
    }
}

fn tessellate(element: &Element, zoom: f32, with_fill: bool) -> ShapePaths {
    let _span = crate::perf::span("tessellate_shape");
    let frame = element.frame;
    let (cx, cy) = frame.center();
    // Local units of a line start on its axis.
    let axis = if element.is_line() {
        frame.height / 2.
    } else {
        0.
    };
    let place = |x: f32, y: f32| {
        let (x, y) = frame.to_slide(x, y + axis);
        point(px((x - cx) * zoom), px((y - cy) * zoom))
    };
    let mut paths = ShapePaths::default();
    if let ElementKind::Line(line) = &element.kind {
        let geometry = shape::line_geometry(frame.width, line);
        let stroke = &line.stroke;
        if let Some((from, to)) = geometry.segment {
            let mut builder = stroke_builder(
                stroke.width,
                shape::dash_array(stroke.dash, stroke.width),
                zoom,
            );
            builder.move_to(place(from.0, from.1));
            builder.line_to(place(to.0, to.1));
            paths.stroke = builder.build().ok();
        }
        let mut filled = PathBuilder::fill();
        let mut open = stroke_builder(stroke.width, None, zoom);
        let (mut any_filled, mut any_open) = (false, false);
        for head in &geometry.heads {
            match head {
                Head::Filled(segs) => {
                    add_segs(&mut filled, segs, &place);
                    any_filled = true;
                }
                Head::Open(points) => {
                    open.move_to(place(points[0].0, points[0].1));
                    open.line_to(place(points[1].0, points[1].1));
                    open.line_to(place(points[2].0, points[2].1));
                    any_open = true;
                }
            }
        }
        paths.heads = any_filled.then(|| filled.build().ok()).flatten();
        paths.open_heads = any_open.then(|| open.build().ok()).flatten();
        return paths;
    }
    let Some(segs) = shape::outline(&element.kind, frame.width, frame.height) else {
        return paths;
    };
    if with_fill {
        let mut builder = PathBuilder::fill();
        add_segs(&mut builder, &segs, &place);
        paths.fill = builder.build().ok();
    }
    if let Some(stroke) = element.kind.stroke() {
        let mut builder = stroke_builder(
            stroke.width,
            shape::dash_array(stroke.dash, stroke.width),
            zoom,
        );
        add_segs(&mut builder, &segs, &place);
        paths.stroke = builder.build().ok();
    }
    paths
}

/// The path moved by `by`.
pub fn translated(path: &gpui_kit::Path<Pixels>, by: Point<Pixels>) -> gpui_kit::Path<Pixels> {
    let mut path = path.clone();
    path.bounds.origin += by;
    for vertex in &mut path.vertices {
        vertex.xy_position += by;
    }
    path
}

/// Converts premultiplied pixels to an image as GPUI takes it: BGRA with
/// straight alpha.
pub fn render_image(pixmap: &Pixmap) -> Option<RenderImage> {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.pixels() {
        let pixel = pixel.demultiply();
        bytes.extend_from_slice(&[pixel.blue(), pixel.green(), pixel.red(), pixel.alpha()]);
    }
    let buffer = image::RgbaImage::from_raw(width, height, bytes)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// The smallest power of √2 at or above `scale`.
fn scale_bucket(scale: f32) -> f32 {
    let step = std::f32::consts::SQRT_2;
    step.powf((scale.max(1e-3).ln() / step.ln()).ceil())
}

/// Most pixels on a side of a rendered fill.
const MAX_RASTER: f32 = 4096.;

/// Renders the fill of a shape at a scale bucket, in slide units from the
/// center of its frame.
/// Renders the fill of a shape, or with `whole` the whole shape, at
/// `scale` device pixels per slide unit, in slide units from the center of
/// its frame.
fn rasterize(shape: &PaintShape, scale: f32, whole: bool) -> Option<(Arc<RenderImage>, Frame)> {
    let _span = crate::perf::span("rasterize_shape");
    let element = &shape.element;
    let mut fill_only = element.clone();
    if !whole {
        match &mut fill_only.kind {
            ElementKind::Rectangle(shape) => shape.stroke = None,
            ElementKind::Ellipse(shape) => shape.stroke = None,
            _ => return None,
        }
    }
    let bounds = fill_only.frame.bounds();
    let largest = bounds.width.max(bounds.height).max(1.);
    let scale = scale.min(MAX_RASTER / largest);
    let pixels = |_: &_, _| shape.picture.clone();
    let (pixmap, area) = render::render_shape_box(&fill_only, shape.opacity, scale, &pixels)?;
    let image = render_image(&pixmap)?;
    let (cx, cy) = element.frame.center();
    Some((
        Arc::new(image),
        Frame {
            x: area.x - cx,
            y: area.y - cy,
            ..area
        },
    ))
}

/// Paints one shape. `origin` is the window position of the slide's
/// top-left corner. With `raster` (the thumbnails, which draw every slide
/// at every frame), the whole shape is one cached image at the exact zoom:
/// one image costs less to paint than its paths.
pub fn paint_shape(
    shape: &PaintShape,
    origin: Point<Pixels>,
    zoom: f32,
    raster: bool,
    window: &mut Window,
) {
    let _span = crate::perf::span("paint_shape");
    let element = &shape.element;
    let frame = element.frame;
    let (cx, cy) = frame.center();
    let center = origin + point(px(cx * zoom), px(cy * zoom));
    if raster {
        paint_whole_raster(shape, center, zoom, window);
        return;
    }
    let fill = element.kind.fill();
    let native = fill.is_none_or(|fill| native_fill(fill, frame.rotation));
    let path_key = PathKey {
        kind: element.kind.clone(),
        width: frame.width,
        height: frame.height,
        rotation: frame.rotation,
        zoom,
    };
    let raster_key = (!native).then(|| RasterKey {
        kind: element.kind.clone(),
        width: frame.width,
        height: frame.height,
        rotation: frame.rotation,
        scale: scale_bucket(zoom * window.scale_factor()),
        opacity: shape.opacity,
        picture: shape
            .picture
            .as_ref()
            .map_or(0, |picture| Arc::as_ptr(picture) as usize),
    });

    let slot = (element.id, zoom.to_bits(), false);
    let mut evicted = Vec::new();
    let (paths, raster) = CACHE.with_borrow_mut(|cache| {
        cache.clock += 1;
        let clock = cache.clock;
        let paths = match cache.paths.get_mut(&slot) {
            Some((key, paths, used)) if *key == path_key => {
                *used = clock;
                place_paths(paths, center)
            }
            _ => {
                let tessellated = Box::new(tessellate(element, zoom, native));
                let placed = place_paths(&tessellated, center);
                cache.paths.insert(slot, (path_key, tessellated, clock));
                placed
            }
        };
        let raster = raster_key.and_then(|raster_key| match cache.rasters.get_mut(&slot) {
            Some((key, image, area, used)) if *key == raster_key => {
                *used = clock;
                Some((image.clone(), *area))
            }
            _ => {
                let (image, area) = rasterize(shape, raster_key.scale, false)?;
                let old = cache
                    .rasters
                    .insert(slot, (raster_key, image.clone(), area, clock));
                evicted.extend(old.map(|entry| entry.1));
                Some((image, area))
            }
        });
        evicted.extend(cache.trim());
        (paths, raster)
    });
    for image in evicted {
        window.drop_image(image).ok();
    }

    if let Some((image, area)) = raster {
        let bounds = gpui_kit::Bounds::new(
            center + point(px(area.x * zoom), px(area.y * zoom)),
            size(px(area.width * zoom), px(area.height * zoom)),
        );
        window
            .paint_image(bounds, bounds, Default::default(), image, 0, false)
            .ok();
    }
    let _span = crate::perf::span("paint_shape_paths");
    if let (Some(path), Some(fill)) = (paths.fill, fill)
        && let Some(background) = background(fill, frame.rotation, shape.opacity)
    {
        window.paint_path(path, background);
    }
    if let Some(stroke) = element.kind.stroke() {
        let color = hsla(stroke.color, stroke.opacity * shape.opacity);
        for path in [paths.stroke, paths.heads, paths.open_heads]
            .into_iter()
            .flatten()
        {
            window.paint_path(path, color);
        }
    }
}

/// Paints a whole shape as one cached image; see [`paint_shape`].
fn paint_whole_raster(shape: &PaintShape, center: Point<Pixels>, zoom: f32, window: &mut Window) {
    let element = &shape.element;
    let frame = element.frame;
    let key = RasterKey {
        kind: element.kind.clone(),
        width: frame.width,
        height: frame.height,
        rotation: frame.rotation,
        scale: zoom * window.scale_factor(),
        opacity: shape.opacity,
        picture: shape
            .picture
            .as_ref()
            .map_or(0, |picture| Arc::as_ptr(picture) as usize),
    };
    let slot = (element.id, zoom.to_bits(), true);
    let mut evicted = Vec::new();
    let raster = CACHE.with_borrow_mut(|cache| {
        cache.clock += 1;
        let clock = cache.clock;
        let raster = match cache.rasters.get_mut(&slot) {
            Some((current, image, area, used)) if *current == key => {
                *used = clock;
                Some((image.clone(), *area))
            }
            _ => rasterize(shape, key.scale, true).map(|(image, area)| {
                let old = cache
                    .rasters
                    .insert(slot, (key, image.clone(), area, clock));
                evicted.extend(old.map(|entry| entry.1));
                (image, area)
            }),
        };
        evicted.extend(cache.trim());
        raster
    });
    for image in evicted {
        window.drop_image(image).ok();
    }
    if let Some((image, area)) = raster {
        let bounds = gpui_kit::Bounds::new(
            center + point(px(area.x * zoom), px(area.y * zoom)),
            size(px(area.width * zoom), px(area.height * zoom)),
        );
        window
            .paint_image(bounds, bounds, Default::default(), image, 0, false)
            .ok();
    }
}

fn place_paths(paths: &ShapePaths, center: Point<Pixels>) -> ShapePaths {
    let _span = crate::perf::span("place_shape_paths");
    let place =
        |path: &Option<gpui_kit::Path<Pixels>>| path.as_ref().map(|path| translated(path, center));
    ShapePaths {
        fill: place(&paths.fill),
        stroke: place(&paths.stroke),
        heads: place(&paths.heads),
        open_heads: place(&paths.open_heads),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{GradientStop, LinearGradient};

    #[test]
    fn scales_round_up_to_a_power_of_the_square_root_of_two() {
        assert!((scale_bucket(1.) - 1.).abs() < 1e-4);
        assert!((scale_bucket(1.2) - std::f32::consts::SQRT_2).abs() < 1e-4);
        assert!((scale_bucket(0.6) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn only_gradients_along_the_screen_axes_are_native() {
        let gradient = |angle: f32| {
            Fill::LinearGradient(LinearGradient {
                angle,
                stops: vec![
                    GradientStop::new(0., Rgb(0)),
                    GradientStop::new(1., Rgb(0xFFFFFF)),
                ],
            })
        };
        assert!(native_fill(&gradient(90.), 0.));
        assert!(native_fill(&gradient(0.), -90.));
        assert!(!native_fill(&gradient(45.), 0.));
        assert!(!native_fill(&gradient(0.), 30.));
    }
}
