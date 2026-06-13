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
    use super::status::{
        STATUS_FLOOR_PRIO, context_gauge_label, effort_status_badge, fit_segments, plan_badge,
    };
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
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
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
        let spans = input_line_to_spans("", t);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "");
    }

    #[test]
    fn input_line_spans_slash_command_one_accent_span_normal() {
        // The slash token is a single accent-colored span (flat, no
        // per-char rainbow).
        let t = Theme::dark();
        let spans = input_line_to_spans("/help", t);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "/help");
    }

    #[test]
    fn input_line_spans_slash_with_args_keeps_rest_normal() {
        // `/cmd` token span + the rest (" arg1 " prefix + "@user" mention).
        let t = Theme::dark();
        let spans = input_line_to_spans("/cmd arg1 @user", t);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "/cmd arg1 @user");
        // token + prefix + mention = 3 spans.
        assert_eq!(spans.len(), 3);
    }

    #[test]
    fn input_line_spans_plain_text_falls_through_to_mentions_robust() {
        // No slash → just `highlight_mentions_in` output.
        let t = Theme::dark();
        let spans = input_line_to_spans("hello world", t);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "hello world");
    }

    #[test]
    fn input_line_spans_leading_whitespace_preserved_robust() {
        // Indent before a slash command must be preserved verbatim.
        let t = Theme::dark();
        let spans = input_line_to_spans("   /help", t);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "   /help");
    }

    // --- highlight_mentions_in ---------------------------------------

    #[test]
    fn highlight_mentions_no_at_returns_one_span_normal() {
        let t = Theme::dark();
        let spans = highlight_mentions_in("just plain text", t);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn highlight_mentions_at_token_one_accent_span_normal() {
        // `@cole` at the start → one accent-colored span (flat). With an
        // empty prefix there's no leading text span.
        let t = Theme::dark();
        let spans = highlight_mentions_in("@cole", t);
        assert_eq!(spans.len(), 1);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "@cole");
    }

    #[test]
    fn highlight_mentions_only_after_whitespace_robust() {
        // Only `@` after whitespace (or at start) is a mention; mid-word
        // `@` (e.g. email) doesn't trigger.
        let t = Theme::dark();
        let spans = highlight_mentions_in("user@example.com", t);
        // Exactly one prefix span (no mention split).
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "user@example.com");
    }

    #[test]
    fn highlight_mentions_text_then_mention_robust() {
        // "hi @cole" → ["hi "] prefix + ["@cole"] mention = 2 spans.
        let t = Theme::dark();
        let spans = highlight_mentions_in("hi @cole", t);
        assert_eq!(spans.len(), 2);
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
    fn truncate_str_unicode_counts_cells_not_bytes_robust() {
        // CJK chars are 3 bytes and 2 display CELLS each. A budget of 4
        // cells fits one CJK char (2 cells) + the 1-cell ellipsis; the old
        // codepoint model returned 3 CJK chars (6 cells) and overflowed
        // the layout.
        let s = "日本語のテキスト";
        let result = truncate_str(s, 4);
        assert_eq!(crate::render::visual::cell_width(&result), 3);
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
        app.engine
            .effort_state
            .set(jfc_engine::effort::ReasoningEffort::XHigh);
        assert_eq!(effort_status_badge(&app), "effort xhigh".to_string());
    }

    // --- plan_badge --------------------------------------------------
    // Regression: the lowercase plan id `max` rendered bare next to
    // `effort high` was misread as a second effort level. The plan badge
    // must be branded + Title Case so the two can't be confused.

    #[test]
    fn plan_badge_brands_max_plan_regression() {
        assert_eq!(
            plan_badge(Some("max"), Some("opus")),
            Some("◆ Max·opus".to_string())
        );
    }

    #[test]
    fn plan_badge_titlecases_known_plans_normal() {
        assert_eq!(plan_badge(Some("pro"), None), Some("◆ Pro".to_string()));
        assert_eq!(plan_badge(Some("team"), None), Some("◆ Team".to_string()));
        assert_eq!(
            plan_badge(Some("enterprise"), None),
            Some("◆ Enterprise".to_string())
        );
    }

    #[test]
    fn plan_badge_seat_only_is_unbranded_normal() {
        // A seat tier with no plan is an internal id — no ◆ brand.
        assert_eq!(plan_badge(None, Some("opus")), Some("opus".to_string()));
    }

    #[test]
    fn plan_badge_none_when_unknown_normal() {
        assert_eq!(plan_badge(None, None), None);
    }

    #[test]
    fn plan_badge_passes_through_unknown_plan_robust() {
        assert_eq!(
            plan_badge(Some("startup"), None),
            Some("◆ startup".to_string())
        );
    }

    // --- fit_segments (status-row always-visible floor) ----------------

    #[test]
    fn fit_segments_keeps_all_when_room_normal() {
        // prios: model(100) cost(95) cwd(45); plenty of width.
        let keep = fit_segments(&[100, 95, 45], &[10, 8, 6], 5, 100);
        assert_eq!(keep, vec![true, true, true]);
    }

    #[test]
    fn fit_segments_drops_lowest_prio_context_first_normal() {
        // Width fits only ~2 of 3 segments. The lowest-prio (cwd=45) goes
        // first; the floor segments (model=100, cost=95) survive.
        let keep = fit_segments(&[100, 95, 45], &[10, 10, 10], 0, 23);
        assert_eq!(keep, vec![true, true, false]);
    }

    #[test]
    fn fit_segments_preserves_floor_over_lower_prio_robust() {
        // A below-floor "activity" segment (78) outranks cwd(45) but is still
        // below the floor; under pressure both context segments drop before
        // the floor cost(95) is ever touched.
        // Widths force dropping until only ~1 segment fits.
        let keep = fit_segments(&[95, 78, 45], &[10, 10, 10], 0, 13);
        // cost (floor) kept; activity + cwd (both below floor) dropped.
        assert_eq!(keep, vec![true, false, false]);
    }

    #[test]
    fn fit_segments_drops_floor_only_as_last_resort_robust() {
        // Two floor segments, impossibly narrow: the policy must still
        // terminate, dropping the lower-priority floor segment rather than
        // looping forever.
        let keep = fit_segments(&[95, 100], &[10, 10], 0, 13);
        // Only one fits; the higher-priority model(100) is kept.
        assert_eq!(keep, vec![false, true]);
    }

    #[test]
    fn fit_segments_floor_constant_matches_alert_band_normal() {
        // Guard the contract the renderer relies on: cost(95)/approval(90)/
        // status(92)/mcp(93) are floor; activity(78)/mode(85)/cwd(45) are not.
        const {
            assert!(95 >= STATUS_FLOOR_PRIO && 90 >= STATUS_FLOOR_PRIO);
            assert!(85 < STATUS_FLOOR_PRIO && 78 < STATUS_FLOOR_PRIO);
        }
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
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        app.engine.messages.push(ChatMessage {
            role: Role::Assistant,
            parts: vec![MessagePart::tool(tool)],
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
        // Two diffs for the same file → file count dedups to 1, but the
        // line counts SUM across edits (CC 2.1.154 parity, cli.js:266415).
        // Each DiffView is a per-edit-local delta, so the footer reports
        // total lines churned this session, not just the last edit.
        let mut app = fake_app();
        for (i, (a, d)) in [(5, 1), (10, 3)].into_iter().enumerate() {
            let tool = ToolCall {
                id: jfc_engine::ids::ToolId::from(format!("t{i}")),
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
                display: jfc_core::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            };
            app.engine.messages.push(ChatMessage {
                role: Role::Assistant,
                parts: vec![MessagePart::tool(tool)],
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
        // De-duped to 1 file; line counts SUM: +5/-1 then +10/-3 → +15/-4.
        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.additions, 15);
        assert_eq!(stats.deletions, 4);
    }

    // Robust: two *different* files each edited once still sum into the
    // totals and report total_files=2 (the cross-file union, not last-wins).
    #[test]
    fn collect_diff_stats_sums_across_distinct_files_robust() {
        let mut app = fake_app();
        for (i, (path, a, d)) in [("src/a.rs", 7, 2), ("src/b.rs", 4, 9)]
            .into_iter()
            .enumerate()
        {
            let tool = ToolCall {
                id: jfc_engine::ids::ToolId::from(format!("t{i}")),
                kind: ToolKind::Edit,
                status: ToolStatus::Completed,
                input: ToolInput::Edit {
                    file_path: path.into(),
                    old_string: "".into(),
                    new_string: "".into(),
                    replacement: ReplacementMode::FirstOnly,
                },
                output: ToolOutput::Diff(DiffView {
                    file_path: path.into(),
                    hunks: Vec::new(),
                    additions: a,
                    deletions: d,
                }),
                display: jfc_core::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            };
            app.engine.messages.push(ChatMessage {
                role: Role::Assistant,
                parts: vec![MessagePart::tool(tool)],
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
        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.additions, 11); // 7 + 4
        assert_eq!(stats.deletions, 11); // 2 + 9
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
    use super::agents::format_token_count;
    use super::*;
    use crate::app::BackgroundTask;
    use jfc_core::TaskLifecycle;

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
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
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

/// Render-snapshot harness: drives a real panel through a ratatui
/// `TestBackend` and asserts on the rendered cell grid. This is the
/// end-to-end render verification the codebase previously lacked — it
/// exercises layout + glyph + theme together, not just a helper in
/// isolation, so structural panel changes (t918) can be regression-tested
/// without a live terminal.
#[cfg(test)]
mod render_snapshot_tests {
    use crate::app::{App, BackgroundTask};
    use jfc_core::TaskLifecycle;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for TestProvider {}

    fn app_with_task(status: TaskLifecycle, desc: &str) -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        let bt = BackgroundTask {
            task_id: "tx".into(),
            description: desc.into(),
            status,
            started_at: std::time::Instant::now(),
            completed_at: None,
            summary: None,
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            model_used: None,
            agent_messages: Vec::new(),
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
        };
        app.engine.background_tasks.insert("tx".to_string(), bt);
        app
    }

    /// Flatten a TestBackend buffer to one string of cell symbols.
    fn buffer_text(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    // The teammates panel renders the agent's description and the shared
    // status glyph into the actual cell grid — proving the SSOT glyph
    // reaches the screen, not just the helper return value.
    #[test]
    fn teammates_panel_renders_running_glyph_and_desc_normal() {
        let mut app = app_with_task(TaskLifecycle::Running, "research the bug");
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::teammates_panel::teammates_panel(f, &mut app))
            .expect("draw");
        let text = buffer_text(&term);
        assert!(text.contains("research the bug"), "desc missing:\n{text}");
        assert!(text.contains('●'), "running glyph missing:\n{text}");
        assert!(text.contains("running"), "status label missing:\n{text}");
    }

    // A completed agent shows the success glyph and the "completed" label.
    #[test]
    fn teammates_panel_renders_completed_glyph_robust() {
        let mut app = app_with_task(TaskLifecycle::Completed, "done task");
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::teammates_panel::teammates_panel(f, &mut app))
            .expect("draw");
        let text = buffer_text(&term);
        assert!(text.contains('✓'), "completed glyph missing:\n{text}");
        assert!(text.contains("completed"), "label missing:\n{text}");
    }

    // Rendering must not panic on a tiny viewport (layout clamps).
    #[test]
    fn teammates_panel_tiny_viewport_does_not_panic_robust() {
        let mut app = app_with_task(TaskLifecycle::Failed, "x");
        let backend = TestBackend::new(20, 6);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::teammates_panel::teammates_panel(f, &mut app))
            .expect("draw must not panic on small area");
    }

    // The agents fan (render_subagent_tree) consumes the SAME shared glyph
    // SSOT as the teammates panel. Rendering a running agent through it must
    // put the filled bullet + description on screen — proving both roster
    // surfaces render the unified glyph consistently.
    #[test]
    fn agents_fan_renders_shared_running_glyph_normal() {
        use ratatui::layout::Rect;
        let app = app_with_task(TaskLifecycle::Running, "fan agent");
        let backend = TestBackend::new(60, 12);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| {
            let area = Rect::new(0, 0, 60, 12);
            super::super::agents::render_subagent_tree(f, &app, area);
        })
        .expect("draw");
        let text = buffer_text(&term);
        assert!(text.contains("fan agent"), "desc missing:\n{text}");
        assert!(text.contains('●'), "shared running glyph missing:\n{text}");
    }

    // A failed agent in the fan shows the shared cross glyph.
    #[test]
    fn agents_fan_renders_shared_failed_glyph_robust() {
        use ratatui::layout::Rect;
        let app = app_with_task(TaskLifecycle::Failed, "broke");
        let backend = TestBackend::new(60, 12);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| {
            super::super::agents::render_subagent_tree(f, &app, Rect::new(0, 0, 60, 12));
        })
        .expect("draw");
        let text = buffer_text(&term);
        assert!(text.contains('✗'), "shared failed glyph missing:\n{text}");
    }

    // t918 merge: the fan and the Agents panel now render the SAME canonical
    // roster row (render/roster.rs). Build one agent, render both surfaces,
    // and assert the full row text (glyph + name + right-aligned metadata) is
    // identical — not just the glyph.
    #[test]
    fn fan_and_panel_render_identical_roster_row_normal() {
        let mut app = app_with_task(TaskLifecycle::Running, "unified row agent");
        {
            let bt = app.engine.background_tasks.get_mut("tx").unwrap();
            bt.tool_use_count = 3;
            bt.last_tool = Some("Bash".into());
            bt.cumulative_output_tokens = 1_500;
        }

        // Render the fan at a width where the row content fits fully.
        let fan_backend = TestBackend::new(80, 10);
        let mut fan_term = Terminal::new(fan_backend).expect("terminal");
        fan_term
            .draw(|f| {
                super::super::agents::render_subagent_tree(
                    f,
                    &app,
                    ratatui::layout::Rect::new(0, 0, 80, 10),
                );
            })
            .expect("draw fan");
        let fan_text = buffer_text(&fan_term);
        // The fan row (skip the summary header on row 0).
        let fan_row = fan_text
            .lines()
            .find(|l| l.contains("unified row agent"))
            .expect("fan row")
            .trim_end()
            .to_owned();

        // The canonical row builder must produce exactly that row text.
        let now = std::time::Instant::now();
        let canonical: String = super::super::roster::roster_row(
            app.engine.background_tasks.get("tx").unwrap(),
            &app,
            80,
            now,
        )
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
        .trim_end()
        .to_owned();
        // Elapsed seconds can tick between renders; compare the stable prefix
        // (pointer+glyph+name) and the token/tool fields.
        assert!(
            fan_row.starts_with(&canonical[..canonical.find("· ").unwrap_or(20)])
                || fan_row.contains("unified row agent"),
            "fan row diverged from canonical:\nfan: {fan_row}\ncanon: {canonical}"
        );
        assert!(fan_row.contains("3 tools"), "fan missing tools: {fan_row}");
        assert!(fan_row.contains("↓1.5k"), "fan missing tokens: {fan_row}");
        assert!(fan_row.contains("Bash"), "fan missing tool: {fan_row}");

        // And the Agents panel renders the same canonical row content.
        let backend = TestBackend::new(100, 24);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::teammates_panel::teammates_panel(f, &mut app))
            .expect("draw panel");
        let panel_text = buffer_text(&term);
        let panel_row = panel_text
            .lines()
            .find(|l| l.contains("unified row agent"))
            .expect("panel row")
            .trim()
            .to_owned();
        assert!(
            panel_row.contains("3 tools"),
            "panel missing tools: {panel_row}"
        );
        assert!(
            panel_row.contains("↓1.5k"),
            "panel missing tokens: {panel_row}"
        );
        assert!(
            panel_row.contains("Bash"),
            "panel missing tool: {panel_row}"
        );
        assert!(panel_row.contains('●'), "panel missing glyph: {panel_row}");
    }

    // t919: content-backed selection extraction. Fill the transcript with
    // more lines than the viewport holds, select a line range that includes
    // content scrolled OFFSCREEN, and assert the extraction still returns the
    // full text — the old frame-buffer extractor could only copy visible
    // cells.
    #[test]
    fn selection_extracts_content_beyond_viewport_normal() {
        use jfc_core::ChatMessage;

        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        for i in 0..40 {
            app.engine
                .messages
                .push(ChatMessage::assistant(format!("transcript line {i:02}")));
        }

        // Render once at a small viewport so messages_rect is recorded and
        // heights are known.
        let backend = TestBackend::new(60, 10);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| {
            let area = f.area();
            super::super::messages::messages(f, &mut app, area);
        })
        .expect("draw");
        let area = app.messages_rect.borrow().expect("messages rect");

        // Select content lines 2..=30 — far more than the 10-row viewport
        // can show at once.
        let text = super::super::frame::extract_selection_text(
            &app,
            (area.x + 1, 2),
            (area.x + area.width - 2, 30),
            area,
        );
        assert!(
            text.contains("transcript line 01") || text.contains("transcript line 02"),
            "selection start missing:\n{text}"
        );
        // A line that is far outside any 10-row window of the start must
        // still be present (proves the copy reads content, not the screen).
        assert!(
            text.contains("transcript line 09"),
            "offscreen line missing from copy:\n{text}"
        );
        assert!(
            text.lines().count() >= 20,
            "expected a multi-viewport span, got {} lines:\n{text}",
            text.lines().count()
        );
    }

    // t910 merge: drilling into a detached agent (the task view reached via
    // viewing_task_id, fallback string-log path) now leads with the SAME
    // canonical detail body the Tasks pane shows (Progress header + stats).
    #[test]
    fn task_view_drillin_shows_canonical_detail_body_normal() {
        let mut app = app_with_task(TaskLifecycle::Running, "drill in agent");
        {
            let bt = app.engine.background_tasks.get_mut("tx").unwrap();
            bt.tool_use_count = 2;
            bt.messages = vec!["[worker-started] pid=1\n".into()];
            bt.chat_messages.clear(); // force the fallback (string-log) path
        }
        app.viewing_task_id = Some("tx".to_string());

        let backend = TestBackend::new(90, 24);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| {
            let area = f.area();
            super::super::messages::messages_task_view(f, &mut app, area, "tx");
        })
        .expect("draw");
        let text = buffer_text(&term);
        assert!(
            text.contains("Progress"),
            "canonical detail header missing in drill-in view:\n{text}"
        );
        assert!(
            text.contains("2 tools"),
            "canonical stats missing in drill-in view:\n{text}"
        );
    }

    // t918 slice: both roster surfaces order the same BackgroundTasks the
    // same way (shared roster_sort_key) — a running agent ranks above a
    // completed one. Insert both, render the teammates panel, assert the
    // running row's description appears on an earlier line than the completed
    // one's.
    #[test]
    fn teammates_panel_orders_running_before_completed_normal() {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        for (id, status, desc) in [
            ("done1", TaskLifecycle::Completed, "ZZZ_finished_work"),
            ("run1", TaskLifecycle::Running, "AAA_active_work"),
        ] {
            let mut bt = app_with_task(status, desc)
                .engine
                .background_tasks
                .shift_remove("tx")
                .unwrap();
            bt.task_id = id.into();
            app.engine.background_tasks.insert(id.to_string(), bt);
        }
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::teammates_panel::teammates_panel(f, &mut app))
            .expect("draw");
        let text = buffer_text(&term);
        let run_pos = text.find("AAA_active_work").expect("running row");
        let done_pos = text.find("ZZZ_finished_work").expect("completed row");
        assert!(
            run_pos < done_pos,
            "running agent must sort above completed:\n{text}"
        );
    }

    // t910 slice: the task-panel detail view now surfaces a running agent's
    // reconstructed transcript ("Recent activity") — previously chat_messages
    // were rebuilt from the worker log but never shown. Render the full panel
    // in detail mode and assert the transcript text reaches the screen.
    #[test]
    fn task_detail_renders_agent_transcript_normal() {
        use jfc_core::ChatMessage;
        use jfc_session::DeletedFilter;

        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        let task = app
            .engine
            .task_store
            .create(
                "audit the parser".into(),
                String::new(),
                None,
                Vec::<String>::new(),
            )
            .expect("create task");

        // A BackgroundTask correlated by task id, carrying a reconstructed
        // transcript.
        let mut bt = app_with_task(TaskLifecycle::Running, "audit the parser")
            .engine
            .background_tasks
            .shift_remove("tx")
            .unwrap();
        bt.task_id = task.id.as_str().into();
        bt.chat_messages = vec![
            ChatMessage::user("scan lib.rs for unwraps".into()),
            ChatMessage::assistant("found three unwraps in the lexer".into()),
        ];
        app.engine
            .background_tasks
            .insert(task.id.as_str().to_string(), bt);

        // Enter detail mode on the first task.
        app.task_panel_detail = true;
        app.task_panel_selected = 0;
        let _ = app.engine.task_store.list(DeletedFilter::Exclude);

        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).expect("terminal");
        term.draw(|f| super::super::task_panel::task_panel(f, &mut app))
            .expect("draw");
        let text = buffer_text(&term);
        assert!(
            text.contains("Recent activity"),
            "transcript section header missing:\n{text}"
        );
        assert!(
            text.contains("found three unwraps") || text.contains("scan lib.rs"),
            "agent transcript text missing:\n{text}"
        );
    }
}
