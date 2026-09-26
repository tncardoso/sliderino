//! Geometry of direct manipulation on the canvas: resizing a frame from one
//! of its handles and snapping moved or resized frames to the slide, to the
//! other elements and to text baselines. Slide units throughout.

use crate::document::Frame;

/// The eight resize handles around a selected frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// Which horizontal edge the handle drags: -1 left, 1 right, 0 none.
    fn dx(self) -> f32 {
        match self {
            Handle::TopLeft | Handle::Left | Handle::BottomLeft => -1.,
            Handle::TopRight | Handle::Right | Handle::BottomRight => 1.,
            Handle::Top | Handle::Bottom => 0.,
        }
    }

    /// Which vertical edge the handle drags: -1 top, 1 bottom, 0 none.
    fn dy(self) -> f32 {
        match self {
            Handle::TopLeft | Handle::Top | Handle::TopRight => -1.,
            Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => 1.,
            Handle::Left | Handle::Right => 0.,
        }
    }

    /// The handle sits on the left or right edge only.
    pub fn is_side(self) -> bool {
        self.dy() == 0.
    }

    /// Where the handle sits on `frame`.
    pub fn position(self, frame: &Frame) -> (f32, f32) {
        (
            frame.x + frame.width * (self.dx() + 1.) / 2.,
            frame.y + frame.height * (self.dy() + 1.) / 2.,
        )
    }
}

/// Smallest width or height a resize leaves.
pub const MIN_SIZE: f32 = 1.;

/// Modifier behavior of a resize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeMode {
    /// Keep the aspect ratio of the original frame (Shift).
    pub keep_ratio: bool,
    /// Resize around the center instead of the opposite edge (Alt).
    pub from_center: bool,
}

/// The frame after dragging `handle` of `origin` by (`dx`, `dy`).
pub fn resize(origin: &Frame, handle: Handle, dx: f32, dy: f32, mode: ResizeMode) -> Frame {
    let (hx, hy) = (handle.dx(), handle.dy());
    let grow = if mode.from_center { 2. } else { 1. };
    let mut width = (origin.width + hx * dx * grow).max(MIN_SIZE);
    let mut height = (origin.height + hy * dy * grow).max(MIN_SIZE);
    if hx == 0. {
        width = origin.width;
    }
    if hy == 0. {
        height = origin.height;
    }

    if mode.keep_ratio && origin.width > 0. && origin.height > 0. {
        let ratio = origin.width / origin.height;
        let scale_x = width / origin.width;
        let scale_y = height / origin.height;
        let scale = match (hx != 0., hy != 0.) {
            (true, true) => {
                if (scale_x - 1.).abs() >= (scale_y - 1.).abs() {
                    scale_x
                } else {
                    scale_y
                }
            }
            (true, false) => scale_x,
            _ => scale_y,
        };
        width = (origin.width * scale).max(MIN_SIZE);
        height = (width / ratio).max(MIN_SIZE);
    }

    // Anchor: the center, or the edge opposite to each dragged one. An axis
    // the handle does not drag stays centered when its size changes.
    let center_x = origin.x + origin.width / 2.;
    let center_y = origin.y + origin.height / 2.;
    let x = if mode.from_center || hx == 0. {
        center_x - width / 2.
    } else if hx < 0. {
        origin.x + origin.width - width
    } else {
        origin.x
    };
    let y = if mode.from_center || hy == 0. {
        center_y - height / 2.
    } else if hy < 0. {
        origin.y + origin.height - height
    } else {
        origin.y
    };
    Frame {
        x,
        y,
        width,
        height,
        rotation: origin.rotation,
    }
}

/// A snap guide line drawn across the slide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide {
    /// A vertical line at this x.
    Vertical(f32),
    /// A horizontal line at this y.
    Horizontal(f32),
}

/// What moved or resized frames snap to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Targets {
    /// Vertical lines: slide edges and center, element edges and centers.
    pub xs: Vec<f32>,
    /// Horizontal lines: slide edges and middle, element edges and middles.
    pub ys: Vec<f32>,
    /// First-line baselines of the other text boxes.
    pub baselines: Vec<f32>,
}

impl Targets {
    pub fn new(slide_width: f32, slide_height: f32) -> Self {
        Self {
            xs: vec![0., slide_width / 2., slide_width],
            ys: vec![0., slide_height / 2., slide_height],
            baselines: Vec::new(),
        }
    }

    pub fn add_frame(&mut self, frame: &Frame) {
        self.xs
            .extend([frame.x, frame.x + frame.width / 2., frame.x + frame.width]);
        self.ys
            .extend([frame.y, frame.y + frame.height / 2., frame.y + frame.height]);
    }
}

/// The smallest shift within `threshold` that puts one of `points` on one of
/// `lines`, and the line it lands on.
fn nearest(points: &[f32], lines: &[f32], threshold: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &point in points {
        for &line in lines {
            let shift = line - point;
            if shift.abs() <= threshold && best.is_none_or(|(b, _)| shift.abs() < b.abs()) {
                best = Some((shift, line));
            }
        }
    }
    best
}

/// Snaps a moved frame: its edges and center to the target lines, and its
/// first baseline (an offset from its top, for text) to other baselines.
/// Returns the snapped frame and the guides to draw.
pub fn snap_move(
    frame: &Frame,
    baseline: Option<f32>,
    targets: &Targets,
    threshold: f32,
) -> (Frame, Vec<Guide>) {
    let mut snapped = *frame;
    let mut guides = Vec::new();
    let xs = [frame.x, frame.x + frame.width / 2., frame.x + frame.width];
    if let Some((shift, line)) = nearest(&xs, &targets.xs, threshold) {
        snapped.x += shift;
        guides.push(Guide::Vertical(line));
    }
    let ys = [frame.y, frame.y + frame.height / 2., frame.y + frame.height];
    let by_edge = nearest(&ys, &targets.ys, threshold);
    let by_baseline =
        baseline.and_then(|offset| nearest(&[frame.y + offset], &targets.baselines, threshold));
    let best = match (by_edge, by_baseline) {
        (Some(edge), Some(base)) if base.0.abs() <= edge.0.abs() => Some(base),
        (edge, None) => edge,
        (Some(edge), Some(_)) => Some(edge),
        (None, base) => base,
    };
    if let Some((shift, line)) = best {
        snapped.y += shift;
        guides.push(Guide::Horizontal(line));
    }
    (snapped, guides)
}

/// Snaps the edges a resize drags. `origin` and `handle` tell which edges
/// move; the others stay put.
pub fn snap_resize(
    frame: &Frame,
    handle: Handle,
    targets: &Targets,
    threshold: f32,
) -> (Frame, Vec<Guide>) {
    let mut snapped = *frame;
    let mut guides = Vec::new();
    let right = frame.x + frame.width;
    let bottom = frame.y + frame.height;
    match handle.dx() {
        dx if dx < 0. => {
            if let Some((shift, line)) = nearest(&[frame.x], &targets.xs, threshold)
                && frame.width - shift >= MIN_SIZE
            {
                snapped.x += shift;
                snapped.width -= shift;
                guides.push(Guide::Vertical(line));
            }
        }
        dx if dx > 0. => {
            if let Some((shift, line)) = nearest(&[right], &targets.xs, threshold)
                && frame.width + shift >= MIN_SIZE
            {
                snapped.width += shift;
                guides.push(Guide::Vertical(line));
            }
        }
        _ => {}
    }
    match handle.dy() {
        dy if dy < 0. => {
            if let Some((shift, line)) = nearest(&[frame.y], &targets.ys, threshold)
                && frame.height - shift >= MIN_SIZE
            {
                snapped.y += shift;
                snapped.height -= shift;
                guides.push(Guide::Horizontal(line));
            }
        }
        dy if dy > 0. => {
            if let Some((shift, line)) = nearest(&[bottom], &targets.ys, threshold)
                && frame.height + shift >= MIN_SIZE
            {
                snapped.height += shift;
                guides.push(Guide::Horizontal(line));
            }
        }
        _ => {}
    }
    (snapped, guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f32, y: f32, width: f32, height: f32) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
            rotation: 0.,
        }
    }

    #[test]
    fn corner_resize_moves_only_the_dragged_edges() {
        let origin = frame(100., 100., 200., 100.);
        let resized = resize(&origin, Handle::TopLeft, -10., 20., ResizeMode::default());
        assert_eq!(resized, frame(90., 120., 210., 80.));
        let resized = resize(&origin, Handle::Right, 50., 999., ResizeMode::default());
        assert_eq!(resized, frame(100., 100., 250., 100.));
    }

    #[test]
    fn resize_never_goes_below_the_minimum() {
        let origin = frame(0., 0., 20., 20.);
        let resized = resize(
            &origin,
            Handle::BottomRight,
            -100.,
            -100.,
            ResizeMode::default(),
        );
        assert_eq!((resized.width, resized.height), (MIN_SIZE, MIN_SIZE));
        let resized = resize(&origin, Handle::Left, 100., 0., ResizeMode::default());
        assert_eq!(resized.x + resized.width, 20., "the right edge stays");
    }

    #[test]
    fn shift_keeps_the_ratio_and_alt_resizes_from_the_center() {
        let origin = frame(0., 0., 200., 100.);
        let ratio = ResizeMode {
            keep_ratio: true,
            from_center: false,
        };
        let resized = resize(&origin, Handle::BottomRight, 100., 10., ratio);
        assert_eq!(resized, frame(0., 0., 300., 150.));
        let resized = resize(&origin, Handle::Right, 100., 0., ratio);
        assert_eq!(
            resized,
            frame(0., -25., 300., 150.),
            "grows around the middle"
        );

        let center = ResizeMode {
            keep_ratio: false,
            from_center: true,
        };
        let resized = resize(&origin, Handle::Right, 10., 0., center);
        assert_eq!(resized, frame(-10., 0., 220., 100.));
    }

    #[test]
    fn moves_snap_to_the_slide_center_and_other_elements() {
        let mut targets = Targets::new(1600., 900.);
        targets.add_frame(&frame(100., 500., 300., 100.));
        // The center of a 200-wide box 3 units off the slide center.
        let (snapped, guides) = snap_move(&frame(703., 200., 200., 50.), None, &targets, 6.);
        assert_eq!(snapped.x, 700.);
        assert_eq!(guides, [Guide::Vertical(800.)]);
        // The top 4 units below the other element's bottom edge.
        let (snapped, guides) = snap_move(&frame(1000., 604., 50., 50.), None, &targets, 6.);
        assert_eq!(snapped.y, 600.);
        assert_eq!(guides, [Guide::Horizontal(600.)]);
        let (unsnapped, guides) = snap_move(&frame(1000., 620., 50., 50.), None, &targets, 6.);
        assert_eq!(unsnapped.y, 620.);
        assert!(guides.is_empty());
    }

    #[test]
    fn text_snaps_its_first_baseline_to_other_baselines() {
        let mut targets = Targets::new(1600., 900.);
        targets.baselines.push(330.);
        let (snapped, guides) = snap_move(&frame(1000., 300., 50., 50.), Some(28.), &targets, 6.);
        assert_eq!(snapped.y, 302.);
        assert_eq!(guides, [Guide::Horizontal(330.)]);
    }

    #[test]
    fn resize_snaps_only_the_dragged_edges() {
        let targets = Targets::new(1600., 900.);
        let (snapped, guides) =
            snap_resize(&frame(4., 4., 1593., 50.), Handle::Right, &targets, 6.);
        assert_eq!(snapped, frame(4., 4., 1596., 50.));
        assert_eq!(guides, [Guide::Vertical(1600.)]);
    }
}
