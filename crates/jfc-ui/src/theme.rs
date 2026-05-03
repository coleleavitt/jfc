use gpui::{Hsla, Rgba};

pub struct Theme {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub surface_code: Hsla,
    pub border: Hsla,
    pub border_focus: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub accent_muted: Hsla,
    pub user_bubble: Hsla,
    pub assistant_bubble: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub error: Hsla,
    pub diff_added_bg: Hsla,
    pub diff_added_text: Hsla,
    pub diff_removed_bg: Hsla,
    pub diff_removed_text: Hsla,
    pub code_keyword: Hsla,
    pub code_string: Hsla,
    pub code_comment: Hsla,
    pub code_function: Hsla,
    pub overlay_bg: Hsla,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            background: hex_to_hsla(0x1a1a2e),
            surface: hex_to_hsla(0x16213e),
            surface_raised: hex_to_hsla(0x1e293b),
            surface_code: hex_to_hsla(0x0f172a),
            border: hex_to_hsla(0x334155),
            border_focus: hex_to_hsla(0xf97316),
            text_primary: hex_to_hsla(0xf1f5f9),
            text_secondary: hex_to_hsla(0x94a3b8),
            text_muted: hex_to_hsla(0x64748b),
            accent: hex_to_hsla(0xf97316),
            accent_muted: hex_to_hsla(0x431407),
            user_bubble: hex_to_hsla(0x1e3a5f),
            assistant_bubble: hex_to_hsla(0x1e293b),
            success: hex_to_hsla(0x22c55e),
            warning: hex_to_hsla(0xeab308),
            error: hex_to_hsla(0xef4444),
            diff_added_bg: hex_to_hsla(0x052e16),
            diff_added_text: hex_to_hsla(0x86efac),
            diff_removed_bg: hex_to_hsla(0x450a0a),
            diff_removed_text: hex_to_hsla(0xfca5a5),
            code_keyword: hex_to_hsla(0xc084fc),
            code_string: hex_to_hsla(0x86efac),
            code_comment: hex_to_hsla(0x6b7280),
            code_function: hex_to_hsla(0x60a5fa),
            overlay_bg: hex_to_hsla_alpha(0x000000, 0.6),
        }
    }
}

fn hex_to_hsla(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Rgba { r, g, b, a: 1.0 }.into()
}

fn hex_to_hsla_alpha(hex: u32, alpha: f32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Rgba { r, g, b, a: alpha }.into()
}
