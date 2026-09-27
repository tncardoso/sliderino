//! Conversions from Sliderino units to the units of DrawingML.
//!
//! A slide unit is 1/120 inch: the 1600 × 900 slide is the 13.333 × 7.5
//! inch (16:9) slide of PowerPoint.

/// English Metric Units in one slide unit (914 400 EMU is one inch).
pub const EMU_PER_UNIT: f64 = 7620.;

/// Points in one slide unit.
pub const POINTS_PER_UNIT: f64 = 0.6;

/// Slide units to EMU, rounded.
pub fn emu(units: f32) -> i64 {
    (units as f64 * EMU_PER_UNIT).round() as i64
}

/// EMU to slide units.
pub fn units(emu: i64) -> f32 {
    (emu as f64 / EMU_PER_UNIT) as f32
}

/// Slide units to hundredths of a point (font sizes, character spacing,
/// exact line spacing).
pub fn centipoints(units: f32) -> i64 {
    (units as f64 * POINTS_PER_UNIT * 100.).round() as i64
}

/// Hundredths of a point to slide units.
pub fn from_centipoints(value: i64) -> f32 {
    (value as f64 / 100. / POINTS_PER_UNIT) as f32
}

/// Degrees to 60 000ths of a degree, in [0, 360°): DrawingML rotations and
/// gradient angles.
pub fn angle(degrees: f32) -> i64 {
    let value = (degrees as f64 * 60_000.).round() as i64;
    value.rem_euclid(21_600_000)
}

/// 60 000ths of a degree to degrees in (-180, 180], as Sliderino keeps them.
pub fn degrees(angle: i64) -> f32 {
    crate::document::normalize_degrees((angle as f64 / 60_000.) as f32)
}

/// A fraction 0..1 to thousandths of a percent (alpha, gradient positions,
/// rectangles in fractions of a box).
pub fn percent(fraction: f32) -> i64 {
    (fraction as f64 * 100_000.).round() as i64
}

/// Thousandths of a percent to a fraction.
pub fn fraction(value: i64) -> f32 {
    (value as f64 / 100_000.) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_slide_is_the_widescreen_slide() {
        assert_eq!(emu(1600.), 12_192_000);
        assert_eq!(emu(900.), 6_858_000);
    }

    #[test]
    fn a_32_unit_font_is_19_2_points() {
        assert_eq!(centipoints(32.), 1920);
        assert_eq!(from_centipoints(1920), 32.);
    }

    #[test]
    fn angles_wrap_into_one_turn() {
        assert_eq!(angle(-90.), 16_200_000);
        assert_eq!(angle(360.), 0);
        assert_eq!(degrees(16_200_000), -90.);
        assert_eq!(degrees(10_800_000), 180.);
    }

    #[test]
    fn round_trips_within_one_emu() {
        for value in [0., 0.1, 13.37, 1599.99] {
            assert!((units(emu(value)) - value).abs() < 1. / 7620.);
        }
    }
}
