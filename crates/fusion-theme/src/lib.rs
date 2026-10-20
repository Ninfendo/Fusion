//! Fusion's colour tokens.
//!
//! A theme is a named set of colours with a purpose: background, text, buy,
//! sell and so on. Screens use the tokens, never raw colours, so a theme
//! change reaches everything. This crate has no UI library in it:
//! `fusion-egui-theme` maps the tokens to egui, and another UI library could
//! map the same tokens.
//!
//! The tests check every text colour against the colours it sits on, with
//! the WCAG contrast formula, so a theme cannot ship with unreadable text.

/// An sRGB colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha: 255 is opaque.
    pub a: u8,
}

impl Rgba {
    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Relative luminance, as WCAG 2.2 defines it: 0 for black, 1 for white.
    pub fn relative_luminance(self) -> f64 {
        fn linear(channel: u8) -> f64 {
            let c = f64::from(channel) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// The WCAG contrast ratio between two colours: 1 for equal colours, 21
    /// for black on white. Text needs at least 4.5.
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// Dark or light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Light text on a dark background.
    Dark,
    /// Dark text on a light background.
    Light,
}

/// A complete set of colour tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Dark or light.
    pub mode: Mode,
    /// Buy and sell in blue and orange instead of green and red.
    pub colour_blind: bool,
    /// Behind everything.
    pub background: Rgba,
    /// The inside of a panel.
    pub panel: Rgba,
    /// Every other row of a grid, and other faint fills.
    pub panel_alt: Rgba,
    /// Lines around panels and controls.
    pub border: Rgba,
    /// Normal text.
    pub text: Rgba,
    /// Labels, hints, less important numbers.
    pub text_muted: Rgba,
    /// Selection, focus, links, the active tab.
    pub accent: Rgba,
    /// Buy side, bids, gains: as text.
    pub buy: Rgba,
    /// Sell side, asks, losses: as text.
    pub sell: Rgba,
    /// Behind a price that just went up.
    pub up_tick: Rgba,
    /// Behind a price that just went down.
    pub down_tick: Rgba,
    /// Warnings.
    pub warning: Rgba,
    /// Errors.
    pub error: Rgba,
    /// Thin lines inside grids.
    pub grid_line: Rgba,
    /// Behind the selected row.
    pub selected_row: Rgba,
}

impl Theme {
    /// The theme for a mode, with or without the colour-blind option.
    pub fn new(mode: Mode, colour_blind: bool) -> Self {
        let base = match mode {
            Mode::Dark => Self::dark(),
            Mode::Light => Self::light(),
        };
        if colour_blind {
            base.with_colour_blind_trading_colours()
        } else {
            base
        }
    }

    /// The dark theme.
    pub fn dark() -> Self {
        Self {
            mode: Mode::Dark,
            colour_blind: false,
            background: Rgba::rgb(0x15, 0x17, 0x1a),
            panel: Rgba::rgb(0x1e, 0x21, 0x26),
            panel_alt: Rgba::rgb(0x23, 0x27, 0x2d),
            border: Rgba::rgb(0x34, 0x3a, 0x42),
            text: Rgba::rgb(0xe6, 0xe8, 0xeb),
            text_muted: Rgba::rgb(0xa7, 0xb0, 0xba),
            accent: Rgba::rgb(0x4c, 0x9a, 0xff),
            buy: Rgba::rgb(0x3d, 0xdc, 0x84),
            sell: Rgba::rgb(0xff, 0x6b, 0x6b),
            up_tick: Rgba::rgb(0x14, 0x33, 0x22),
            down_tick: Rgba::rgb(0x40, 0x16, 0x19),
            warning: Rgba::rgb(0xf5, 0xa6, 0x23),
            error: Rgba::rgb(0xff, 0x6b, 0x6b),
            grid_line: Rgba::rgb(0x2b, 0x30, 0x36),
            selected_row: Rgba::rgb(0x2a, 0x3b, 0x55),
        }
    }

    /// The light theme.
    pub fn light() -> Self {
        Self {
            mode: Mode::Light,
            colour_blind: false,
            background: Rgba::rgb(0xf5, 0xf6, 0xf8),
            panel: Rgba::rgb(0xff, 0xff, 0xff),
            panel_alt: Rgba::rgb(0xf0, 0xf2, 0xf5),
            border: Rgba::rgb(0xd5, 0xd9, 0xde),
            text: Rgba::rgb(0x1b, 0x1f, 0x24),
            text_muted: Rgba::rgb(0x5b, 0x65, 0x70),
            accent: Rgba::rgb(0x1f, 0x6f, 0xeb),
            buy: Rgba::rgb(0x13, 0x73, 0x33),
            sell: Rgba::rgb(0xb4, 0x23, 0x18),
            up_tick: Rgba::rgb(0xd7, 0xf0, 0xdd),
            down_tick: Rgba::rgb(0xf9, 0xd9, 0xd7),
            warning: Rgba::rgb(0x9a, 0x67, 0x00),
            error: Rgba::rgb(0xb4, 0x23, 0x18),
            grid_line: Rgba::rgb(0xe3, 0xe6, 0xea),
            selected_row: Rgba::rgb(0xdb, 0xe7, 0xfd),
        }
    }

    /// The same theme with buy and sell in blue and orange, which people with
    /// red-green colour blindness can tell apart.
    pub fn with_colour_blind_trading_colours(self) -> Self {
        let (buy, sell, up_tick, down_tick) = match self.mode {
            Mode::Dark => (
                Rgba::rgb(0x5a, 0xa9, 0xff),
                Rgba::rgb(0xff, 0xa6, 0x3d),
                Rgba::rgb(0x14, 0x29, 0x4a),
                Rgba::rgb(0x3d, 0x27, 0x07),
            ),
            Mode::Light => (
                Rgba::rgb(0x1a, 0x5f, 0xb4),
                Rgba::rgb(0xa6, 0x4b, 0x00),
                Rgba::rgb(0xd6, 0xe6, 0xfa),
                Rgba::rgb(0xfb, 0xe3, 0xc8),
            ),
        };
        Self {
            colour_blind: true,
            buy,
            sell,
            up_tick,
            down_tick,
            ..self
        }
    }
}

#[cfg(test)]
mod tests;
