//! Viewport camera of the canvas: how the slide maps onto the screen.
//!
//! Editor state only; it is not part of the saved document. The camera is
//! anchored at the viewport center, so resizing the window keeps the slide
//! where it was relative to the middle of the canvas.

use gpui_kit::{Bounds, Edges, Pixels, Point, Size, point, px, size};

use crate::document::SlideSize;

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Logical pixels per slide unit; 1.0 is 100%.
    pub zoom: f32,
    /// Offset of the slide center from the viewport center.
    pub pan: Point<Pixels>,
}

impl Camera {
    /// Largest zoom (never above 100%) that shows the whole slide inside the
    /// viewport minus `insets`, centered in that inset area.
    pub fn fit(viewport: Size<Pixels>, slide: SlideSize, insets: Edges<Pixels>) -> Self {
        let available_w = (viewport.width - insets.left - insets.right).max(px(1.));
        let available_h = (viewport.height - insets.top - insets.bottom).max(px(1.));
        let zoom = (f32::from(available_w) / slide.width as f32)
            .min(f32::from(available_h) / slide.height as f32)
            .min(1.)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Self {
            zoom,
            pan: point(
                (insets.left - insets.right) / 2.,
                (insets.top - insets.bottom) / 2.,
            ),
        }
    }

    /// Multiplies the zoom by `factor`, keeping the slide point under
    /// `anchor` (an offset from the viewport center) in place.
    pub fn zoom_at(&mut self, anchor: Point<Pixels>, factor: f32) {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.zoom;
        self.pan = anchor - (anchor - self.pan) * ratio;
        self.zoom = zoom;
    }

    /// Sets the zoom around the slide center, leaving the pan untouched.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn pan_by(&mut self, delta: Point<Pixels>) {
        self.pan += delta;
    }

    /// Displayed size of the slide.
    pub fn slide_size(&self, slide: SlideSize) -> Size<Pixels> {
        size(
            px(slide.width as f32 * self.zoom),
            px(slide.height as f32 * self.zoom),
        )
    }

    /// Where the slide sits on screen, given the viewport bounds.
    pub fn slide_rect(&self, slide: SlideSize, viewport: Bounds<Pixels>) -> Bounds<Pixels> {
        let size = self.slide_size(slide);
        let center = viewport.center() + self.pan;
        Bounds {
            origin: point(center.x - size.width / 2., center.y - size.height / 2.),
            size,
        }
    }

    /// Zoom as shown in the UI, such as "63%".
    pub fn label(&self) -> String {
        format!("{:.0}%", self.zoom * 100.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDE: SlideSize = SlideSize {
        width: 1600,
        height: 900,
    };

    fn close(a: Pixels, b: Pixels) -> bool {
        (f32::from(a) - f32::from(b)).abs() < 0.01
    }

    fn viewport() -> Bounds<Pixels> {
        Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(1000.), px(700.)),
        }
    }

    #[test]
    fn fit_shows_the_whole_slide_centered_in_the_insets() {
        let insets = Edges {
            top: px(40.),
            right: px(50.),
            bottom: px(100.),
            left: px(50.),
        };
        let camera = Camera::fit(viewport().size, SLIDE, insets);
        assert!(
            (camera.zoom - 0.5625).abs() < 1e-6,
            "width limits: 900/1600"
        );
        let rect = camera.slide_rect(SLIDE, viewport());
        assert!(close(rect.origin.x, px(150.)));
        assert!(close(rect.right(), px(1050.)));
        assert!(rect.origin.y >= px(90.) && rect.bottom() <= px(650.));
        assert!(close(rect.origin.y - px(90.), px(650.) - rect.bottom()));
    }

    #[test]
    fn fit_never_exceeds_100_percent() {
        let camera = Camera::fit(size(px(4000.), px(3000.)), SLIDE, Edges::default());
        assert_eq!(camera.zoom, 1.);
        assert_eq!(camera.label(), "100%");
    }

    #[test]
    fn zoom_at_keeps_the_anchored_point_still() {
        let mut camera = Camera {
            zoom: 0.5,
            pan: point(px(30.), px(-20.)),
        };
        let anchor = point(px(-120.), px(80.));
        let slide_point = (anchor - camera.pan) / camera.zoom;
        camera.zoom_at(anchor, 1.1);
        assert!((camera.zoom - 0.55).abs() < 1e-6);
        let after = camera.pan + slide_point * camera.zoom;
        assert!(close(after.x, anchor.x) && close(after.y, anchor.y));
    }

    #[test]
    fn zoom_is_clamped() {
        let mut camera = Camera {
            zoom: 1.,
            pan: point(px(0.), px(0.)),
        };
        for _ in 0..100 {
            camera.zoom_at(point(px(10.), px(10.)), 1.1);
        }
        assert_eq!(camera.zoom, MAX_ZOOM);
        for _ in 0..200 {
            camera.zoom_at(point(px(10.), px(10.)), 1. / 1.1);
        }
        assert_eq!(camera.zoom, MIN_ZOOM);
        camera.set_zoom(20.);
        assert_eq!(camera.zoom, MAX_ZOOM);
    }
}
