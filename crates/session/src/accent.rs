//! What an accent is drawn as, in light and dark (`specs/model.md`, "The look"). Tuned, each value
//! is the chosen colour within the chroma the approved look allows, at the lightness nearest it
//! where text reads at WCAG AA, on the accent and in the accent on every surface, in the colours
//! Glaze's tokens draw them with. Not tuned, each is the colour itself.

use glave_event::Color;

use crate::protocol::Shades;

/// WCAG AA, for text.
const LEAST_CONTRAST: f64 = 4.5;
/// The darkest surface in light and the lightest in dark that the accent is text on
/// (`--glz-surface-selected`).
const SURFACE_LIGHT: Color = Color::rgb(0xe4, 0xe4, 0xe4);
const SURFACE_DARK: Color = Color::rgb(0x25, 0x27, 0x2a);
/// What text on the accent is drawn in (`--glz-on-accent`).
const ON_LIGHT: Color = Color::rgb(0xff, 0xff, 0xff);
const ON_DARK: Color = Color::rgb(0, 0, 0);
/// How far the accent moves towards the text on it when under the pointer
/// (`--glz-accent-hover`).
const HOVER: f64 = 0.14;
/// The lightest the accent is in light and the darkest in dark, as `--glz-accent-text` keeps it,
/// and the most colourful each is, which the owner approved in the mockup.
const LIGHTNESS_LIGHT: f64 = 0.5;
const LIGHTNESS_DARK: f64 = 0.78;
const CHROMA_LIGHT: f64 = 0.17;
const CHROMA_DARK: f64 = 0.13;
/// Each try moves this much lightness on.
const STEP: f64 = 0.01;

/// What `color` is drawn as: tuned where `adjust` says, else exactly it.
pub(crate) fn shades(color: Color, adjust: bool) -> Shades {
    if adjust {
        Shades {
            light: tuned(color, Mode::Light),
            dark: tuned(color, Mode::Dark),
        }
    } else {
        Shades {
            light: color,
            dark: color,
        }
    }
}

/// Light or dark, and what the accent must read against in each.
#[derive(Clone, Copy)]
enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// The text on the accent.
    const fn on(self) -> Color {
        match self {
            Self::Light => ON_LIGHT,
            Self::Dark => ON_DARK,
        }
    }

    /// The surface the accent is text on that it stands out from least.
    const fn surface(self) -> Color {
        match self {
            Self::Light => SURFACE_LIGHT,
            Self::Dark => SURFACE_DARK,
        }
    }

    /// Whether text reads on `shade`, under the pointer too, and `shade` reads as text.
    fn reads(self, shade: Color) -> bool {
        let lch = Lch::of(shade);
        let on = Lch::of(self.on());
        let hover = Lch {
            lightness: lch.lightness + (on.lightness - lch.lightness) * HOVER,
            chroma: lch.chroma * (1.0 - HOVER),
            hue: lch.hue,
        };
        contrast(shade, self.on()) >= LEAST_CONTRAST
            && contrast(shade, self.surface()) >= LEAST_CONTRAST
            && contrast(hover.color(), self.on()) >= LEAST_CONTRAST
    }

    /// `lch` within what this mode allows.
    fn within(self, lch: Lch) -> Lch {
        let (lightness, chroma) = match self {
            Self::Light => (lch.lightness.min(LIGHTNESS_LIGHT), CHROMA_LIGHT),
            Self::Dark => (lch.lightness.max(LIGHTNESS_DARK), CHROMA_DARK),
        };
        Lch {
            lightness,
            chroma: lch.chroma.min(chroma),
            hue: lch.hue,
        }
    }

    /// `lch` a try further from what the text on the accent is.
    fn further(self, lch: Lch) -> Lch {
        let step = match self {
            Self::Light => -STEP,
            Self::Dark => STEP,
        };
        Lch {
            lightness: (lch.lightness + step).clamp(0.0, 1.0),
            ..lch
        }
    }
}

/// `color` as `mode` draws it: itself, where it is within `mode` and reads as it is.
fn tuned(color: Color, mode: Mode) -> Color {
    let chosen = Lch::of(color);
    let mut lch = mode.within(chosen);
    if lch == chosen && mode.reads(color) {
        return color;
    }
    // From no lightness to full is a hundred steps, and black on white reads, as white on black.
    for _ in 0..=100 {
        let shade = lch.color();
        if mode.reads(shade) {
            return shade;
        }
        lch = mode.further(lch);
    }
    match mode {
        Mode::Light => ON_DARK,
        Mode::Dark => ON_LIGHT,
    }
}

/// How two colours stand apart, by WCAG's formula.
fn contrast(one: Color, other: Color) -> f64 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// A colour's relative luminance.
fn luminance(color: Color) -> f64 {
    let [red, green, blue] = color.channels().map(|channel| linear(f64::from(channel)));
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// An sRGB channel, 0 to 255, in linear light, 0 to 1.
fn linear(channel: f64) -> f64 {
    let channel = channel / 255.0;
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// A channel in linear light, as sRGB's 0 to 1.
fn gamma(channel: f64) -> f64 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// A colour in OKLCH, the space Glaze's tokens work in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lch {
    lightness: f64,
    chroma: f64,
    /// In radians.
    hue: f64,
}

impl Lch {
    /// `color`, by Björn Ottosson's matrices.
    fn of(color: Color) -> Self {
        let [red, green, blue] = color.channels().map(|channel| linear(f64::from(channel)));
        let long = (0.412_221_47 * red + 0.536_332_54 * green + 0.051_445_99 * blue).cbrt();
        let medium = (0.211_903_5 * red + 0.680_699_55 * green + 0.107_396_96 * blue).cbrt();
        let short = (0.088_302_46 * red + 0.281_718_84 * green + 0.629_978_7 * blue).cbrt();
        let reddish = 1.977_998_5 * long - 2.428_592_2 * medium + 0.450_593_7 * short;
        let yellowish = 0.025_904_04 * long + 0.782_771_77 * medium - 0.808_675_77 * short;
        Self {
            lightness: 0.210_454_26 * long + 0.793_617_8 * medium - 0.004_072_05 * short,
            chroma: reddish.hypot(yellowish),
            hue: yellowish.atan2(reddish),
        }
    }

    /// The nearest colour sRGB can show: its lightness and hue, its chroma lowered until it fits.
    fn color(self) -> Color {
        let [mut least, mut most] = [0.0, self.chroma];
        if let Some(fits) = self.rgb(most) {
            return fits;
        }
        for _ in 0..16 {
            let middle = f64::midpoint(least, most);
            if self.rgb(middle).is_some() {
                least = middle;
            } else {
                most = middle;
            }
        }
        self.rgb(least).unwrap_or_else(|| grey(self.lightness))
    }

    /// The colour at `chroma`, where sRGB can show it.
    fn rgb(self, chroma: f64) -> Option<Color> {
        let (reddish, yellowish) = (chroma * self.hue.cos(), chroma * self.hue.sin());
        let lightness = self.lightness;
        let long = (lightness + 0.396_337_78 * reddish + 0.215_803_76 * yellowish).powi(3);
        let medium = (lightness - 0.105_561_346 * reddish - 0.063_854_17 * yellowish).powi(3);
        let short = (lightness - 0.089_484_18 * reddish - 1.291_485_5 * yellowish).powi(3);
        let red = 4.076_741_7 * long - 3.307_711_6 * medium + 0.230_969_9 * short;
        let green = -1.268_438 * long + 2.609_757_4 * medium - 0.341_319_4 * short;
        let blue = -0.004_196_086 * long - 0.703_418_6 * medium + 1.707_614_7 * short;
        Some(Color::rgb(byte(red)?, byte(green)?, byte(blue)?))
    }
}

/// The grey of `lightness`.
fn grey(lightness: f64) -> Color {
    let level = byte(lightness.clamp(0.0, 1.0).powi(3)).unwrap_or(0);
    Color::rgb(level, level, level)
}

/// A channel in linear light as an sRGB byte, where it is within what sRGB shows.
fn byte(channel: f64) -> Option<u8> {
    const SLACK: f64 = 0.000_1;
    if !(-SLACK..=1.0 + SLACK).contains(&channel) {
        return None;
    }
    let level = (gamma(channel.clamp(0.0, 1.0)) * 255.0).round();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0 to 1 above, so it is a whole number from 0 to 255"
    )]
    Some(level as u8)
}

#[cfg(test)]
mod tests;
