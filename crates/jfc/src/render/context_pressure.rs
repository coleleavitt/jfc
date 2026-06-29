use ratatui::{
    style::Style,
    text::{Line, Span},
};

use super::visual::fmt_number;
use crate::theme::Theme;

pub(super) fn nudge_line(
    nudge: jfc_engine::context_accounting::ContextPressureNudge,
    theme: Theme,
) -> Line<'static> {
    let color = match nudge.kind {
        jfc_engine::context_accounting::ContextPressureNudgeKind::ChannelOne => theme.warning,
        jfc_engine::context_accounting::ContextPressureNudgeKind::ChannelTwo => theme.error,
        jfc_engine::context_accounting::ContextPressureNudgeKind::Emergency => theme.error,
    };
    let short_kind = match nudge.kind {
        jfc_engine::context_accounting::ContextPressureNudgeKind::ChannelOne => "ch1",
        jfc_engine::context_accounting::ContextPressureNudgeKind::ChannelTwo => "ch2",
        jfc_engine::context_accounting::ContextPressureNudgeKind::Emergency => "emergency",
    };
    Line::from(vec![
        Span::styled("ctx_reduce ", Style::default().fg(theme.text_muted)),
        Span::styled(short_kind, Style::default().fg(color)),
        Span::styled(
            format!(" drop {}", fmt_number(nudge.reclaim_floor_tokens)),
            Style::default().fg(theme.text_secondary),
        ),
    ])
}
