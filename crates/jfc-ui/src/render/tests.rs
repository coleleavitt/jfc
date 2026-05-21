use super::*;
#[cfg(test)]
mod task_view_tests {
    use super::*;
    use std::collections::HashSet;

    /// Flatten a `Line` to plain string for substring assertions —
    /// markdown::to_lines produces multi-span lines (syntect highlighting),
    /// so we can't assert on a single span's `.content`.
    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn markdown_renders_xml_stripped_in_task_view_normal() {
        // The bug: subagent view was rendering `<tool_call>{...}</tool_call>`
        // as literal angle brackets. Routing through `markdown::to_lines`
        // (which calls `strip_inline_tool_xml`) replaces them with the
        // `⟪tool_call⟫` marker so users see structure, not raw XML.
        let theme = Theme::dark();
        let messages = vec!["Before <tool_call>{\"name\":\"foo\"}</tool_call> after".to_string()];
        let expanded = HashSet::new();
        let lines = task_view_body_lines(&messages, &expanded, &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            !joined.contains("<tool_call>"),
            "literal <tool_call> should be stripped, got: {joined}"
        );
        assert!(
            !joined.contains("</tool_call>"),
            "literal </tool_call> should be stripped, got: {joined}"
        );
        assert!(
            joined.contains("⟪tool_call⟫"),
            "expected the strip marker in output, got: {joined}"
        );
    }

    #[test]
    fn long_message_collapses_normal() {
        // 100-line entry → preview (5 lines) + 1 muted footer row when
        // collapsed; full content when the index is in `expanded`.
        let theme = Theme::dark();
        let body: String = (1..=100)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![body];

        let collapsed_lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let collapsed_text: String = collapsed_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            collapsed_text.contains("press o to expand"),
            "collapsed view should show expansion hint, got: {collapsed_text}"
        );
        assert!(
            collapsed_text.contains("line 1"),
            "collapsed view should include first preview line"
        );
        assert!(
            collapsed_text.contains("line 5"),
            "collapsed view should include 5th preview line"
        );
        assert!(
            !collapsed_text.contains("line 50"),
            "collapsed view should hide line 50, got: {collapsed_text}"
        );
        assert!(
            !collapsed_text.contains("line 100"),
            "collapsed view should hide tail content"
        );

        let mut expanded = HashSet::new();
        expanded.insert(0);
        let expanded_lines = task_view_body_lines(&messages, &expanded, &theme, 80, false);
        let expanded_text: String = expanded_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            expanded_text.contains("line 1"),
            "expanded view should include first line"
        );
        assert!(
            expanded_text.contains("line 50"),
            "expanded view should include middle line"
        );
        assert!(
            expanded_text.contains("line 100"),
            "expanded view should include last line"
        );
        assert!(
            !expanded_text.contains("press o to expand"),
            "expanded view should not show the collapse hint"
        );
    }

    #[test]
    fn short_message_passes_through_untouched_robust() {
        // Below the line/byte threshold → no preview truncation, no
        // expansion footer, just whatever markdown::to_lines produced.
        let theme = Theme::dark();
        let messages = vec!["just one short line".to_string()];
        let lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("just one short line"));
        assert!(!joined.contains("press o to expand"));
    }

    #[test]
    fn large_byte_payload_collapses_even_without_many_lines_robust() {
        // A single >5 KB line still trips the byte threshold even though
        // the line count is 1 — guards against unwrapped JSON dumps.
        let theme = Theme::dark();
        let big = "x".repeat(TASK_VIEW_COLLAPSE_BYTES + 100);
        let messages = vec![big];
        let lines = task_view_body_lines(&messages, &HashSet::new(), &theme, 80, false);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("press o to expand"),
            "byte-threshold trip should show expansion hint"
        );
    }
}

#[cfg(test)]
mod mcp_tests {
    use super::*;

    #[test]
    fn mcp_status_color_connected_is_success_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Connected, t), t.success);
    }

    #[test]
    fn mcp_status_color_disabled_is_muted_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Disabled, t), t.text_muted);
    }

    #[test]
    fn mcp_status_color_error_is_error_normal() {
        let t = Theme::dark();
        assert_eq!(mcp_status_color(McpStatus::Error, t), t.error);
    }
}

#[cfg(test)]
mod render_helpers_tests {
    use super::*;
    use crate::theme::Theme;

    fn t() -> Theme {
        Theme::dark()
    }

    // ─── pulse_color ───────────────────────────────────────────────
    // Normal: t=0 returns c1, t=1 returns c2, t=0.5 returns midpoint.
    #[test]
    fn pulse_color_endpoints_normal() {
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 0.0), Color::Rgb(0, 0, 0));
        assert_eq!(pulse_color(c1, c2, 1.0), Color::Rgb(200, 100, 50));
        // Midpoint blend.
        if let Color::Rgb(r, g, b) = pulse_color(c1, c2, 0.5) {
            assert!((r as i32 - 100).abs() <= 1);
            assert!((g as i32 - 50).abs() <= 1);
            assert!((b as i32 - 25).abs() <= 1);
        } else {
            panic!("expected Rgb");
        }
    }

    // Robust: ANSI-named colors (no RGB triple) fall back to the
    // start color since blending isn't well-defined.
    #[test]
    fn pulse_color_named_falls_back_robust() {
        assert_eq!(pulse_color(Color::Red, Color::Blue, 0.5), Color::Red);
    }

    // Robust: `t` outside [0,1] gets clamped via interpolate_rgb.
    #[test]
    fn pulse_color_clamps_t_robust() {
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(255, 255, 255);
        // t = -1 should clamp to 0 → c1
        assert_eq!(pulse_color(c1, c2, -1.0), Color::Rgb(0, 0, 0));
        // t = 5 should clamp to 1 → c2
        assert_eq!(pulse_color(c1, c2, 5.0), Color::Rgb(255, 255, 255));
    }

    // ─── tail_truncate ─────────────────────────────────────────────
    // Normal: short input fits, returns unchanged.
    #[test]
    fn tail_truncate_short_unchanged_normal() {
        assert_eq!(tail_truncate("hello", 10), "hello");
    }

    // Normal: long input keeps the tail with a `…/` prefix.
    #[test]
    fn tail_truncate_keeps_tail_normal() {
        let s = "/home/cole/RustProjects/active/jfc";
        let truncated = tail_truncate(s, 12);
        assert!(truncated.starts_with("…/"));
        assert!(truncated.ends_with("jfc"));
        assert!(truncated.chars().count() <= 12);
    }

    // Robust: max=0 returns empty (no panic).
    #[test]
    fn tail_truncate_zero_max_robust() {
        assert_eq!(tail_truncate("anything", 0), "");
    }

    // Robust: max < 4 (too narrow for "…/") falls back to head truncation.
    #[test]
    fn tail_truncate_narrow_falls_back_robust() {
        let s = "long/path/here";
        let result = tail_truncate(s, 3);
        // Should not panic, should be 3 cells or fewer.
        assert!(result.chars().count() <= 3);
    }

    // ─── wrap_text_to_width ────────────────────────────────────────
    // Normal: text shorter than width returns one line.
    #[test]
    fn wrap_text_short_one_line_normal() {
        let lines = wrap_text_to_width("hello world", 30);
        assert_eq!(lines, vec!["hello world"]);
    }

    // Normal: long text wraps at word boundaries.
    #[test]
    fn wrap_text_word_wraps_normal() {
        let lines = wrap_text_to_width("one two three four five", 10);
        // Each line should be ≤ 10 chars, breaking at spaces.
        for l in &lines {
            assert!(l.chars().count() <= 10, "line too long: {l:?}");
            assert!(!l.trim().is_empty(), "blank line in middle: {lines:?}");
        }
    }

    // Robust: a single word longer than width gets truncated with `…`
    // so it doesn't bleed off the edge.
    #[test]
    fn wrap_text_oversize_word_truncates_robust() {
        let lines = wrap_text_to_width("supercalifragilistic", 8);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].chars().count() <= 8);
        assert!(lines[0].ends_with('…') || lines[0].chars().count() < 8);
    }

    // Robust: width=0 returns one empty line, no panic.
    #[test]
    fn wrap_text_zero_width_robust() {
        let lines = wrap_text_to_width("anything", 0);
        assert_eq!(lines, vec![String::new()]);
    }

    // Robust: empty input returns one empty line so callers can
    // unconditionally `.push(Line::from(row))`.
    #[test]
    fn wrap_text_empty_input_robust() {
        let lines = wrap_text_to_width("", 20);
        assert_eq!(lines, vec![String::new()]);
    }

    // (path_color tests live alongside the `message_view::path_color`
    // helper so they can use the in-module function directly without
    // needing a re-export.)

    // ─── parse_prompt_mode ─────────────────────────────────────────
    // Normal: each named preset parses to its variant.
    #[test]
    fn parse_prompt_mode_named_presets_normal() {
        assert!(matches!(parse_prompt_mode(":comet"), PromptMode::Comet));
        assert!(matches!(parse_prompt_mode(":moon"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":dice"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":notes"), PromptMode::Notes));
        assert!(matches!(
            parse_prompt_mode(":hourglass"),
            PromptMode::Hourglass
        ));
        assert!(matches!(parse_prompt_mode(":atom"), PromptMode::Atom));
    }

    // Normal: aliases resolve to the same variant.
    #[test]
    fn parse_prompt_mode_aliases_normal() {
        assert!(matches!(parse_prompt_mode(":moons"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":die"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":music"), PromptMode::Notes));
        assert!(matches!(parse_prompt_mode(":time"), PromptMode::Hourglass));
    }

    // Normal: bare char becomes Static.
    #[test]
    fn parse_prompt_mode_bare_char_static_normal() {
        if let PromptMode::Static(s) = parse_prompt_mode("⌬") {
            assert_eq!(s, "⌬");
        } else {
            panic!("expected Static");
        }
    }

    // Robust: empty input falls through to default Comet.
    #[test]
    fn parse_prompt_mode_empty_default_robust() {
        assert!(matches!(parse_prompt_mode(""), PromptMode::Comet));
    }

    // Robust: a too-long literal (>2 chars) falls through to default.
    #[test]
    fn parse_prompt_mode_long_literal_default_robust() {
        assert!(matches!(parse_prompt_mode("abcd"), PromptMode::Comet));
    }

    // ─── prompt_mode_frame ─────────────────────────────────────────
    // Normal: comet returns the comet glyph regardless of streaming/ms.
    #[test]
    fn prompt_mode_frame_comet_constant_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, false, 0), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, true, 1234), "☄");
    }

    // Normal: moon idle is full moon, streaming cycles through 4 phases.
    #[test]
    fn prompt_mode_frame_moon_cycle_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, false, 0), "●");
        // Frame at ms=0 is FRAMES[0] = "○".
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, true, 0), "○");
        // Frame at ms=250 is FRAMES[1] = "◐".
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, true, 250), "◐");
    }

    // Normal: dice idle stays at ⚀, streaming shuffles.
    #[test]
    fn prompt_mode_frame_dice_idle_normal() {
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, false, 0), "⚀");
        // Streaming at ms=0 is the first face.
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, true, 0), "⚀");
    }

    // Robust: hourglass flips every 800ms.
    #[test]
    fn prompt_mode_frame_hourglass_flip_robust() {
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 0), "⌛");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 800), "⌚");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 1600), "⌛");
    }
}

// =====================================================================

#[cfg(test)]
mod pure_helper_tests {
    use super::input_box::{input_soft_wrapped_lines, input_visual_line_count};
    use super::model_picker::{provider_color, provider_label};
    use super::status::{context_gauge_label, effort_status_badge};
    use super::*;
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use ratatui_textarea::TextArea;

    /// Stub provider for `App::new` — none of the helpers under test
    /// dispatch through it, but `App::new` requires a `dyn Provider`.
    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            #[allow(dead_code)] messages: Vec<ProviderMessage>,
            #[allow(dead_code)] options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for TestProvider {}

    fn fake_app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    // --- pulse_color -------------------------------------------------

    #[test]
    fn pulse_color_t_zero_returns_first_normal() {
        // t=0 should give back exactly c1's RGB.
        let c1 = Color::Rgb(10, 20, 30);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 0.0), Color::Rgb(10, 20, 30));
    }

    #[test]
    fn pulse_color_t_one_returns_second_normal() {
        // t=1 should give back c2's RGB exactly.
        let c1 = Color::Rgb(10, 20, 30);
        let c2 = Color::Rgb(200, 100, 50);
        assert_eq!(pulse_color(c1, c2, 1.0), Color::Rgb(200, 100, 50));
    }

    #[test]
    fn pulse_color_midpoint_blends_normal() {
        // t=0.5 should land between the endpoints in each channel.
        let c1 = Color::Rgb(0, 0, 0);
        let c2 = Color::Rgb(100, 200, 50);
        match pulse_color(c1, c2, 0.5) {
            Color::Rgb(r, g, b) => {
                assert!((45..=55).contains(&r), "midpoint r should be ~50, got {r}");
                assert!((95..=105).contains(&g), "midpoint g should be ~100");
                assert!((20..=30).contains(&b), "midpoint b should be ~25");
            }
            other => panic!("expected Rgb, got {other:?}"),
        }
    }

    #[test]
    fn pulse_color_named_color_falls_back_robust() {
        // Non-RGB endpoints can't interpolate — return c1 unchanged so the
        // pulse just freezes on that color rather than panicking.
        let c1 = Color::Red;
        let c2 = Color::Rgb(50, 50, 50);
        assert_eq!(pulse_color(c1, c2, 0.5), Color::Red);
    }

    #[test]
    fn pulse_color_named_second_falls_back_robust() {
        // Symmetric: c2 named, c1 RGB → also falls back to c1.
        let c1 = Color::Rgb(10, 10, 10);
        let c2 = Color::Blue;
        assert_eq!(pulse_color(c1, c2, 0.5), Color::Rgb(10, 10, 10));
    }

    // --- pulse_color_pub (public wrapper) ----------------------------

    #[test]
    fn pulse_color_pub_matches_private_normal() {
        let c1 = Color::Rgb(20, 40, 60);
        let c2 = Color::Rgb(100, 120, 140);
        assert_eq!(pulse_color_pub(c1, c2, 0.25), pulse_color(c1, c2, 0.25),);
    }

    // --- tail_truncate -----------------------------------------------

    #[test]
    fn tail_truncate_short_passes_through_normal() {
        // Below the cap → return unchanged.
        assert_eq!(tail_truncate("hello", 10), "hello");
    }

    #[test]
    fn tail_truncate_long_keeps_tail_normal() {
        // Long path: drop the head, prepend `…/`, keep the meaningful end.
        let result = tail_truncate("/home/cole/RustProjects/active/jfc", 12);
        assert!(result.starts_with("…/"), "got {result:?}");
        assert!(result.contains("jfc"), "got {result:?}");
        // Must respect the requested width (chars, not bytes).
        assert!(result.chars().count() <= 12, "got {result:?}");
    }

    #[test]
    fn tail_truncate_zero_max_returns_empty_robust() {
        // max=0 → empty string, not a panic.
        assert_eq!(tail_truncate("anything", 0), String::new());
    }

    #[test]
    fn tail_truncate_too_narrow_falls_back_to_head_truncate_robust() {
        // max < 4 leaves no room for the `…/` indicator → head-truncate
        // (matching truncate_str behavior with the trailing ellipsis).
        let result = tail_truncate("/foo/bar/baz", 3);
        assert_eq!(result.chars().count(), 3);
        assert!(result.ends_with('…'), "got {result:?}");
    }

    #[test]
    fn tail_truncate_unicode_chars_robust() {
        // Width is in chars, not bytes — multi-byte glyphs shouldn't break it.
        let s = "日本語/プロジェクト/foo";
        let result = tail_truncate(s, 8);
        assert!(result.chars().count() <= 8);
        assert!(result.starts_with("…/"), "got {result:?}");
    }

    // --- wrap_text_to_width ------------------------------------------

    #[test]
    fn wrap_text_short_returns_one_row_normal() {
        let rows = wrap_text_to_width("hello world", 80);
        assert_eq!(rows, vec!["hello world".to_string()]);
    }

    #[test]
    fn wrap_text_breaks_on_whitespace_normal() {
        // Each row is a complete fragment, broken on whitespace.
        let rows = wrap_text_to_width("alpha beta gamma delta", 12);
        for row in &rows {
            assert!(row.chars().count() <= 12, "row {row:?} exceeds width 12");
        }
        assert!(rows.len() >= 2, "should wrap into at least 2 rows");
    }

    #[test]
    fn wrap_text_zero_width_returns_single_empty_robust() {
        // width=0 short-circuits: one empty row so callers can `.push`.
        let rows = wrap_text_to_width("anything here", 0);
        assert_eq!(rows, vec![String::new()]);
    }

    #[test]
    fn wrap_text_long_word_hard_truncates_robust() {
        // A single word longer than width can't break on whitespace —
        // hard-truncate that word with `…` so it doesn't overflow.
        let rows = wrap_text_to_width("supercalifragilisticexpialidocious", 10);
        assert!(rows.iter().any(|r| r.ends_with('…')), "rows: {rows:?}");
        for r in &rows {
            assert!(r.chars().count() <= 10, "row {r:?} exceeded width");
        }
    }

    #[test]
    fn wrap_text_empty_input_returns_empty_row_robust() {
        // Empty input → at least one row so `out.push(Line::from(...))`
        // always has something to render.
        let rows = wrap_text_to_width("", 20);
        assert_eq!(rows, vec![String::new()]);
    }

    // --- parse_prompt_mode -------------------------------------------

    #[test]
    fn parse_prompt_mode_named_presets_normal() {
        assert!(matches!(parse_prompt_mode(":comet"), PromptMode::Comet));
        assert!(matches!(parse_prompt_mode(":moon"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":moons"), PromptMode::Moon));
        assert!(matches!(parse_prompt_mode(":dice"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":die"), PromptMode::Dice));
        assert!(matches!(parse_prompt_mode(":notes"), PromptMode::Notes));
        assert!(matches!(parse_prompt_mode(":music"), PromptMode::Notes));
        assert!(matches!(
            parse_prompt_mode(":hourglass"),
            PromptMode::Hourglass
        ));
        assert!(matches!(parse_prompt_mode(":time"), PromptMode::Hourglass));
        assert!(matches!(parse_prompt_mode(":atom"), PromptMode::Atom));
    }

    #[test]
    fn parse_prompt_mode_static_single_char_normal() {
        match parse_prompt_mode("⌬") {
            PromptMode::Static(s) => assert_eq!(s, "⌬"),
            other => panic!("expected Static, got {other:?}"),
        }
    }

    #[test]
    fn parse_prompt_mode_static_two_chars_normal() {
        match parse_prompt_mode("ab") {
            PromptMode::Static(s) => assert_eq!(s, "ab"),
            other => panic!("expected Static, got {other:?}"),
        }
    }

    #[test]
    fn parse_prompt_mode_long_input_falls_back_to_comet_robust() {
        // 3+ chars and not a named preset → fall back to Comet (default).
        assert!(matches!(parse_prompt_mode("xyz123"), PromptMode::Comet));
    }

    #[test]
    fn parse_prompt_mode_empty_falls_back_to_comet_robust() {
        // Empty string → comet (no Static branch since chars=0).
        assert!(matches!(parse_prompt_mode(""), PromptMode::Comet));
    }

    #[test]
    fn parse_prompt_mode_trims_whitespace_robust() {
        // Whitespace around a preset token must not break the match.
        assert!(matches!(parse_prompt_mode("  :moon  "), PromptMode::Moon));
    }

    // --- prompt_mode_frame -------------------------------------------

    #[test]
    fn prompt_mode_frame_static_glyphs_normal() {
        // Comet/Atom always return their static glyph regardless of state.
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, false, 0), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Comet, true, 9999), "☄");
        assert_eq!(prompt_mode_frame(&PromptMode::Atom, true, 0), "⚛");
    }

    #[test]
    fn prompt_mode_frame_idle_states_settle_normal() {
        // Non-streaming → each mode lands on its rest glyph.
        assert_eq!(prompt_mode_frame(&PromptMode::Moon, false, 0), "●");
        assert_eq!(prompt_mode_frame(&PromptMode::Dice, false, 0), "⚀");
        assert_eq!(prompt_mode_frame(&PromptMode::Notes, false, 0), "♪");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, false, 0), "⌛");
    }

    #[test]
    fn prompt_mode_frame_streaming_cycles_moon_normal() {
        // 4 distinct frames at 250ms cadence — sample several.
        let f0 = prompt_mode_frame(&PromptMode::Moon, true, 0);
        let f1 = prompt_mode_frame(&PromptMode::Moon, true, 250);
        let f2 = prompt_mode_frame(&PromptMode::Moon, true, 500);
        let f3 = prompt_mode_frame(&PromptMode::Moon, true, 750);
        assert!(
            [f0, f1, f2, f3]
                .iter()
                .all(|g| ["○", "◐", "●", "◑"].contains(g))
        );
    }

    #[test]
    fn prompt_mode_frame_hourglass_alternates_robust() {
        // 800ms flip cadence — at 0 and 1600ms, full glass; at 800ms,
        // empty.
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 0), "⌛");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 800), "⌚");
        assert_eq!(prompt_mode_frame(&PromptMode::Hourglass, true, 1600), "⌛");
    }

    #[test]
    fn prompt_mode_frame_static_branch_returns_empty_sentinel_robust() {
        // Static is rendered separately by the input renderer; this fn
        // returns "" as a sentinel for that branch.
        assert_eq!(
            prompt_mode_frame(&PromptMode::Static("X".to_string()), true, 0),
            ""
        );
    }

    // --- comet_config_from_state -------------------------------------

    #[test]
    fn comet_config_idle_uses_accent_color_normal() {
        // Default idle app: head color = theme accent, lap_ms = 3500.
        let app = fake_app();
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert_eq!(cfg.count, 1);
        assert_eq!(cfg.head, app.theme.accent);
        assert_eq!(cfg.base, app.theme.border);
        assert!(!cfg.reverse_base, "idle should not reverse base");
    }

    #[test]
    fn comet_config_bash_mode_uses_warning_normal() {
        // Input starting with `!` → bash-mode → warning color.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["!ls".to_string()]);
        let cfg = comet_config_from_state(&app, app.theme, 2);
        assert_eq!(cfg.head, app.theme.warning);
        assert!(
            !cfg.reverse_base,
            "bash mode wins over tool-running, no reverse"
        );
    }

    #[test]
    fn comet_config_running_tool_reverses_robust() {
        // A running tool in the latest message → reverse_base = true,
        // warning color.
        let mut app = fake_app();
        let tool = ToolCall {
            id: "t1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Running,
            input: ToolInput::Bash {
                command: "sleep 1".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            parts: vec![MessagePart::Tool(tool)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        });
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert_eq!(cfg.head, app.theme.warning);
        assert!(cfg.reverse_base, "tool-running should reverse direction");
    }

    #[test]
    fn comet_config_trail_clamped_robust() {
        // Trail is clamped to 2..=12 even with extreme env values.
        // Default (no env) is 6.
        let app = fake_app();
        // Make sure no env var pollutes this test.
        unsafe {
            std::env::remove_var("JFC_BORDER_COMET_TRAIL");
        }
        let cfg = comet_config_from_state(&app, app.theme, 1);
        assert!((2..=12).contains(&cfg.trail_len));
    }

    // --- perimeter_cells ---------------------------------------------

    #[test]
    fn perimeter_cells_3x3_rect_normal() {
        // 3x3 rect: 8 perimeter cells (no interior).
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
        });
        assert_eq!(cells.len(), 8);
        // First cell = top-left; last cell = (0,1) (left edge bottom-up
        // skipping bottom-left already added).
        assert_eq!(cells[0], (0, 0));
    }

    #[test]
    fn perimeter_cells_walks_clockwise_from_topleft_normal() {
        // 4x4: top edge L→R, right top→bottom, bottom R→L, left bottom→top.
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        });
        // 4*4 = 16 total cells; perimeter = 4*4 - 2*2 = 12.
        assert_eq!(cells.len(), 12);
        assert_eq!(cells[0], (0, 0));
        // After top edge (4 cells), we should be on the right edge.
        assert_eq!(cells[4], (3, 1));
    }

    #[test]
    fn perimeter_cells_too_small_returns_empty_robust() {
        // Width or height < 2 → empty (no meaningful perimeter).
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 5,
        });
        assert!(cells.is_empty());
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 1,
        });
        assert!(cells.is_empty());
    }

    #[test]
    fn perimeter_cells_offset_rect_robust() {
        // Non-zero origin: the cells should reflect the absolute coords.
        let cells = perimeter_cells(Rect {
            x: 10,
            y: 20,
            width: 3,
            height: 3,
        });
        assert!(cells.contains(&(10, 20)));
        assert!(cells.contains(&(12, 22)));
        // Interior cell (11, 21) must NOT be in the perimeter.
        assert!(!cells.contains(&(11, 21)));
    }

    #[test]
    fn perimeter_cells_2x2_no_duplicates_robust() {
        // The smallest valid rect: 2x2. Each corner is one cell;
        // none should be duplicated.
        let cells = perimeter_cells(Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        });
        let mut sorted = cells.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), cells.len(), "duplicates: {cells:?}");
        assert_eq!(cells.len(), 4);
    }

    // --- input_visual_line_count + input_soft_wrapped_lines ----------

    #[test]
    fn input_visual_line_count_short_text_one_line_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abc".to_string()]);
        assert_eq!(input_visual_line_count(&app, 80), 1);
    }

    #[test]
    fn input_visual_line_count_wraps_long_line_normal() {
        // A 12-char line at width 5 = 3 visual rows (5/5/2).
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abcdefghijkl".to_string()]);
        assert_eq!(input_visual_line_count(&app, 5), 3);
    }

    #[test]
    fn input_visual_line_count_empty_returns_one_robust() {
        // Empty input still renders the placeholder — count is 1, not 0.
        let app = fake_app();
        assert_eq!(input_visual_line_count(&app, 80), 1);
    }

    #[test]
    fn input_soft_wrapped_cursor_at_start_normal() {
        // Cursor at (0, 0) → visual_cursor_row=0, col=0.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["hello".to_string()]);
        let (lines, row, col) = input_soft_wrapped_lines(&app, 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn input_soft_wrapped_cursor_after_wrap_robust() {
        // 10-char line at width=5 → wraps to 2 visual rows. Cursor at
        // logical col 8 → visual row 1 col 3.
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["abcdefghij".to_string()]);
        app.textarea
            .move_cursor(ratatui_textarea::CursorMove::Jump(0, 8));
        let (lines, row, col) = input_soft_wrapped_lines(&app, 5);
        assert_eq!(lines.len(), 2);
        assert_eq!(row, 1);
        assert_eq!(col, 3);
    }

    #[test]
    fn input_soft_wrapped_empty_uses_placeholder_robust() {
        // All-empty input → placeholder string is the only line.
        let app = fake_app();
        let (lines, row, col) = input_soft_wrapped_lines(&app, 80);
        assert_eq!(lines, vec!["send a message…".to_string()]);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    // --- input_line_to_spans -----------------------------------------

    #[test]
    fn input_line_spans_empty_returns_one_raw_normal() {
        let t = Theme::dark();
        let spans = input_line_to_spans("", t, 0.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "");
    }

    #[test]
    fn input_line_spans_slash_command_colors_each_char_normal() {
        // Every char of the slash token gets its own colored span (rainbow).
        let t = Theme::dark();
        let spans = input_line_to_spans("/help", t, 0.0);
        // The leading `/` plus each char → 5 styled spans with no rest.
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn input_line_spans_slash_with_args_keeps_rest_normal() {
        // After the slash token, the rest goes through highlight_mentions_in.
        let t = Theme::dark();
        let spans = input_line_to_spans("/cmd arg1 @user", t, 0.0);
        // At minimum one span per char in `/cmd` plus one or more for the rest.
        assert!(spans.len() > 4);
    }

    #[test]
    fn input_line_spans_plain_text_falls_through_to_mentions_robust() {
        // No slash → just `highlight_mentions_in` output.
        let t = Theme::dark();
        let spans = input_line_to_spans("hello world", t, 0.0);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hello world");
    }

    #[test]
    fn input_line_spans_leading_whitespace_preserved_robust() {
        // Indent before a slash command must be preserved verbatim.
        let t = Theme::dark();
        let spans = input_line_to_spans("   /help", t, 0.0);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "   /help");
    }

    // --- highlight_mentions_in ---------------------------------------

    #[test]
    fn highlight_mentions_no_at_returns_one_span_normal() {
        let t = Theme::dark();
        let spans = highlight_mentions_in("just plain text", t, 0.0);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn highlight_mentions_at_token_gets_per_char_spans_normal() {
        // `@cole` at the start → 5 styled spans (one per char). With
        // empty prefix there's no leading text span.
        let t = Theme::dark();
        let spans = highlight_mentions_in("@cole", t, 0.0);
        assert_eq!(spans.len(), 5);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "@cole");
    }

    #[test]
    fn highlight_mentions_only_after_whitespace_robust() {
        // Only `@` after whitespace (or at start) is a mention; mid-word
        // `@` (e.g. email) doesn't trigger.
        let t = Theme::dark();
        let spans = highlight_mentions_in("user@example.com", t, 0.0);
        // Exactly one prefix span (no mention split).
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "user@example.com");
    }

    #[test]
    fn highlight_mentions_text_then_mention_robust() {
        // "hi @cole" → ["hi "] + 5 chars of "@cole" = 6 spans.
        let t = Theme::dark();
        let spans = highlight_mentions_in("hi @cole", t, 0.0);
        assert_eq!(spans.len(), 6);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hi @cole");
    }

    // --- gauge_color (existing private helper) -----------------------

    #[test]
    fn gauge_color_buckets_normal() {
        let t = Theme::dark();
        // 0..60% = success, 60..85% = warning, 85+ = error.
        assert_eq!(gauge_color(0.0, t), t.success);
        assert_eq!(gauge_color(50.0, t), t.success);
        assert_eq!(gauge_color(70.0, t), t.warning);
        assert_eq!(gauge_color(90.0, t), t.error);
    }

    #[test]
    fn gauge_color_boundaries_robust() {
        let t = Theme::dark();
        // 60.0 — exactly on warning boundary.
        assert_eq!(gauge_color(60.0, t), t.warning);
        // 85.0 — exactly on error boundary.
        assert_eq!(gauge_color(85.0, t), t.error);
        // Just below boundaries.
        assert_eq!(gauge_color(59.9, t), t.success);
        assert_eq!(gauge_color(84.9, t), t.warning);
    }

    // --- truncate_str (private) --------------------------------------

    #[test]
    fn truncate_str_short_passes_through_normal() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long_appends_ellipsis_normal() {
        let result = truncate_str("hello world", 5);
        assert!(result.ends_with('…'), "got {result:?}");
        assert_eq!(result.chars().count(), 5);
    }

    #[test]
    fn truncate_str_zero_max_returns_empty_robust() {
        assert_eq!(truncate_str("anything", 0), "");
    }

    #[test]
    fn truncate_str_unicode_counts_chars_not_bytes_robust() {
        // CJK chars are 3 bytes each but 1 column in our model.
        let s = "日本語のテキスト";
        let result = truncate_str(s, 4);
        assert_eq!(result.chars().count(), 4);
        assert!(result.ends_with('…'));
    }

    // --- fmt_number --------------------------------------------------

    #[test]
    fn fmt_number_below_thousand_normal() {
        assert_eq!(fmt_number(0), "0");
        assert_eq!(fmt_number(42), "42");
        assert_eq!(fmt_number(999), "999");
    }

    #[test]
    fn fmt_number_thousands_get_separator_normal() {
        assert_eq!(fmt_number(1_000), "1,000");
        assert_eq!(fmt_number(12_345), "12,345");
        assert_eq!(fmt_number(999_999), "999,999");
    }

    #[test]
    fn fmt_number_millions_get_decimal_robust() {
        assert_eq!(fmt_number(1_000_000), "1.0M");
        assert_eq!(fmt_number(1_500_000), "1.5M");
        assert_eq!(fmt_number(12_345_678), "12.3M");
    }

    // --- context_gauge_label -----------------------------------------

    #[test]
    fn context_gauge_label_format_normal() {
        let label = context_gauge_label(50_000, 200_000, 25);
        assert_eq!(label, " ctx 50k / 200k · 25% ");
    }

    #[test]
    fn context_gauge_label_zero_used_robust() {
        let label = context_gauge_label(0, 200_000, 0);
        assert_eq!(label, " ctx 0k / 200k · 0% ");
    }

    // --- effort_status_badge -----------------------------------------

    #[test]
    fn effort_status_badge_shows_default_when_unpinned_normal() {
        let app = fake_app();
        assert_eq!(effort_status_badge(&app), "effort default".to_string());
    }

    #[test]
    fn effort_status_badge_shows_pinned_level_normal() {
        let mut app = fake_app();
        app.effort_state.set(crate::effort::ReasoningEffort::XHigh);
        assert_eq!(effort_status_badge(&app), "effort xhigh".to_string());
    }

    // --- provider_color / provider_label -----------------------------

    #[test]
    fn provider_color_known_providers_normal() {
        assert_eq!(provider_color("anthropic"), Color::Rgb(204, 120, 50));
        assert_eq!(provider_color("anthropic-oauth"), Color::Rgb(204, 120, 50));
        assert_eq!(provider_color("openwebui"), Color::Rgb(100, 180, 200));
    }

    #[test]
    fn provider_color_unknown_returns_gray_robust() {
        assert_eq!(provider_color("xenu"), Color::Gray);
        assert_eq!(provider_color(""), Color::Gray);
    }

    #[test]
    fn provider_label_known_providers_normal() {
        assert_eq!(provider_label("anthropic"), "API");
        assert_eq!(provider_label("anthropic-oauth"), "OAuth");
        assert_eq!(provider_label("openwebui"), "OpenWebUI");
    }

    #[test]
    fn provider_label_unknown_returns_question_mark_robust() {
        assert_eq!(provider_label("???"), "?");
        assert_eq!(provider_label(""), "?");
    }

    // --- collect_diff_stats ------------------------------------------

    #[test]
    fn collect_diff_stats_empty_app_normal() {
        let app = fake_app();
        let stats = collect_diff_stats(&app);
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.additions, 0);
        assert_eq!(stats.deletions, 0);
        assert!(stats.files.is_empty());
    }

    #[test]
    fn collect_diff_stats_aggregates_diffs_normal() {
        let mut app = fake_app();
        let diff = DiffView {
            file_path: "src/foo.rs".into(),
            hunks: Vec::new(),
            additions: 10,
            deletions: 3,
        };
        let tool = ToolCall {
            id: "t1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            input: ToolInput::Edit {
                file_path: "src/foo.rs".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            parts: vec![MessagePart::Tool(tool)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        });
        let stats = collect_diff_stats(&app);
        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.additions, 10);
        assert_eq!(stats.deletions, 3);
        assert_eq!(stats.files, vec!["src/foo.rs".to_string()]);
    }

    #[test]
    fn collect_diff_stats_dedupes_by_path_robust() {
        // Two diffs for the same file → last entry wins (not summed).
        let mut app = fake_app();
        for (i, (a, d)) in [(5, 1), (10, 3)].into_iter().enumerate() {
            let tool = ToolCall {
                id: crate::ids::ToolId::from(format!("t{i}")),
                kind: ToolKind::Edit,
                status: ToolStatus::Completed,
                input: ToolInput::Edit {
                    file_path: "src/foo.rs".into(),
                    old_string: "".into(),
                    new_string: "".into(),
                    replacement: ReplacementMode::FirstOnly,
                },
                output: ToolOutput::Diff(DiffView {
                    file_path: "src/foo.rs".into(),
                    hunks: Vec::new(),
                    additions: a,
                    deletions: d,
                }),
                display: crate::types::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            };
            app.messages.push(ChatMessage {
                role: Role::Assistant,
                parts: vec![MessagePart::Tool(tool)],
                agent_name: None,
                model_name: None,
                cost_tier: None,
                elapsed: None,
                usage: None,
                queued: false,
                attachments: Vec::new(),
            });
        }
        let stats = collect_diff_stats(&app);
        // De-duped to 1 file, last edit wins → +10/-3.
        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.additions, 10);
        assert_eq!(stats.deletions, 3);
    }

    // --- current_slash_prefix / slash_matches ------------------------

    #[test]
    fn current_slash_prefix_single_token_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/he".to_string()]);
        assert_eq!(current_slash_prefix(&app), Some("/he".to_string()));
    }

    #[test]
    fn current_slash_prefix_with_args_drops_after_space_normal() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/help me".to_string()]);
        assert_eq!(current_slash_prefix(&app), Some("/help".to_string()));
    }

    #[test]
    fn current_slash_prefix_no_slash_returns_none_robust() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["hello".to_string()]);
        assert_eq!(current_slash_prefix(&app), None);
    }

    #[test]
    fn current_slash_prefix_multiline_returns_none_robust() {
        let mut app = fake_app();
        app.textarea = TextArea::from(vec!["/help".to_string(), "extra".to_string()]);
        assert_eq!(current_slash_prefix(&app), None);
    }

    #[test]
    fn slash_matches_filters_prefix_normal() {
        // Filter the static SLASH_COMMANDS list by prefix.
        let matches = slash_matches("/");
        assert!(!matches.is_empty(), "/ should match every command");
    }

    #[test]
    fn slash_matches_no_hits_robust() {
        let matches = slash_matches("/zzz_nonexistent");
        assert!(matches.is_empty());
    }

    // --- ordered_sidebar_sessions ------------------------------------

    #[test]
    fn ordered_sidebar_sessions_empty_app_normal() {
        let app = fake_app();
        // No saved sessions means empty result (the helper just orders
        // app.session_meta which starts empty).
        let sessions = ordered_sidebar_sessions(&app);
        assert!(sessions.is_empty());
    }
}

#[cfg(test)]
mod subagent_counter_tests {
    use super::*;
    use crate::app::BackgroundTask;
    use crate::types::TaskLifecycle;

    fn task_with(tools: u32, in_tok: u64, out_tok: u64) -> BackgroundTask {
        BackgroundTask {
            task_id: "t1".into(),
            description: "research".into(),
            status: TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            completed_at: None,
            summary: None,
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: tools,
            latest_input_tokens: in_tok,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: out_tok,
            model_used: None,
            agent_messages: Vec::new(),
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
        }
    }

    // Normal: <1000 tokens stays raw.
    #[test]
    fn format_token_count_under_thousand_raw_normal() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(1), "1");
        assert_eq!(format_token_count(999), "999");
    }

    // Normal: >=1000 collapses to single-decimal "k".
    #[test]
    fn format_token_count_thousands_normal() {
        assert_eq!(format_token_count(1_000), "1.0k");
        assert_eq!(format_token_count(8_945), "8.9k");
        assert_eq!(format_token_count(89_745), "89.7k");
    }

    // Normal: >=1_000_000 collapses to single-decimal "M".
    #[test]
    fn format_token_count_millions_normal() {
        assert_eq!(format_token_count(1_000_000), "1.0M");
        assert_eq!(format_token_count(1_240_000), "1.2M");
    }

    // Robust: u64::MAX renders without panicking.
    #[test]
    fn format_token_count_u64_max_robust() {
        let _ = format_token_count(u64::MAX);
    }

    // Normal: subagent counters render in v131-style suffix order
    // (tool count, then token count).
    #[test]
    fn format_subagent_counters_full_normal() {
        let bt = task_with(22, 50_000, 39_745);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("22 tools"));
        assert!(s.contains("89.7k tok"));
        assert!(s.starts_with(" · "));
    }

    // Normal: singular form for exactly 1 tool.
    #[test]
    fn format_subagent_counters_singular_tool_normal() {
        let bt = task_with(1, 0, 500);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("1 tool"));
        assert!(!s.contains("1 tools"));
    }

    // Robust: zero tools and zero tokens produces an empty suffix
    // (we suppress the row entirely until the agent has reported
    // something, otherwise the UI flickers " · 0 tools" right after
    // spawn).
    #[test]
    fn format_subagent_counters_empty_when_zero_robust() {
        let bt = task_with(0, 0, 0);
        assert_eq!(format_subagent_counters(&bt), "");
    }

    // Robust: tool count without tokens still renders (and vice versa).
    #[test]
    fn format_subagent_counters_partial_data_robust() {
        let only_tools = task_with(3, 0, 0);
        let s = format_subagent_counters(&only_tools);
        assert!(s.contains("3 tools"));
        assert!(!s.contains("tok"));

        let only_tokens = task_with(0, 1_500, 0);
        let s2 = format_subagent_counters(&only_tokens);
        assert!(s2.contains("1.5k tok"));
        assert!(!s2.contains("tools"));
    }

    // Normal: combined input + cumulative_output sum is what gets
    // formatted (matches v131's `latestInputTokens + cumulativeOutputTokens`).
    #[test]
    fn format_subagent_counters_sums_input_and_output_normal() {
        let bt = task_with(0, 80_000, 9_745);
        let s = format_subagent_counters(&bt);
        assert!(s.contains("89.7k tok"));
    }
}
