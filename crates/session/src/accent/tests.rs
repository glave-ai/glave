use proptest::prelude::*;

use super::*;

/// Every promise a tuned accent makes, as Glaze's tokens draw it.
fn assert_reads(color: Color) {
    let drawn = shades(color, true);
    for (mode, shade, on_white) in [
        (Mode::Light, drawn.light, true),
        (Mode::Dark, drawn.dark, false),
    ] {
        assert!(
            mode.reads(shade),
            "{color} gives {shade}, which doesn't read"
        );
        // `--glz-on-accent` is white below 0.65 lightness and black above: the text promised.
        assert_eq!(
            Lch::of(shade).lightness < 0.65,
            on_white,
            "{color}: {shade}"
        );
    }
}

#[test]
fn every_hue_reads_in_light_and_dark() {
    for hue in 0..360 {
        for lightness in [0.05, 0.25, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95] {
            for chroma in [0.02, 0.08, 0.15, 0.25, 0.37] {
                let color = Lch {
                    lightness,
                    chroma,
                    hue: f64::from(hue).to_radians(),
                }
                .color();
                assert_reads(color);
            }
        }
    }
}

proptest! {
    #[test]
    fn any_colour_reads_in_light_and_dark(red: u8, green: u8, blue: u8) {
        assert_reads(Color::rgb(red, green, blue));
    }

    #[test]
    fn a_colour_comes_back_from_oklch_within_a_step(red: u8, green: u8, blue: u8) {
        let color = Color::rgb(red, green, blue);
        let back = Lch::of(color).color();
        for (one, other) in color.channels().into_iter().zip(back.channels()) {
            prop_assert!(one.abs_diff(other) <= 1, "{color} came back {back}");
        }
    }
}

#[test]
fn not_tuned_it_is_the_exact_colour_in_both() {
    let yellow = Color::new("#f2c200").unwrap();
    let exact = shades(yellow, false);
    assert_eq!(exact.light, yellow);
    assert_eq!(exact.dark, yellow);
}

#[test]
fn a_colour_that_reads_stays_exactly_itself() {
    let ink = Color::new("#1b1c1d").unwrap();
    assert_eq!(shades(ink, true).light, ink);
    let paper = Color::new("#ececec").unwrap();
    assert_eq!(shades(paper, true).dark, paper);
}

#[test]
fn a_bright_colour_keeps_its_hue_where_it_has_to_move() {
    let yellow = Color::new("#f2c200").unwrap();
    let tuned = shades(yellow, true);
    assert_ne!(tuned.light, yellow);
    let (from, to) = (Lch::of(yellow).hue, Lch::of(tuned.light).hue);
    assert!((from - to).abs() < 0.1, "{from} became {to}");
}

/// What the promises above rest on, as Glaze's tokens say it: if one changes, so must this.
#[test]
fn the_colours_it_reads_against_are_glazes() {
    let tokens = include_str!("../../../../web/glaze/styles/tokens.css");
    for token in [
        "--glz-surface-selected: light-dark(#e4e4e4, #25272a);",
        "--glz-on-accent: oklch(from var(--glz-accent) clamp(0, (0.65 - l) * 999, 1) 0 none);",
        "--glz-accent-hover: color-mix(in oklch, var(--glz-accent), var(--glz-on-accent) 14%);",
        "oklch(from var(--glz-accent) min(l, 0.5) c h),",
        "oklch(from var(--glz-accent) max(l, 0.78) c h)",
    ] {
        assert!(tokens.contains(token), "tokens.css no longer says {token}");
    }
}
