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
            bg: Color::Rgb(30, 30, 46),       // Base
            surface: Color::Rgb(49, 50, 68),   // Surface0
            surface_raised: Color::Rgb(69, 71, 90), // Surface1
            border: Color::Rgb(88, 91, 112),   // Surface2
            text_primary: Color::Rgb(205, 214, 244),  // Text
            text_secondary: Color::Rgb(186, 194, 222), // Subtext1
            text_muted: Color::Rgb(127, 132, 156),     // Overlay1
            accent: Color::Rgb(137, 180, 250),         // Blue
            success: Color::Rgb(166, 227, 161),        // Green
            warning: Color::Rgb(249, 226, 175),        // Yellow
            error: Color::Rgb(243, 139, 168),          // Red
            user_bubble_bg: Color::Rgb(49, 50, 68),
            asst_bubble_bg: Color::Rgb(30, 30, 46),
            code_bg: Color::Rgb(24, 24, 37),           // Mantle
            code_fg: Color::Rgb(205, 214, 244),
            code_string: Color::Rgb(166, 227, 161),
            code_keyword: Color::Rgb(203, 166, 247),   // Mauve
            code_comment: Color::Rgb(127, 132, 156),
            code_number: Color::Rgb(250, 179, 135),    // Peach
            reasoning_bg: Color::Rgb(49, 50, 68),
            reasoning_fg: Color::Rgb(166, 173, 200),
        }
    }

    /// Look up a theme by name. Returns None for unknown names so
    /// the caller can show an error toast.
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            "solarized" | "solarized-dark" => Some(Self::solarized_dark()),
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin()),
            _ => None,
        }
    }

    pub fn available_names() -> &'static [&'static str] {
        &["dark", "light", "solarized", "catppuccin"]
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
        assert!(Theme::by_name("DARK").is_none(), "case-sensitive lookup");
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
