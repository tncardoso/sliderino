//! Palette of the editor chrome and the kit theme built from it.
//!
//! Colors come from the "Sliderino — Editor" artboard in Paper. Components of
//! the kit read `assets/themes/light.json`; elements drawn by Sliderino itself
//! read the constants below.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::{App, Hsla, rgb};

const LIGHT_THEME: &str = include_str!("../assets/themes/light.json");

pub fn background() -> Hsla {
    rgb(0xFFFFFF).into()
}

pub fn border() -> Hsla {
    rgb(0xE6E6E6).into()
}

pub fn text() -> Hsla {
    rgb(0x000000).into()
}

pub fn text_muted() -> Hsla {
    rgb(0x6B6B6B).into()
}

pub fn text_faint() -> Hsla {
    rgb(0xB5B5B5).into()
}

pub fn field() -> Hsla {
    rgb(0xF5F5F5).into()
}

pub fn canvas() -> Hsla {
    rgb(0xEEEEEC).into()
}

pub fn surface() -> Hsla {
    rgb(0xF4F4F2).into()
}

pub fn ink() -> Hsla {
    rgb(0x111111).into()
}

pub fn accent() -> Hsla {
    rgb(0x1F4BFF).into()
}

pub fn accent_soft() -> Hsla {
    rgb(0xF3F5FF).into()
}

pub fn ok() -> Hsla {
    rgb(0x1E8A4C).into()
}

pub fn ok_soft() -> Hsla {
    rgb(0xE8F5EC).into()
}

pub fn warn() -> Hsla {
    rgb(0xB26A00).into()
}

pub fn warn_soft() -> Hsla {
    rgb(0xFFF3E0).into()
}

pub fn placeholder_line() -> Hsla {
    rgb(0xD0D0D0).into()
}

/// Applies the Sliderino light theme to the kit. Call after `gpui_kit::init`.
pub fn apply(cx: &mut App) {
    let config: ThemeConfig =
        serde_json::from_str(LIGHT_THEME).expect("assets/themes/light.json is a valid theme");
    Theme::global_mut(cx).apply_config(&Rc::new(config));
    Theme::sync_base(cx);
}
