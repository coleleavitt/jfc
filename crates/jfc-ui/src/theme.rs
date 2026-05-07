#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub border: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub user_bubble_bg: Color,
    pub asst_bubble_bg: Color,
    pub code_bg: Color,
    pub code_fg: Color,
    pub code_string: Color,
    pub code_keyword: Color,
    pub code_comment: Color,
    pub code_number: Color,
    pub reasoning_bg: Color,
    pub reasoning_fg: Color,
}

impl Theme {
    /// Default dark theme — high-contrast indigo/blue accents.
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(15, 15, 20),
            surface: Color::Rgb(25, 25, 35),
            surface_raised: Color::Rgb(35, 35, 50),
            border: Color::Rgb(55, 55, 75),
            text_primary: Color::Rgb(220, 220, 230),
            text_secondary: Color::Rgb(160, 160, 180),
            text_muted: Color::Rgb(90, 90, 110),
            accent: Color::Rgb(100, 160, 255),
            success: Color::Rgb(100, 210, 130),
            warning: Color::Rgb(255, 190, 80),
            error: Color::Rgb(255, 100, 100),
            user_bubble_bg: Color::Rgb(30, 45, 70),
            asst_bubble_bg: Color::Rgb(25, 30, 40),
            code_bg: Color::Rgb(20, 20, 30),
            code_fg: Color::Rgb(200, 200, 210),
            code_string: Color::Rgb(150, 220, 130),
            code_keyword: Color::Rgb(130, 170, 255),
            code_comment: Color::Rgb(100, 110, 130),
            code_number: Color::Rgb(255, 180, 100),
            reasoning_bg: Color::Rgb(30, 30, 45),
            reasoning_fg: Color::Rgb(120, 130, 160),
        }
    }

    /// Light theme — soft cream background with conservative accents.
    /// Tuned for daytime use; contrast meets WCAG AA on body text.
    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(250, 248, 244),
            surface: Color::Rgb(240, 236, 230),
            surface_raised: Color::Rgb(228, 222, 214),
            border: Color::Rgb(190, 184, 175),
            text_primary: Color::Rgb(40, 35, 30),
            text_secondary: Color::Rgb(85, 80, 70),
            text_muted: Color::Rgb(140, 130, 115),
            accent: Color::Rgb(50, 110, 200),
            success: Color::Rgb(40, 140, 70),
            warning: Color::Rgb(190, 110, 30),
            error: Color::Rgb(190, 50, 50),
            user_bubble_bg: Color::Rgb(225, 235, 245),
            asst_bubble_bg: Color::Rgb(235, 232, 226),
            code_bg: Color::Rgb(232, 228, 220),
            code_fg: Color::Rgb(50, 45, 40),
            code_string: Color::Rgb(50, 130, 60),
            code_keyword: Color::Rgb(120, 50, 160),
            code_comment: Color::Rgb(140, 130, 115),
            code_number: Color::Rgb(180, 90, 30),
            reasoning_bg: Color::Rgb(238, 234, 226),
            reasoning_fg: Color::Rgb(110, 100, 85),
        }
    }

    /// Solarized dark — Ethan Schoonover's palette, mapped onto our
    /// theme slots. Beloved by terminal users for low eye-strain.
    pub fn solarized_dark() -> Self {
        Self {
            bg: Color::Rgb(0, 43, 54),
            surface: Color::Rgb(7, 54, 66),
            surface_raised: Color::Rgb(20, 70, 84),
            border: Color::Rgb(40, 95, 110),
            text_primary: Color::Rgb(238, 232, 213),
            text_secondary: Color::Rgb(147, 161, 161),
            text_muted: Color::Rgb(101, 123, 131),
            accent: Color::Rgb(38, 139, 210),
            success: Color::Rgb(133, 153, 0),
            warning: Color::Rgb(181, 137, 0),
            error: Color::Rgb(220, 50, 47),
            user_bubble_bg: Color::Rgb(7, 54, 66),
            asst_bubble_bg: Color::Rgb(0, 43, 54),
            code_bg: Color::Rgb(0, 36, 46),
            code_fg: Color::Rgb(238, 232, 213),
            code_string: Color::Rgb(133, 153, 0),
            code_keyword: Color::Rgb(38, 139, 210),
            code_comment: Color::Rgb(88, 110, 117),
            code_number: Color::Rgb(203, 75, 22),
            reasoning_bg: Color::Rgb(7, 54, 66),
            reasoning_fg: Color::Rgb(147, 161, 161),
        }
    }

    /// Catppuccin mocha — popular pastel-on-dark palette. Mirrors
    /// the syntect theme jfc bundles via `two-face`, so prose and
    /// fenced code share a coherent look.
    pub fn catppuccin() -> Self {
        Self {
            bg: Color::Rgb(30, 30, 46),                // Base
            surface: Color::Rgb(49, 50, 68),           // Surface0
            surface_raised: Color::Rgb(69, 71, 90),    // Surface1
            border: Color::Rgb(88, 91, 112),           // Surface2
            text_primary: Color::Rgb(205, 214, 244),   // Text
            text_secondary: Color::Rgb(186, 194, 222), // Subtext1
            text_muted: Color::Rgb(127, 132, 156),     // Overlay1
            accent: Color::Rgb(137, 180, 250),         // Blue
            success: Color::Rgb(166, 227, 161),        // Green
            warning: Color::Rgb(249, 226, 175),        // Yellow
            error: Color::Rgb(243, 139, 168),          // Red
            user_bubble_bg: Color::Rgb(49, 50, 68),
            asst_bubble_bg: Color::Rgb(30, 30, 46),
            code_bg: Color::Rgb(24, 24, 37), // Mantle
            code_fg: Color::Rgb(205, 214, 244),
            code_string: Color::Rgb(166, 227, 161),
            code_keyword: Color::Rgb(203, 166, 247), // Mauve
            code_comment: Color::Rgb(127, 132, 156),
            code_number: Color::Rgb(250, 179, 135), // Peach
            reasoning_bg: Color::Rgb(49, 50, 68),
            reasoning_fg: Color::Rgb(166, 173, 200),
        }
    }

    /// Tokyo Night — folke/tokyonight.nvim. Cool indigo background
    /// with violet/blue accents; popular for low eye-strain night work.
    pub fn tokyo_night() -> Self {
        Self {
            bg: Color::Rgb(26, 27, 38),
            surface: Color::Rgb(22, 22, 30),
            surface_raised: Color::Rgb(41, 46, 66),
            border: Color::Rgb(65, 72, 104),
            text_primary: Color::Rgb(192, 202, 245),
            text_secondary: Color::Rgb(169, 177, 214),
            text_muted: Color::Rgb(86, 95, 137),
            accent: Color::Rgb(122, 162, 247),
            success: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            user_bubble_bg: Color::Rgb(30, 32, 48),
            asst_bubble_bg: Color::Rgb(22, 22, 30),
            code_bg: Color::Rgb(20, 21, 30),
            code_fg: Color::Rgb(192, 202, 245),
            code_string: Color::Rgb(158, 206, 106),
            code_keyword: Color::Rgb(187, 154, 247),
            code_comment: Color::Rgb(86, 95, 137),
            code_number: Color::Rgb(255, 158, 100),
            reasoning_bg: Color::Rgb(36, 40, 59),
            reasoning_fg: Color::Rgb(154, 165, 206),
        }
    }

    /// Dracula — dracula/dracula. Vivid violet/pink/cyan on dark
    /// charcoal; arguably the most-recognizable dark palette.
    pub fn dracula() -> Self {
        Self {
            bg: Color::Rgb(40, 42, 54),
            surface: Color::Rgb(44, 46, 62),
            surface_raised: Color::Rgb(68, 71, 90),
            border: Color::Rgb(98, 114, 164),
            text_primary: Color::Rgb(248, 248, 242),
            text_secondary: Color::Rgb(191, 191, 191),
            text_muted: Color::Rgb(98, 114, 164),
            accent: Color::Rgb(189, 147, 249),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            error: Color::Rgb(255, 85, 85),
            user_bubble_bg: Color::Rgb(44, 46, 62),
            asst_bubble_bg: Color::Rgb(40, 42, 54),
            code_bg: Color::Rgb(34, 36, 48),
            code_fg: Color::Rgb(248, 248, 242),
            code_string: Color::Rgb(241, 250, 140),
            code_keyword: Color::Rgb(255, 121, 198),
            code_comment: Color::Rgb(98, 114, 164),
            code_number: Color::Rgb(189, 147, 249),
            reasoning_bg: Color::Rgb(50, 52, 68),
            reasoning_fg: Color::Rgb(180, 180, 200),
        }
    }

    /// Nord — arcticicestudio/nord. Cold polar palette with subdued
    /// frost accents — strong for daytime use under bright lighting.
    pub fn nord() -> Self {
        Self {
            bg: Color::Rgb(46, 52, 64),       // nord0
            surface: Color::Rgb(59, 66, 82),  // nord1
            surface_raised: Color::Rgb(67, 76, 94), // nord2
            border: Color::Rgb(76, 86, 106),  // nord3
            text_primary: Color::Rgb(216, 222, 233), // nord4
            text_secondary: Color::Rgb(229, 233, 240), // nord5
            text_muted: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(136, 192, 208), // nord8 (frost)
            success: Color::Rgb(163, 190, 140), // nord14
            warning: Color::Rgb(235, 203, 139), // nord13
            error: Color::Rgb(191, 97, 106),    // nord11
            user_bubble_bg: Color::Rgb(59, 66, 82),
            asst_bubble_bg: Color::Rgb(46, 52, 64),
            code_bg: Color::Rgb(36, 41, 51),
            code_fg: Color::Rgb(216, 222, 233),
            code_string: Color::Rgb(163, 190, 140),
            code_keyword: Color::Rgb(129, 161, 193), // nord9
            code_comment: Color::Rgb(76, 86, 106),
            code_number: Color::Rgb(180, 142, 173), // nord15
            reasoning_bg: Color::Rgb(59, 66, 82),
            reasoning_fg: Color::Rgb(180, 188, 200),
        }
    }

    /// Gruvbox Dark — morhetz/gruvbox. Warm retro greys with
    /// orange/yellow accents; cherished by old-school terminal users.
    pub fn gruvbox_dark() -> Self {
        Self {
            bg: Color::Rgb(40, 40, 40),       // bg0
            surface: Color::Rgb(60, 56, 54),  // bg1
            surface_raised: Color::Rgb(80, 73, 69), // bg2
            border: Color::Rgb(102, 92, 84),  // bg3
            text_primary: Color::Rgb(235, 219, 178), // fg1
            text_secondary: Color::Rgb(213, 196, 161), // fg2
            text_muted: Color::Rgb(168, 153, 132),     // fg4
            accent: Color::Rgb(131, 165, 152),         // blue/aqua
            success: Color::Rgb(184, 187, 38),         // green
            warning: Color::Rgb(250, 189, 47),         // yellow
            error: Color::Rgb(251, 73, 52),            // red
            user_bubble_bg: Color::Rgb(60, 56, 54),
            asst_bubble_bg: Color::Rgb(40, 40, 40),
            code_bg: Color::Rgb(29, 32, 33), // bg0_h
            code_fg: Color::Rgb(235, 219, 178),
            code_string: Color::Rgb(184, 187, 38),
            code_keyword: Color::Rgb(251, 73, 52),
            code_comment: Color::Rgb(146, 131, 116), // gray
            code_number: Color::Rgb(211, 134, 155),  // purple
            reasoning_bg: Color::Rgb(60, 56, 54),
            reasoning_fg: Color::Rgb(189, 174, 147),
        }
    }

    /// Monokai — TextMate's classic. Hot pink keywords, lime strings,
    /// cyan accent on black-coffee bg.
    pub fn monokai() -> Self {
        Self {
            bg: Color::Rgb(39, 40, 34),
            surface: Color::Rgb(45, 46, 37),
            surface_raised: Color::Rgb(62, 61, 50),
            border: Color::Rgb(73, 72, 62),
            text_primary: Color::Rgb(248, 248, 242),
            text_secondary: Color::Rgb(207, 207, 194),
            text_muted: Color::Rgb(117, 113, 94),
            accent: Color::Rgb(102, 217, 239),
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(230, 219, 116),
            error: Color::Rgb(249, 38, 114),
            user_bubble_bg: Color::Rgb(45, 46, 37),
            asst_bubble_bg: Color::Rgb(39, 40, 34),
            code_bg: Color::Rgb(33, 34, 28),
            code_fg: Color::Rgb(248, 248, 242),
            code_string: Color::Rgb(230, 219, 116),
            code_keyword: Color::Rgb(249, 38, 114),
            code_comment: Color::Rgb(117, 113, 94),
            code_number: Color::Rgb(174, 129, 255),
            reasoning_bg: Color::Rgb(48, 49, 41),
            reasoning_fg: Color::Rgb(207, 207, 194),
        }
    }

    /// Ayu Dark — ayu-theme/ayu. Deep navy with electric cyan and
    /// orange-yellow accents. Sister to Tokyo Night with warmer code.
    pub fn ayu_dark() -> Self {
        Self {
            bg: Color::Rgb(10, 14, 20),
            surface: Color::Rgb(15, 20, 25),
            surface_raised: Color::Rgb(22, 27, 34),
            border: Color::Rgb(40, 50, 64),
            text_primary: Color::Rgb(179, 177, 173),
            text_secondary: Color::Rgb(130, 136, 146),
            text_muted: Color::Rgb(77, 85, 102),
            accent: Color::Rgb(57, 186, 230),
            success: Color::Rgb(194, 217, 76),
            warning: Color::Rgb(255, 180, 84),
            error: Color::Rgb(240, 113, 120),
            user_bubble_bg: Color::Rgb(15, 20, 25),
            asst_bubble_bg: Color::Rgb(10, 14, 20),
            code_bg: Color::Rgb(7, 11, 16),
            code_fg: Color::Rgb(179, 177, 173),
            code_string: Color::Rgb(194, 217, 76),
            code_keyword: Color::Rgb(255, 143, 64),
            code_comment: Color::Rgb(92, 103, 115),
            code_number: Color::Rgb(255, 180, 84),
            reasoning_bg: Color::Rgb(20, 25, 31),
            reasoning_fg: Color::Rgb(150, 158, 170),
        }
    }

    /// Rose Pine — rose-pine/rose-pine. Soft mauve/foam/rose
    /// pastels on a deep purple-grey bg. Gentle on the eyes.
    pub fn rose_pine() -> Self {
        Self {
            bg: Color::Rgb(25, 23, 36),       // base
            surface: Color::Rgb(31, 29, 46),  // surface
            surface_raised: Color::Rgb(38, 35, 58), // overlay
            border: Color::Rgb(64, 60, 88),
            text_primary: Color::Rgb(224, 222, 244), // text
            text_secondary: Color::Rgb(144, 140, 170), // subtle
            text_muted: Color::Rgb(110, 106, 134), // muted
            accent: Color::Rgb(196, 167, 231), // iris
            success: Color::Rgb(156, 207, 216), // foam
            warning: Color::Rgb(246, 193, 119), // gold
            error: Color::Rgb(235, 111, 146), // love
            user_bubble_bg: Color::Rgb(31, 29, 46),
            asst_bubble_bg: Color::Rgb(25, 23, 36),
            code_bg: Color::Rgb(20, 18, 30),
            code_fg: Color::Rgb(224, 222, 244),
            code_string: Color::Rgb(246, 193, 119),
            code_keyword: Color::Rgb(196, 167, 231),
            code_comment: Color::Rgb(110, 106, 134),
            code_number: Color::Rgb(235, 188, 186), // rose
            reasoning_bg: Color::Rgb(31, 29, 46),
            reasoning_fg: Color::Rgb(180, 175, 210),
        }
    }

    /// One Dark — Atom's flagship. Slate background with cool blue
    /// accent and warm orange numbers; balanced for full-day work.
    pub fn one_dark() -> Self {
        Self {
            bg: Color::Rgb(40, 44, 52),
            surface: Color::Rgb(33, 37, 43),
            surface_raised: Color::Rgb(62, 68, 81),
            border: Color::Rgb(82, 90, 102),
            text_primary: Color::Rgb(171, 178, 191),
            text_secondary: Color::Rgb(130, 137, 151),
            text_muted: Color::Rgb(92, 99, 112),
            accent: Color::Rgb(97, 175, 239),
            success: Color::Rgb(152, 195, 121),
            warning: Color::Rgb(229, 192, 123),
            error: Color::Rgb(224, 108, 117),
            user_bubble_bg: Color::Rgb(33, 37, 43),
            asst_bubble_bg: Color::Rgb(40, 44, 52),
            code_bg: Color::Rgb(28, 31, 38),
            code_fg: Color::Rgb(171, 178, 191),
            code_string: Color::Rgb(152, 195, 121),
            code_keyword: Color::Rgb(198, 120, 221),
            code_comment: Color::Rgb(92, 99, 112),
            code_number: Color::Rgb(209, 154, 102),
            reasoning_bg: Color::Rgb(45, 49, 58),
            reasoning_fg: Color::Rgb(160, 168, 182),
        }
    }

    /// GitHub Light — primer/github-vscode-theme. Crisp white bg
    /// with GitHub's official accent palette. Excellent for daytime
    /// office work or sharing screenshots.
    pub fn github_light() -> Self {
        Self {
            bg: Color::Rgb(255, 255, 255),
            surface: Color::Rgb(246, 248, 250),
            surface_raised: Color::Rgb(234, 238, 242),
            border: Color::Rgb(208, 215, 222),
            text_primary: Color::Rgb(36, 41, 47),
            text_secondary: Color::Rgb(87, 96, 106),
            text_muted: Color::Rgb(110, 119, 129),
            accent: Color::Rgb(9, 105, 218),
            success: Color::Rgb(26, 127, 55),
            warning: Color::Rgb(154, 103, 0),
            error: Color::Rgb(207, 34, 46),
            user_bubble_bg: Color::Rgb(221, 244, 255),
            asst_bubble_bg: Color::Rgb(246, 248, 250),
            code_bg: Color::Rgb(246, 248, 250),
            code_fg: Color::Rgb(36, 41, 47),
            code_string: Color::Rgb(10, 48, 105),
            code_keyword: Color::Rgb(207, 34, 46),
            code_comment: Color::Rgb(110, 119, 129),
            code_number: Color::Rgb(5, 80, 174),
            reasoning_bg: Color::Rgb(243, 246, 249),
            reasoning_fg: Color::Rgb(80, 88, 100),
        }
    }

    /// Look up a theme by name. Returns None for unknown names so
    /// the caller can show an error toast. Lookup is case-insensitive
    /// and accepts aliases (`solarized` ↔ `solarized-dark`,
    /// `catppuccin` ↔ `catppuccin-mocha`, `tokyo` ↔ `tokyo-night`).
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            "solarized" | "solarized-dark" => Some(Self::solarized_dark()),
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin()),
            "tokyo" | "tokyo-night" | "tokyonight" => Some(Self::tokyo_night()),
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            "gruvbox" | "gruvbox-dark" => Some(Self::gruvbox_dark()),
            "monokai" => Some(Self::monokai()),
            "ayu" | "ayu-dark" => Some(Self::ayu_dark()),
            "rose-pine" | "rosepine" | "rose_pine" => Some(Self::rose_pine()),
            "one-dark" | "onedark" | "atom" | "atom-one-dark" => Some(Self::one_dark()),
            "github" | "github-light" => Some(Self::github_light()),
            _ => None,
        }
    }

    /// Canonical names for `/theme` listing. Aliases are NOT included
    /// — users see one entry per visually distinct palette.
    pub fn available_names() -> &'static [&'static str] {
        &[
            "dark",
            "light",
            "solarized",
            "catppuccin",
            "tokyo-night",
            "dracula",
            "nord",
            "gruvbox",
            "monokai",
            "ayu",
            "rose-pine",
            "one-dark",
            "github-light",
        ]
    }
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::default().fg(self.text_primary).bg(self.bg)
    }
    pub fn surface(&self) -> Style {
        Style::default().bg(self.surface)
    }
    pub fn border(&self) -> Style {
        Style::default().fg(self.border)
    }
    pub fn muted(&self) -> Style {
        Style::default().fg(self.text_muted)
    }
    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent)
    }
    pub fn bold(&self) -> Style {
        Style::default()
            .fg(self.text_primary)
            .add_modifier(Modifier::BOLD)
    }
    pub fn italic(&self) -> Style {
        Style::default()
            .fg(self.text_primary)
            .add_modifier(Modifier::ITALIC)
    }
    pub fn success(&self) -> Style {
        Style::default().fg(self.success)
    }
    pub fn warning(&self) -> Style {
        Style::default().fg(self.warning)
    }
    pub fn error(&self) -> Style {
        Style::default().fg(self.error)
    }
    pub fn code_block(&self) -> Style {
        Style::default().fg(self.code_fg).bg(self.code_bg)
    }
    pub fn inline_code(&self) -> Style {
        Style::default().fg(self.code_string)
    }
    pub fn reasoning(&self) -> Style {
        Style::default().fg(self.reasoning_fg).bg(self.reasoning_bg)
    }
    pub fn user_label(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub fn asst_label(&self) -> Style {
        Style::default()
            .fg(self.text_secondary)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pull the RGB triple out of a `Color`. Returns None for non-RGB
    /// variants so tests can detect when a theme accidentally maps a
    /// slot to a 16-color terminal value (which would render incorrectly
    /// against a custom palette).
    fn rgb_of(color: Color) -> Option<(u8, u8, u8)> {
        match color {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        }
    }

    /// Every slot on every bundled theme must be a true-color RGB value —
    /// using a 16-color or palette index would produce the wrong contrast
    /// against neighbouring slots that *are* RGB.
    fn assert_all_slots_rgb(theme: &Theme, name: &str) {
        let slots: [(&str, Color); 21] = [
            ("bg", theme.bg),
            ("surface", theme.surface),
            ("surface_raised", theme.surface_raised),
            ("border", theme.border),
            ("text_primary", theme.text_primary),
            ("text_secondary", theme.text_secondary),
            ("text_muted", theme.text_muted),
            ("accent", theme.accent),
            ("success", theme.success),
            ("warning", theme.warning),
            ("error", theme.error),
            ("user_bubble_bg", theme.user_bubble_bg),
            ("asst_bubble_bg", theme.asst_bubble_bg),
            ("code_bg", theme.code_bg),
            ("code_fg", theme.code_fg),
            ("code_string", theme.code_string),
            ("code_keyword", theme.code_keyword),
            ("code_comment", theme.code_comment),
            ("code_number", theme.code_number),
            ("reasoning_bg", theme.reasoning_bg),
            ("reasoning_fg", theme.reasoning_fg),
        ];
        for (slot_name, color) in slots {
            assert!(
                rgb_of(color).is_some(),
                "theme {name} slot {slot_name} must be Color::Rgb, got {color:?}",
            );
        }
    }

    /// Distinct foreground/background pairs are required for legibility:
    /// if `text_primary == bg` the theme is unreadable. The minimum 32 / 256
    /// luminance gap is conservative — actual perceptual contrast checks
    /// would need a WCAG calculation, but a flat-equal check catches the
    /// most common authoring mistake.
    fn assert_text_distinct_from_bg(theme: &Theme, name: &str) {
        let (fr, fg, fb) = rgb_of(theme.text_primary).unwrap();
        let (br, bg, bb) = rgb_of(theme.bg).unwrap();
        let max_diff = ((fr as i32 - br as i32).abs())
            .max((fg as i32 - bg as i32).abs())
            .max((fb as i32 - bb as i32).abs());
        assert!(
            max_diff > 32,
            "theme {name}: text_primary and bg too close — max channel diff {max_diff}",
        );
    }

    // ─── per-theme palette sanity ────────────────────────────────────────

    #[test]
    fn dark_theme_has_rgb_slots_normal() {
        let t = Theme::dark();
        assert_all_slots_rgb(&t, "dark");
        assert_text_distinct_from_bg(&t, "dark");
    }

    #[test]
    fn light_theme_has_rgb_slots_normal() {
        let t = Theme::light();
        assert_all_slots_rgb(&t, "light");
        assert_text_distinct_from_bg(&t, "light");
    }

    #[test]
    fn solarized_dark_theme_has_rgb_slots_normal() {
        let t = Theme::solarized_dark();
        assert_all_slots_rgb(&t, "solarized_dark");
        assert_text_distinct_from_bg(&t, "solarized_dark");
    }

    #[test]
    fn catppuccin_theme_has_rgb_slots_normal() {
        let t = Theme::catppuccin();
        assert_all_slots_rgb(&t, "catppuccin");
        assert_text_distinct_from_bg(&t, "catppuccin");
    }

    /// Every canonical theme — including the new opencode-style palettes —
    /// gets the same legibility + RGB-slot guarantees as the originals.
    /// Using `available_names()` here means new themes added to the list
    /// pick up these checks automatically without per-theme test code.
    #[test]
    fn every_canonical_theme_passes_palette_checks_normal() {
        for name in Theme::available_names() {
            let t = Theme::by_name(name)
                .unwrap_or_else(|| panic!("available name {name:?} must resolve"));
            assert_all_slots_rgb(&t, name);
            assert_text_distinct_from_bg(&t, name);
            // Semantic colors must be distinct so red/yellow/green
            // can't collide on any palette.
            assert_ne!(rgb_of(t.success), rgb_of(t.warning), "{name}: success vs warning");
            assert_ne!(rgb_of(t.warning), rgb_of(t.error), "{name}: warning vs error");
            assert_ne!(rgb_of(t.success), rgb_of(t.error), "{name}: success vs error");
        }
    }

    #[test]
    fn dark_and_light_have_inverted_brightness_robust() {
        // Sanity-check the dark/light division: dark.bg should be much
        // darker than light.bg. If a refactor accidentally swapped them
        // this test catches it before users see white-on-white.
        let dark_bg_luma: u32 = {
            let (r, g, b) = rgb_of(Theme::dark().bg).unwrap();
            r as u32 + g as u32 + b as u32
        };
        let light_bg_luma: u32 = {
            let (r, g, b) = rgb_of(Theme::light().bg).unwrap();
            r as u32 + g as u32 + b as u32
        };
        assert!(
            light_bg_luma > dark_bg_luma + 200,
            "light bg luma ({light_bg_luma}) should dwarf dark bg luma ({dark_bg_luma})",
        );
    }

    #[test]
    fn each_theme_distinguishes_user_and_asst_bubbles_robust() {
        for (name, theme) in [
            ("dark", Theme::dark()),
            ("light", Theme::light()),
            ("solarized_dark", Theme::solarized_dark()),
            ("catppuccin", Theme::catppuccin()),
        ] {
            assert_ne!(
                rgb_of(theme.user_bubble_bg),
                rgb_of(theme.asst_bubble_bg),
                "theme {name}: user/asst bubble must be visually distinct",
            );
        }
    }

    #[test]
    fn semantic_colors_are_distinct_per_theme_robust() {
        // success/warning/error must each be different — otherwise a red
        // exit code looks identical to a yellow warning.
        for (name, theme) in [
            ("dark", Theme::dark()),
            ("light", Theme::light()),
            ("solarized_dark", Theme::solarized_dark()),
            ("catppuccin", Theme::catppuccin()),
        ] {
            assert_ne!(
                rgb_of(theme.success),
                rgb_of(theme.warning),
                "theme {name}: success and warning must differ",
            );
            assert_ne!(
                rgb_of(theme.warning),
                rgb_of(theme.error),
                "theme {name}: warning and error must differ",
            );
            assert_ne!(
                rgb_of(theme.success),
                rgb_of(theme.error),
                "theme {name}: success and error must differ",
            );
        }
    }

    // ─── Theme::by_name dispatch ─────────────────────────────────────────

    #[test]
    fn by_name_resolves_canonical_names_normal() {
        assert!(Theme::by_name("dark").is_some());
        assert!(Theme::by_name("light").is_some());
        assert!(Theme::by_name("solarized").is_some());
        assert!(Theme::by_name("catppuccin").is_some());
    }

    #[test]
    fn by_name_resolves_aliases_normal() {
        // Both "solarized" and "solarized-dark" should map to the same
        // theme — and likewise "catppuccin" / "catppuccin-mocha".
        let s1 = Theme::by_name("solarized").unwrap();
        let s2 = Theme::by_name("solarized-dark").unwrap();
        assert_eq!(rgb_of(s1.bg), rgb_of(s2.bg));

        let c1 = Theme::by_name("catppuccin").unwrap();
        let c2 = Theme::by_name("catppuccin-mocha").unwrap();
        assert_eq!(rgb_of(c1.bg), rgb_of(c2.bg));
    }

    #[test]
    fn by_name_returns_none_for_unknown_robust() {
        assert!(Theme::by_name("not-a-theme").is_none());
        assert!(Theme::by_name("").is_none());
    }

    /// Lookup is case-insensitive — users frequently type `/theme Dracula`
    /// and shouldn't have to memorise the canonical lowercase form. The
    /// previous version of this test asserted case-*sensitive* lookup
    /// returned None; we changed the contract because the case-folded
    /// form costs nothing and avoids "unknown theme 'DARK'" papercuts.
    #[test]
    fn by_name_is_case_insensitive_normal() {
        assert!(Theme::by_name("DARK").is_some());
        assert!(Theme::by_name("Dracula").is_some());
        assert!(Theme::by_name("Tokyo-Night").is_some());
        assert!(Theme::by_name("GITHUB-LIGHT").is_some());
    }

    #[test]
    fn available_names_is_non_empty_and_resolves_normal() {
        let names = Theme::available_names();
        assert!(!names.is_empty(), "must list at least one theme");
        for name in names {
            assert!(
                Theme::by_name(name).is_some(),
                "available name {name:?} must resolve via by_name",
            );
        }
    }

    #[test]
    fn available_names_does_not_include_aliases_normal() {
        // The list shows canonical names only — aliases ("solarized-dark",
        // "catppuccin-mocha") aren't surfaced.
        let names = Theme::available_names();
        assert!(!names.contains(&"solarized-dark"));
        assert!(!names.contains(&"catppuccin-mocha"));
    }

    // ─── Style helpers (impl block 2) ─────────────────────────────────────

    #[test]
    fn base_style_uses_text_primary_and_bg_normal() {
        let theme = Theme::dark();
        let style = theme.base();
        assert_eq!(style.fg, Some(theme.text_primary));
        assert_eq!(style.bg, Some(theme.bg));
    }

    #[test]
    fn surface_style_uses_surface_color_normal() {
        let theme = Theme::dark();
        let style = theme.surface();
        assert_eq!(style.bg, Some(theme.surface));
    }

    #[test]
    fn border_style_uses_border_fg_normal() {
        let theme = Theme::light();
        let style = theme.border();
        assert_eq!(style.fg, Some(theme.border));
    }

    #[test]
    fn muted_style_uses_text_muted_normal() {
        let theme = Theme::dark();
        let style = theme.muted();
        assert_eq!(style.fg, Some(theme.text_muted));
    }

    #[test]
    fn accent_style_uses_accent_color_normal() {
        let theme = Theme::dark();
        let style = theme.accent();
        assert_eq!(style.fg, Some(theme.accent));
    }

    #[test]
    fn bold_and_italic_styles_carry_modifiers_normal() {
        let theme = Theme::dark();

        let b = theme.bold();
        assert!(b.add_modifier.contains(Modifier::BOLD));

        let i = theme.italic();
        assert!(i.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn semantic_helpers_use_semantic_slots_normal() {
        let theme = Theme::dark();
        assert_eq!(theme.success().fg, Some(theme.success));
        assert_eq!(theme.warning().fg, Some(theme.warning));
        assert_eq!(theme.error().fg, Some(theme.error));
    }

    #[test]
    fn code_block_style_combines_code_fg_and_bg_normal() {
        let theme = Theme::dark();
        let style = theme.code_block();
        assert_eq!(style.fg, Some(theme.code_fg));
        assert_eq!(style.bg, Some(theme.code_bg));
    }

    #[test]
    fn inline_code_uses_code_string_color_normal() {
        let theme = Theme::dark();
        let style = theme.inline_code();
        assert_eq!(style.fg, Some(theme.code_string));
    }

    #[test]
    fn reasoning_combines_fg_and_bg_normal() {
        let theme = Theme::dark();
        let style = theme.reasoning();
        assert_eq!(style.fg, Some(theme.reasoning_fg));
        assert_eq!(style.bg, Some(theme.reasoning_bg));
    }

    #[test]
    fn user_label_is_bold_accent_normal() {
        let theme = Theme::dark();
        let style = theme.user_label();
        assert_eq!(style.fg, Some(theme.accent));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn asst_label_is_bold_text_secondary_normal() {
        let theme = Theme::light();
        let style = theme.asst_label();
        assert_eq!(style.fg, Some(theme.text_secondary));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn theme_is_copyable_normal() {
        // The Copy bound matters because Theme is held in App and passed
        // by value into render functions every frame. If a refactor adds
        // a String field this test breaks at compile time.
        let t = Theme::dark();
        let copy = t;
        let _both_usable = (t.accent, copy.accent);
    }
}
