use super::*;

/// WCAG 2.2: normal text needs 4.5:1, large text and UI parts 3:1.
const TEXT: f64 = 4.5;
const UI_PART: f64 = 3.0;

fn every_theme() -> Vec<Theme> {
    let mut themes = Vec::new();
    for mode in [Mode::Dark, Mode::Light] {
        for colour_blind in [false, true] {
            themes.push(Theme::new(mode, colour_blind));
        }
    }
    themes
}

fn assert_contrast(theme: &Theme, name: &str, fg: Rgba, bg: Rgba, min: f64) {
    let ratio = fg.contrast_ratio(bg);
    assert!(
        ratio >= min,
        "{:?} colour-blind={}: {name} is {ratio:.2}:1, needs {min}:1",
        theme.mode,
        theme.colour_blind
    );
}

#[test]
fn contrast_follows_the_known_formula() {
    let black = Rgba::rgb(0, 0, 0);
    let white = Rgba::rgb(255, 255, 255);
    assert!((black.contrast_ratio(white) - 21.0).abs() < 1e-9);
    assert!((white.contrast_ratio(black) - 21.0).abs() < 1e-9);
    assert!((white.contrast_ratio(white) - 1.0).abs() < 1e-9);
    // A mid grey: #777777 on white is about 4.48:1, just under the text limit.
    let grey = Rgba::rgb(0x77, 0x77, 0x77);
    assert!((grey.contrast_ratio(white) - 4.48).abs() < 0.01);
}

#[test]
fn luminance_uses_the_srgb_curve() {
    // Below the knee the curve is linear: 10/255 / 12.92.
    let dark = Rgba::rgb(10, 10, 10);
    assert!((dark.relative_luminance() - (10.0 / 255.0 / 12.92)).abs() < 1e-12);
    assert!((Rgba::rgb(255, 255, 255).relative_luminance() - 1.0).abs() < 1e-12);
    assert!(Rgba::rgb(0, 255, 0).relative_luminance() > Rgba::rgb(255, 0, 0).relative_luminance());
}

#[test]
fn text_is_readable_everywhere_it_sits() {
    for t in every_theme() {
        for (bg_name, bg) in [
            ("background", t.background),
            ("panel", t.panel),
            ("panel_alt", t.panel_alt),
            ("selected_row", t.selected_row),
        ] {
            assert_contrast(&t, &format!("text on {bg_name}"), t.text, bg, TEXT);
            assert_contrast(
                &t,
                &format!("muted text on {bg_name}"),
                t.text_muted,
                bg,
                TEXT,
            );
        }
    }
}

#[test]
fn buy_and_sell_are_readable_on_panels_and_on_tick_flashes() {
    for t in every_theme() {
        for (bg_name, bg) in [
            ("panel", t.panel),
            ("panel_alt", t.panel_alt),
            ("up_tick", t.up_tick),
            ("down_tick", t.down_tick),
        ] {
            assert_contrast(&t, &format!("buy on {bg_name}"), t.buy, bg, TEXT);
            assert_contrast(&t, &format!("sell on {bg_name}"), t.sell, bg, TEXT);
            assert_contrast(&t, &format!("text on {bg_name}"), t.text, bg, TEXT);
        }
    }
}

#[test]
fn warnings_errors_and_the_accent_stand_out() {
    for t in every_theme() {
        assert_contrast(&t, "warning on panel", t.warning, t.panel, TEXT);
        assert_contrast(&t, "error on panel", t.error, t.panel, TEXT);
        assert_contrast(&t, "accent on panel", t.accent, t.panel, UI_PART);
    }
}

#[test]
fn the_colour_blind_option_changes_only_buy_and_sell_colours() {
    for mode in [Mode::Dark, Mode::Light] {
        let plain = Theme::new(mode, false);
        let blind = Theme::new(mode, true);
        assert!(blind.colour_blind);
        assert_ne!(plain.buy, blind.buy);
        assert_ne!(plain.sell, blind.sell);
        assert_ne!(plain.up_tick, blind.up_tick);
        assert_ne!(plain.down_tick, blind.down_tick);
        let same_rest = Theme {
            colour_blind: false,
            buy: plain.buy,
            sell: plain.sell,
            up_tick: plain.up_tick,
            down_tick: plain.down_tick,
            ..blind
        };
        assert_eq!(same_rest, plain);
    }
}

#[test]
fn the_constructors_set_the_mode() {
    assert_eq!(Theme::new(Mode::Dark, false), Theme::dark());
    assert_eq!(Theme::new(Mode::Light, false), Theme::light());
    assert_eq!(Theme::dark().mode, Mode::Dark);
    assert_eq!(Theme::light().mode, Mode::Light);
    assert!(
        Theme::dark().text.relative_luminance() > Theme::dark().background.relative_luminance()
    );
    assert!(
        Theme::light().text.relative_luminance() < Theme::light().background.relative_luminance()
    );
}
