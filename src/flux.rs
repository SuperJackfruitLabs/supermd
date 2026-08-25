//! Flux: time-of-day theme adaptation. Sunrise/sunset from the NOAA
//! solar equations (offline, no permissions), a day↔night blend with
//! smooth transitions, and color-temperature warming for themes.

use crate::theme::Theme;
use gpui::Hsla;

/// Fallback day window when no coordinates are configured or the sun
/// never rises/sets (polar day and night): 7:00–19:00 local.
pub const DEFAULT_SUNRISE_MIN: f64 = 7.0 * 60.0;
pub const DEFAULT_SUNSET_MIN: f64 = 19.0 * 60.0;

/// Sunrise and sunset in minutes after UTC midnight for a day of the
/// year (1-based). None when the sun never crosses the horizon.
/// Longitude is positive east.
pub fn sun_times_utc(lat_deg: f64, lon_deg: f64, day_of_year: u32) -> Option<(f64, f64)> {
    // NOAA general solar position calculations (fractional-year form,
    // accurate to a couple of minutes — plenty for a theme fade).
    let gamma = 2.0 * std::f64::consts::PI / 365.0 * (day_of_year as f64 - 1.0 + 0.5);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * gamma.cos()
            - 0.032077 * gamma.sin()
            - 0.014615 * (2.0 * gamma).cos()
            - 0.040849 * (2.0 * gamma).sin());
    let decl = 0.006918 - 0.399912 * gamma.cos() + 0.070257 * gamma.sin()
        - 0.006758 * (2.0 * gamma).cos()
        + 0.000907 * (2.0 * gamma).sin()
        - 0.002697 * (3.0 * gamma).cos()
        + 0.00148 * (3.0 * gamma).sin();

    let lat = lat_deg.to_radians();
    // Zenith 90.833°: the sun's radius plus atmospheric refraction.
    let cos_ha = (90.833f64.to_radians().cos() - lat.sin() * decl.sin()) / (lat.cos() * decl.cos());
    if !(-1.0..=1.0).contains(&cos_ha) {
        return None; // polar day or polar night
    }
    let ha_deg = cos_ha.acos().to_degrees();
    let sunrise = 720.0 - 4.0 * (lon_deg + ha_deg) - eqtime;
    let sunset = 720.0 - 4.0 * (lon_deg - ha_deg) - eqtime;
    Some((sunrise, sunset))
}

/// Local-time sunrise/sunset: UTC result shifted by the timezone
/// offset, wrapped into 0..1440.
pub fn sun_times_local(
    lat_deg: f64,
    lon_deg: f64,
    day_of_year: u32,
    offset_min: f64,
) -> Option<(f64, f64)> {
    let (rise, set) = sun_times_utc(lat_deg, lon_deg, day_of_year)?;
    Some(((rise + offset_min).rem_euclid(1440.0), (set + offset_min).rem_euclid(1440.0)))
}

/// 0.0 in full day, 1.0 in full night, ramping linearly across a
/// `transition_min`-wide window centered on each sun event.
pub fn night_blend(now_min: f64, sunrise_min: f64, sunset_min: f64, transition_min: f64) -> f32 {
    let half = transition_min / 2.0;
    let blend = if (sunrise_min - half..=sunrise_min + half).contains(&now_min) {
        0.5 - (now_min - sunrise_min) / transition_min
    } else if (sunset_min - half..=sunset_min + half).contains(&now_min) {
        0.5 + (now_min - sunset_min) / transition_min
    } else if now_min > sunrise_min && now_min < sunset_min {
        0.0
    } else {
        1.0
    };
    blend.clamp(0.0, 1.0) as f32
}

/// RGB channel multipliers for a color temperature, normalized so
/// 6500 K is the identity. Warmer (lower kelvin) trims green and blue.
pub fn kelvin_multipliers(kelvin: f64) -> (f32, f32, f32) {
    // Tanner Helland's kelvin→RGB fit, for the ≤6600 K branch we use.
    fn raw(kelvin: f64) -> (f64, f64, f64) {
        let t = (kelvin / 100.0).clamp(10.0, 66.0);
        let g = (99.4708025861 * t.ln() - 161.1195681661).clamp(0.0, 255.0);
        let b = if t <= 19.0 {
            0.0
        } else {
            (138.5177312231 * (t - 10.0).ln() - 305.0447927307).clamp(0.0, 255.0)
        };
        (255.0, g, b)
    }
    let (r0, g0, b0) = raw(6500.0);
    let (r, g, b) = raw(kelvin);
    (
        (r / r0).min(1.0) as f32,
        (g / g0).min(1.0) as f32,
        (b / b0).min(1.0) as f32,
    )
}

/// Blend a color toward its warmed version. `blend` 0 is a no-op.
pub fn warm(color: Hsla, blend: f32, mult: (f32, f32, f32)) -> Hsla {
    if blend <= 0.0 {
        return color;
    }
    let rgba = gpui::Rgba::from(color);
    let lerp = |channel: f32, m: f32| channel * (1.0 - blend + blend * m);
    gpui::Rgba {
        r: lerp(rgba.r, mult.0),
        g: lerp(rgba.g, mult.1),
        b: lerp(rgba.b, mult.2),
        a: rgba.a,
    }
    .into()
}

/// Warm every color of a theme by `blend` toward `kelvin`.
pub fn warm_theme(theme: &Theme, blend: f32, kelvin: f64) -> Theme {
    let mult = kelvin_multipliers(kelvin);
    theme.map_colors(|color| warm(color, blend, mult))
}

/// Blend for the wall clock right now: 0 while flux is disabled, the
/// configured coordinates' sun window otherwise (fixed 7:00–19:00
/// without coordinates or under a polar sun).
pub fn current_blend(flux: &crate::settings::FluxSettings) -> f32 {
    if !flux.enabled {
        return 0.0;
    }
    use chrono::{Datelike, Offset, Timelike};
    let now = chrono::Local::now();
    let minutes = now.hour() as f64 * 60.0 + now.minute() as f64;
    let window = match (flux.latitude, flux.longitude) {
        (Some(lat), Some(lon)) => {
            let offset = now.offset().fix().local_minus_utc() as f64 / 60.0;
            sun_times_local(lat, lon, now.ordinal(), offset)
        }
        _ => None,
    };
    let (sunrise, sunset) = window.unwrap_or((DEFAULT_SUNRISE_MIN, DEFAULT_SUNSET_MIN));
    night_blend(minutes, sunrise, sunset, flux.transition_minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── solar math ──

    #[test]
    fn equator_equinox_is_a_twelve_hour_day() {
        // Mar 20 (day 79) at (0, 0): sunrise ≈ 06:04 UTC, sunset ≈
        // 18:11 UTC — refraction makes the day slightly longer than 12h.
        let (rise, set) = sun_times_utc(0.0, 0.0, 79).unwrap();
        assert!((rise - 364.0).abs() < 12.0, "sunrise {rise}");
        assert!((set - 1091.0).abs() < 12.0, "sunset {set}");
        let day = set - rise;
        assert!((day - 727.0).abs() < 10.0, "day length {day}");
    }

    #[test]
    fn london_summer_solstice_matches_published_times() {
        // Jun 21 (day 172) at 51.51 N: sunrise 03:43 UTC, sunset 20:21 UTC.
        let (rise, set) = sun_times_utc(51.5074, -0.1278, 172).unwrap();
        assert!((rise - 223.0).abs() < 12.0, "sunrise {rise}");
        assert!((set - 1221.0).abs() < 12.0, "sunset {set}");
    }

    #[test]
    fn polar_day_and_night_return_none() {
        assert_eq!(sun_times_utc(69.65, 18.96, 172), None, "midnight sun");
        assert_eq!(sun_times_utc(69.65, 18.96, 355), None, "polar night");
        assert!(sun_times_utc(69.65, 18.96, 79).is_some(), "equinox is normal");
    }

    #[test]
    fn moving_west_delays_utc_times() {
        let (rise0, set0) = sun_times_utc(0.0, 0.0, 79).unwrap();
        let (rise_w, set_w) = sun_times_utc(0.0, -15.0, 79).unwrap();
        assert!((rise_w - rise0 - 60.0).abs() < 2.0);
        assert!((set_w - set0 - 60.0).abs() < 2.0);
    }

    #[test]
    fn local_times_apply_the_offset_and_wrap() {
        // Equator at 77.6 E with IST (+330): sunrise ≈ 06:04 UTC-at-
        // longitude → local morning.
        let (rise_utc, _) = sun_times_utc(0.0, 77.59, 79).unwrap();
        let (rise_local, set_local) = sun_times_local(0.0, 77.59, 79, 330.0).unwrap();
        assert!((rise_local - (rise_utc + 330.0)).abs() < 0.01);
        assert!((0.0..1440.0).contains(&rise_local));
        assert!((0.0..1440.0).contains(&set_local));
    }

    // ── blend ──

    #[test]
    fn blend_truth_table() {
        let b = |now: f64| night_blend(now, 360.0, 1080.0, 40.0);
        assert_eq!(b(720.0), 0.0, "noon");
        assert_eq!(b(0.0), 1.0, "midnight");
        assert_eq!(b(1439.0), 1.0, "just before midnight");
        assert_eq!(b(360.0), 0.5, "sunrise midpoint");
        assert_eq!(b(340.0), 1.0, "ramp start");
        assert_eq!(b(380.0), 0.0, "ramp end");
        assert_eq!(b(350.0), 0.75, "quarter into sunrise ramp");
        assert_eq!(b(1080.0), 0.5, "sunset midpoint");
        assert_eq!(b(1100.0), 1.0, "night after sunset ramp");
        assert_eq!(b(1070.0), 0.25);
    }

    // ── warmth ──

    #[test]
    fn six_thousand_five_hundred_kelvin_is_identity() {
        let (r, g, b) = kelvin_multipliers(6500.0);
        assert!((r - 1.0).abs() < 0.01 && (g - 1.0).abs() < 0.01 && (b - 1.0).abs() < 0.01);
    }

    #[test]
    fn lower_kelvin_trims_blue_hardest() {
        let (r, g, b) = kelvin_multipliers(3400.0);
        assert_eq!(r, 1.0);
        assert!((0.68..0.82).contains(&g), "g {g}");
        assert!((0.45..0.62).contains(&b), "b {b}");
        let (_, _, b_warmer) = kelvin_multipliers(2700.0);
        assert!(b_warmer < b, "warmer means less blue");
    }

    #[test]
    fn current_blend_is_zero_while_disabled() {
        let flux = crate::settings::FluxSettings::default();
        assert_eq!(current_blend(&flux), 0.0);
    }

    #[test]
    fn warm_at_zero_blend_is_untouched() {
        let color = gpui::rgb(0x89a1c4).into();
        assert_eq!(warm(color, 0.0, kelvin_multipliers(3400.0)), color);
    }

    #[test]
    fn warm_theme_preserves_structure_and_reduces_blue() {
        let theme = Theme::light();
        let same = warm_theme(&theme, 0.0, 3400.0);
        assert_eq!(same.bg, theme.bg);
        assert_eq!(same.syntax.keyword, theme.syntax.keyword);

        let night = warm_theme(&theme, 1.0, 3400.0);
        let before = gpui::Rgba::from(theme.bg);
        let after = gpui::Rgba::from(night.bg);
        assert!(after.b < before.b, "blue dropped: {} → {}", before.b, after.b);
        assert!((after.r - before.r).abs() < 0.02, "red held");
        assert_eq!(night.is_dark, theme.is_dark);
        assert_eq!(night.body_family, theme.body_family);
        assert_eq!(night.body_size, theme.body_size);
    }
}
