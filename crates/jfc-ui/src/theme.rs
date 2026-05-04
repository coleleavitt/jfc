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
