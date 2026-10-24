//! Desktop colour palettes and egui style setup.
//!
//! This module keeps the same four curated themes but maps their semantic
//! slots onto egui visuals. All widgets read from [`Palette`], so switching
//! themes is a single lookup.

use eframe::egui::{
    self, style::WidgetVisuals, Color32, FontFamily, FontId, Stroke, Style, TextStyle, Visuals,
};

use crate::config::ThemeKind;
use crate::model::LogLevel;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub elevated: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub red: Color32,
    pub blue: Color32,
    pub purple: Color32,
    pub border: Color32,
    pub selection: Color32,
    pub light: bool,
}

impl Palette {
    pub fn for_theme(theme: ThemeKind) -> Self {
        match theme {
            ThemeKind::Mocha => Self {
                bg: rgb(0x18, 0x18, 0x25),
                panel: rgb(0x1e, 0x1e, 0x2e),
                surface: rgb(0x31, 0x32, 0x44),
                elevated: rgb(0x45, 0x47, 0x5a),
                text: rgb(0xcd, 0xd6, 0xf4),
                subtext: rgb(0xa6, 0xad, 0xc8),
                muted: rgb(0x7f, 0x84, 0x9c),
                accent: rgb(0xcb, 0xa6, 0xf7),
                accent_soft: rgb(0xf5, 0xc2, 0xe7),
                green: rgb(0xa6, 0xe3, 0xa1),
                yellow: rgb(0xf9, 0xe2, 0xaf),
                red: rgb(0xf3, 0x8b, 0xa8),
                blue: rgb(0x89, 0xb4, 0xfa),
                purple: rgb(0xb4, 0xbe, 0xfe),
                border: rgb(0x45, 0x47, 0x5a),
                selection: rgb(0x58, 0x5b, 0x70),
                light: false,
            },
            ThemeKind::Nord => Self {
                bg: rgb(0x2e, 0x34, 0x40),
                panel: rgb(0x33, 0x3a, 0x47),
                surface: rgb(0x3b, 0x42, 0x52),
                elevated: rgb(0x43, 0x4c, 0x5e),
                text: rgb(0xec, 0xef, 0xf4),
                subtext: rgb(0xd8, 0xde, 0xe9),
                muted: rgb(0x81, 0x8c, 0xa1),
                accent: rgb(0x88, 0xc0, 0xd0),
                accent_soft: rgb(0x8f, 0xbc, 0xbb),
                green: rgb(0xa3, 0xbe, 0x8c),
                yellow: rgb(0xeb, 0xcb, 0x8b),
                red: rgb(0xbf, 0x61, 0x6a),
                blue: rgb(0x81, 0xa1, 0xc1),
                purple: rgb(0xb4, 0x8e, 0xad),
                border: rgb(0x4c, 0x56, 0x6a),
                selection: rgb(0x4c, 0x56, 0x6a),
                light: false,
            },
            ThemeKind::Dracula => Self {
                bg: rgb(0x21, 0x22, 0x2c),
                panel: rgb(0x28, 0x2a, 0x36),
                surface: rgb(0x44, 0x47, 0x5a),
                elevated: rgb(0x52, 0x56, 0x6e),
                text: rgb(0xf8, 0xf8, 0xf2),
                subtext: rgb(0xc8, 0xc8, 0xd8),
                muted: rgb(0x62, 0x72, 0xa4),
                accent: rgb(0xbd, 0x93, 0xf9),
                accent_soft: rgb(0xff, 0x79, 0xc6),
                green: rgb(0x50, 0xfa, 0x7b),
                yellow: rgb(0xf1, 0xfa, 0x8c),
                red: rgb(0xff, 0x55, 0x55),
                blue: rgb(0x8b, 0xe9, 0xfd),
                purple: rgb(0xff, 0xb8, 0x6c),
                border: rgb(0x44, 0x47, 0x5a),
                selection: rgb(0x44, 0x47, 0x5a),
                light: false,
            },
            ThemeKind::Latte => Self {
                bg: rgb(0xef, 0xf1, 0xf5),
                panel: rgb(0xe6, 0xe9, 0xef),
                surface: rgb(0xdc, 0xe0, 0xe8),
                elevated: rgb(0xcc, 0xd0, 0xda),
                text: rgb(0x4c, 0x4f, 0x69),
                subtext: rgb(0x5c, 0x5f, 0x77),
                muted: rgb(0x8c, 0x8f, 0xa1),
                accent: rgb(0x88, 0x39, 0xef),
                accent_soft: rgb(0xea, 0x76, 0xcb),
                green: rgb(0x40, 0xa0, 0x2b),
                yellow: rgb(0xdf, 0x8e, 0x1d),
                red: rgb(0xd2, 0x0f, 0x39),
                blue: rgb(0x1e, 0x66, 0xf5),
                purple: rgb(0x72, 0x87, 0xfd),
                border: rgb(0xbc, 0xc0, 0xcc),
                selection: rgb(0xac, 0xb0, 0xbe),
                light: true,
            },
        }
    }

    pub fn log_color(&self, level: LogLevel) -> Color32 {
        match level {
            LogLevel::Info => self.blue,
            LogLevel::Success => self.green,
            LogLevel::Warn => self.yellow,
            LogLevel::Error => self.red,
        }
    }

    pub fn driver_color(&self, driver: crate::model::Driver) -> Color32 {
        match driver {
            crate::model::Driver::Postgres => self.blue,
            crate::model::Driver::MySql => self.yellow,
            crate::model::Driver::Sqlite => self.green,
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn apply(ctx: &egui::Context, theme: ThemeKind) {
    install_symbol_font(ctx);
    let p = Palette::for_theme(theme);
    let mut style = Style {
        visuals: if p.light {
            Visuals::light()
        } else {
            Visuals::dark()
        },
        ..Default::default()
    };
    let v = &mut style.visuals;
    v.dark_mode = !p.light;
    v.override_text_color = Some(p.text);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.bg;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.window_corner_radius = 10.into();
    v.menu_corner_radius = 8.into();
    v.hyperlink_color = p.blue;
    v.warn_fg_color = p.yellow;
    v.error_fg_color = p.red;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0_f32, p.text);
    v.widgets = egui::style::Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: p.panel,
            weak_bg_fill: p.panel,
            bg_stroke: Stroke::new(1.0_f32, p.border),
            corner_radius: 6.into(),
            fg_stroke: Stroke::new(1.0_f32, p.subtext),
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            bg_fill: p.surface,
            weak_bg_fill: p.surface,
            bg_stroke: Stroke::new(1.0_f32, p.border),
            corner_radius: 6.into(),
            fg_stroke: Stroke::new(1.0_f32, p.text),
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            bg_fill: p.elevated,
            weak_bg_fill: p.elevated,
            bg_stroke: Stroke::new(1.0_f32, p.accent),
            corner_radius: 6.into(),
            fg_stroke: Stroke::new(1.0_f32, p.text),
            expansion: 1.0,
        },
        active: WidgetVisuals {
            bg_fill: p.accent,
            weak_bg_fill: p.accent,
            bg_stroke: Stroke::new(1.0_f32, p.accent_soft),
            corner_radius: 6.into(),
            fg_stroke: Stroke::new(1.0_f32, p.bg),
            expansion: 1.0,
        },
        open: WidgetVisuals {
            bg_fill: p.elevated,
            weak_bg_fill: p.elevated,
            bg_stroke: Stroke::new(1.0_f32, p.border),
            corner_radius: 6.into(),
            fg_stroke: Stroke::new(1.0_f32, p.text),
            expansion: 0.0,
        },
    };

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(19.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 10.0;

    ctx.set_style(style);
}

/// Ubuntu-Light has no arrows or geometric shapes. Appending Hack (the
/// monospace face egui already bundles) as a last-resort fallback keeps the
/// schema icons, sort arrows and tab markers legible without shipping a font.
fn install_symbol_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .push("Hack".to_owned());
    ctx.set_fonts(fonts);
}

/// Return a one-line summary of the palette for tests and status tooltips.
pub fn summary(theme: ThemeKind) -> String {
    let p = Palette::for_theme(theme);
    format!(
        "{} · #{} text on #{}",
        theme.label(),
        hex(p.text),
        hex(p.panel)
    )
}

fn hex(color: Color32) -> String {
    format!("{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_have_readable_contrast() {
        for theme in ThemeKind::ALL {
            let p = Palette::for_theme(theme);
            let text = relative_luminance(p.text);
            let panel = relative_luminance(p.panel);
            let contrast = (text.max(panel) + 0.05) / (text.min(panel) + 0.05);
            assert!(
                contrast > 4.5,
                "{} contrast is only {contrast:.2}",
                theme.label()
            );
            assert!(!summary(theme).is_empty());
        }
    }

    fn relative_luminance(color: Color32) -> f32 {
        let channel = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }
}
