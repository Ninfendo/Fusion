//! Maps Fusion's colour tokens onto egui.
//!
//! Standard egui widgets take their colours from [`visuals`]. Fusion's own
//! widgets (grids, the ladder, flashes) read the tokens directly through
//! [`color`].

use egui::{Color32, Context, Visuals};
use fusion_theme::{Mode, Rgba, Theme};

/// A token colour as an egui colour.
pub fn color(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// egui's light or dark theme for a mode.
pub fn egui_theme(mode: Mode) -> egui::Theme {
    match mode {
        Mode::Dark => egui::Theme::Dark,
        Mode::Light => egui::Theme::Light,
    }
}

/// egui visuals built from the tokens.
pub fn visuals(theme: &Theme) -> Visuals {
    let mut v = match theme.mode {
        Mode::Dark => Visuals::dark(),
        Mode::Light => Visuals::light(),
    };
    v.panel_fill = color(theme.background);
    v.window_fill = color(theme.panel);
    v.extreme_bg_color = color(theme.panel);
    v.faint_bg_color = color(theme.panel_alt);
    v.code_bg_color = color(theme.panel_alt);
    v.override_text_color = Some(color(theme.text));
    v.hyperlink_color = color(theme.accent);
    v.warn_fg_color = color(theme.warning);
    v.error_fg_color = color(theme.error);
    v.selection.bg_fill = color(theme.selected_row);
    v.selection.stroke.color = color(theme.accent);
    v.window_stroke.color = color(theme.border);
    v.widgets.noninteractive.bg_stroke.color = color(theme.border);
    v
}

/// Makes `theme` the active egui theme.
pub fn apply(ctx: &Context, theme: &Theme) {
    let which = egui_theme(theme.mode);
    ctx.set_visuals_of(which, visuals(theme));
    ctx.set_theme(which);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_keep_their_channels() {
        let c = color(Rgba {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        });
        assert_eq!(c, Color32::from_rgb(10, 20, 30));
    }

    #[test]
    fn visuals_carry_the_surface_tokens() {
        for theme in [Theme::dark(), Theme::light()] {
            let v = visuals(&theme);
            assert_eq!(v.dark_mode, theme.mode == Mode::Dark);
            assert_eq!(v.panel_fill, color(theme.background));
            assert_eq!(v.window_fill, color(theme.panel));
            assert_eq!(v.extreme_bg_color, color(theme.panel));
            assert_eq!(v.faint_bg_color, color(theme.panel_alt));
            assert_eq!(v.code_bg_color, color(theme.panel_alt));
            assert_eq!(v.window_stroke.color, color(theme.border));
            let lines = v.widgets.noninteractive.bg_stroke.color;
            assert_eq!(lines, color(theme.border));
        }
    }

    #[test]
    fn visuals_carry_the_text_and_accent_tokens() {
        for theme in [Theme::dark(), Theme::light()] {
            let v = visuals(&theme);
            assert_eq!(v.override_text_color, Some(color(theme.text)));
            assert_eq!(v.hyperlink_color, color(theme.accent));
            assert_eq!(v.warn_fg_color, color(theme.warning));
            assert_eq!(v.error_fg_color, color(theme.error));
            assert_eq!(v.selection.bg_fill, color(theme.selected_row));
            assert_eq!(v.selection.stroke.color, color(theme.accent));
        }
    }

    #[test]
    fn modes_map_to_egui_themes() {
        assert_eq!(egui_theme(Mode::Dark), egui::Theme::Dark);
        assert_eq!(egui_theme(Mode::Light), egui::Theme::Light);
    }

    #[test]
    fn apply_switches_the_context() {
        let ctx = Context::default();
        apply(&ctx, &Theme::light());
        assert_eq!(ctx.theme(), egui::Theme::Light);
        apply(&ctx, &Theme::dark());
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            color(Theme::dark().background)
        );
    }
}
