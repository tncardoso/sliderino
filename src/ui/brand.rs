//! The Sliderino mark: an outlined slide behind a filled blue one. From the
//! "Sliderino — Home" artboard in Paper, drawn with two boxes instead of an
//! SVG.

use gpui_kit::{Div, Hsla, IntoElement, ParentElement, Styled, div, px};

use crate::theme;

/// One rectangle of the mark in the units of its SVG view box.
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
}

/// The two rectangles in the view box `origin` of size `view`, scaled by
/// `scale`. The outline is centered on the edge of its rectangle, as an SVG
/// stroke is.
fn mark_boxes(scale: f32, origin: (f32, f32), view: (f32, f32), stroke: f32, outline: Hsla) -> Div {
    let back = Rect {
        x: 21.,
        y: 14.,
        width: 35.,
        height: 23.,
        radius: 4.,
    };
    let front = Rect {
        x: 8.,
        y: 28.,
        width: 35.,
        height: 23.,
        radius: 4.,
    };
    let place = |rect: &Rect, grow: f32| {
        div()
            .absolute()
            .left(px((rect.x - grow - origin.0) * scale))
            .top(px((rect.y - grow - origin.1) * scale))
            .w(px((rect.width + 2. * grow) * scale))
            .h(px((rect.height + 2. * grow) * scale))
            .rounded(px((rect.radius + grow) * scale))
    };
    div()
        .relative()
        .flex_shrink_0()
        .w(px(view.0 * scale))
        .h(px(view.1 * scale))
        .child(
            place(&back, stroke / 2.)
                .border(px(stroke * scale))
                .border_color(outline),
        )
        .child(place(&front, 0.).bg(theme::accent()))
}

/// The large mark of the Home screen, `height` pixels high.
pub fn mark(height: f32) -> impl IntoElement {
    mark_boxes(height / 41., (7., 11.), (51., 41.), 4., theme::ink())
}

/// The app icon of the title bars: the mark on a dark 22px tile.
pub fn app_icon() -> impl IntoElement {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(6.))
        .bg(theme::ink())
        .child(mark_boxes(
            18. / 64.,
            (0., 0.),
            (64., 64.),
            6.,
            theme::background(),
        ))
}
