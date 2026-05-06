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
