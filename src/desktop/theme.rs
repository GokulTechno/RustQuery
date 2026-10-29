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
                muted: rgb(0x8b, 0x90, 0xa8),
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
                muted: rgb(0x9a, 0xa7, 0xbf),
                accent: rgb(0x88, 0xc0, 0xd0),
                accent_soft: rgb(0x8f, 0xbc, 0xbb),
                green: rgb(0xa3, 0xbe, 0x8c),
                yellow: rgb(0xeb, 0xcb, 0x8b),
                red: rgb(0xe5, 0x94, 0x9d),
                blue: rgb(0x8f, 0xb1, 0xd1),
                purple: rgb(0xc7, 0x9f, 0xc2),
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
                muted: rgb(0x8b, 0x9b, 0xc9),
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
                muted: rgb(0x5f, 0x62, 0x73),
                accent: rgb(0x7c, 0x2e, 0xe0),
                accent_soft: rgb(0xea, 0x76, 0xcb),
                green: rgb(0x2a, 0x71, 0x1b),
                yellow: rgb(0x8f, 0x54, 0x00),
                red: rgb(0xc5, 0x0e, 0x34),
                blue: rgb(0x18, 0x59, 0xd6),
                purple: rgb(0x4c, 0x56, 0xc4),
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

    /// Foreground on filled accent buttons (Run, Connect, primary modal actions).
    pub fn on_accent(&self) -> Color32 {
        if self.light {
            Color32::WHITE
        } else {
            self.bg
        }
    }

    /// Foreground on filled destructive buttons (Stop, delete confirmations).
    pub fn on_destructive(&self) -> Color32 {
        if self.light {
            Color32::WHITE
        } else {
            self.bg
        }
    }

    /// Dimmed overlay behind modal dialogs.
    pub fn modal_scrim(&self) -> Color32 {
        if self.light {
            Color32::from_black_alpha(72)
        } else {
            Color32::from_black_alpha(150)
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Layout sizes derived from the user font-scale setting.
#[derive(Debug, Clone, Copy)]
pub struct UiMetrics {
    pub body: f32,
    pub small: f32,
    pub heading: f32,
    pub monospace: f32,
    pub interact_height: f32,
    pub tab_bar_height: f32,
    pub status_bar_height: f32,
    pub grid_row_height: f32,
    pub grid_header_height: f32,
    pub grid_gutter: f32,
}

pub fn metrics(scale_percent: u16) -> UiMetrics {
    let s = (scale_percent.clamp(90, 130) as f32) / 100.0;
    UiMetrics {
        small: 12.0 * s,
        body: 14.5 * s,
        heading: 20.5 * s,
        monospace: 14.0 * s,
        interact_height: 30.0 * s,
        tab_bar_height: 42.0 * s,
        status_bar_height: 32.0 * s,
        grid_row_height: 24.0 * s,
        grid_header_height: 32.0 * s,
        grid_gutter: 62.0 * s,
    }
}

pub fn apply(ctx: &egui::Context, theme: ThemeKind, scale_percent: u16) {
    install_symbol_font(ctx);
    let m = metrics(scale_percent);
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
    v.selection.stroke = Stroke::new(1.0_f32, p.accent.gamma_multiply(0.85));
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
            FontId::new(m.small, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(m.body, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(m.body, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(m.heading, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(m.monospace, FontFamily::Monospace),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(9.0 * (s(scale_percent)), 7.0 * (s(scale_percent)));
    style.spacing.button_padding = egui::vec2(12.0 * (s(scale_percent)), 6.0 * (s(scale_percent)));
    style.spacing.interact_size.y = m.interact_height;
    style.spacing.scroll.bar_width = 11.0 * (s(scale_percent));

    ctx.set_style(style);
}

fn s(scale_percent: u16) -> f32 {
    (scale_percent.clamp(90, 130) as f32) / 100.0
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
            // Body copy, muted hints and semantic labels all appear directly
            // on the panel or the editor/result background.
            for (name, color) in [
                ("text", p.text),
                ("subtext", p.subtext),
                ("muted", p.muted),
                ("accent", p.accent),
                ("green", p.green),
                ("yellow", p.yellow),
                ("red", p.red),
                ("blue", p.blue),
                ("purple", p.purple),
            ] {
                for (surface_name, surface) in [("panel", p.panel), ("background", p.bg)] {
                    let contrast = contrast_ratio(color, surface);
                    assert!(
                        contrast >= 4.5,
                        "{} {name} on {surface_name} is only {contrast:.2}:1",
                        theme.label()
                    );
                }
            }
            assert!(
                contrast_ratio(p.on_accent(), p.accent) >= 4.5,
                "{} accent button label is unreadable",
                theme.label()
            );
            assert!(
                contrast_ratio(p.on_destructive(), p.red) >= 4.5,
                "{} destructive button label is unreadable",
                theme.label()
            );
            assert!(!summary(theme).is_empty());
        }
    }

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let a = relative_luminance(a);
        let b = relative_luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
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
