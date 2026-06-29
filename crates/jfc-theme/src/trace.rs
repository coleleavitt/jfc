use crate::Theme;
use ratatui::style::Color;

pub(crate) fn record_theme_constructor(name: &'static str) {
    linkscope::record_items("theme.constructor", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "theme.constructor",
        [linkscope::TraceField::text("canonical", name)],
    );
}

pub(crate) fn record_theme_catalog(label: &'static str, names: usize, aliases: usize) {
    linkscope::record_items(label, usize_to_u64_saturating(names));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("names", usize_to_u64_saturating(names)),
            linkscope::TraceField::count("aliases", usize_to_u64_saturating(aliases)),
        ],
    );
}

pub(crate) fn record_cached_palette(theme: &Theme) {
    let rgb_slots = [
        theme.bg,
        theme.surface,
        theme.surface_raised,
        theme.border,
        theme.text_primary,
        theme.text_secondary,
        theme.text_muted,
        theme.accent,
        theme.success,
        theme.warning,
        theme.error,
        theme.user_bubble_bg,
        theme.asst_bubble_bg,
        theme.code_bg,
        theme.code_fg,
        theme.code_string,
        theme.code_keyword,
        theme.code_comment,
        theme.code_number,
        theme.reasoning_bg,
        theme.reasoning_fg,
        theme.accent_secondary,
        theme.cost_signal,
    ]
    .into_iter()
    .filter(|color| matches!(color, Color::Rgb(_, _, _)))
    .count();
    linkscope::record_items(
        "theme.palette.rgb_slots",
        usize_to_u64_saturating(rgb_slots),
    );
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "theme.palette.shape",
        [
            linkscope::TraceField::count("rgb_slots", usize_to_u64_saturating(rgb_slots)),
            linkscope::TraceField::count("semantic_slots", 23),
            linkscope::TraceField::count("cached_styles", 9),
        ],
    );
}

pub(crate) fn record_style_accessor(name: &'static str) {
    linkscope::record_items("theme.style.accessor", 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "theme.style.accessor",
        [linkscope::TraceField::text("name", name)],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
