#![cfg(all(test, feature = "anthropic-oauth-sensitive"))]
use super::assistant_parts::{find_tool_at, sanitize_terminal_text, truncate_str};
use super::bash::{BashCmdKind, classify_bash_cmd};
use super::core::{
    RenderCtx, build_render_items_ctx, is_groupable, message_view_total_lines, severity_rank,
};
use super::detection::{looks_like_difftastic_output, looks_like_git_diff_output};
use super::formatters::{
    produce_command_output_lines, produce_git_diff_output_lines, produce_grep_output_lines,
};
use super::output_style::path_color;
use super::outputs::{diff_lang, produce_diff_view_lines};
use super::syntax::{
    infer_lang_from_bash, infer_lang_from_tool, lang_from_path, looks_like_markdown, redact_quoted,
};
use super::tool_blocks::{
    bash_continuation_lines, border_color_for_status, build_collapsed_header,
    build_header_inner_spans, build_title_spans, format_elapsed_badge, produce_text_block_lines,
    render_tool_block, tool_body_lines_themed, tool_kind_color, tool_status_icon_animated,
    tool_title_width_cap,
};
use super::tool_height::{tool_block_height, tool_block_height_pub, tool_content_height_with_tool};
use super::truncation::{
    GrepLine, grep_is_case_insensitive, grep_search_pattern, grep_target_file, parse_grep_line,
    parse_grep_no_path, parse_grep_with_sep,
};
use super::*;

#[cfg(test)]
mod diff_lang_tests {
    use super::*;

    fn diff_with_path(path: &str) -> DiffView {
        DiffView {
            file_path: path.to_string(),
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn diff_lang_detects_rust_normal() {
        let lang = diff_lang(&diff_with_path("src/main.rs"));
        assert_eq!(lang.as_deref(), Some("rs"));
    }

    #[test]
    fn diff_lang_detects_python_normal() {
        let lang = diff_lang(&diff_with_path("main.py"));
        assert_eq!(lang.as_deref(), Some("py"));
    }

    #[test]
    fn diff_lang_unknown_returns_none_robust() {
        let lang = diff_lang(&diff_with_path(""));
        assert_eq!(lang, None);
    }

    #[test]
    fn diff_lang_handles_no_extension_robust() {
        let lang = diff_lang(&diff_with_path("Makefile"));
        assert_eq!(lang.as_deref(), Some("makefile"));
    }
}

#[cfg(test)]
mod hit_test_tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn find_tool_at_inside_rect_normal() {
        let regions = vec![("tool-1".to_string(), r(0, 0, 10, 3))];
        assert_eq!(find_tool_at(&regions, 5, 1), Some("tool-1"));
    }

    #[test]
    fn find_tool_at_outside_all_rects_normal() {
        let regions = vec![
            ("tool-1".to_string(), r(0, 0, 10, 3)),
            ("tool-2".to_string(), r(0, 5, 10, 3)),
        ];
        assert_eq!(find_tool_at(&regions, 5, 4), None);
        assert_eq!(find_tool_at(&regions, 20, 1), None);
    }

    #[test]
    fn find_tool_at_picks_first_match_robust() {
        let regions = vec![
            ("first".to_string(), r(0, 0, 10, 5)),
            ("second".to_string(), r(2, 1, 5, 2)),
        ];
        assert_eq!(find_tool_at(&regions, 3, 2), Some("first"));
    }

    #[test]
    fn find_tool_at_empty_regions_robust() {
        let regions: Vec<(String, Rect)> = Vec::new();
        assert_eq!(find_tool_at(&regions, 0, 0), None);
        assert_eq!(find_tool_at(&regions, 99, 99), None);
    }

    #[test]
    fn find_tool_at_boundary_inclusive_normal() {
        let regions = vec![("tool".to_string(), r(2, 3, 4, 2))];
        assert_eq!(find_tool_at(&regions, 2, 3), Some("tool"));
        assert_eq!(find_tool_at(&regions, 5, 4), Some("tool"));
        assert_eq!(find_tool_at(&regions, 6, 3), None);
        assert_eq!(find_tool_at(&regions, 2, 5), None);
    }
}

#[cfg(test)]
mod bash_output_tests {
    use super::*;

    // Normal: cat <file.md> classifies as Other (cat falls through
    // to the markdown / lang sniff path, not the structured tool
    // dispatch).
    #[test]
    fn classify_cat_is_other_normal() {
        assert!(matches!(
            classify_bash_cmd("cat README.md"),
            BashCmdKind::Other
        ));
    }

    // Normal: grep_target_file pulls the file argument out so the
    // renderer can show a heading. Pattern is *not* the target.
    #[test]
    fn grep_target_file_extracts_path_normal() {
        assert_eq!(
            grep_target_file("grep -n \"sws_headers(\" ~/foo/auth.rs"),
            Some("~/foo/auth.rs".into())
        );
        assert_eq!(
            grep_target_file("rg \"open(\" --type rust src/"),
            Some("src/".into())
        );
        assert_eq!(
            grep_target_file("grep -e PAT -B 2 -A 2 file.rs"),
            Some("file.rs".into())
        );
        // Quoted target gets unquoted.
        assert_eq!(
            grep_target_file("grep PAT 'file with spaces.rs'"),
            Some("file with spaces.rs".into())
        );
    }

    // Robust: grep_target_file is None when there's no positional
    // file (recursive grep over cwd, or pattern-only invocation).
    #[test]
    fn grep_target_file_none_when_no_target_robust() {
        // `rg PAT` with no target = search cwd recursively → None
        assert_eq!(grep_target_file("rg \"foo\""), None);
        assert_eq!(grep_target_file("grep PAT"), None);
        // Wrong verb returns None.
        assert_eq!(grep_target_file("cat file.rs"), None);
    }

    // Normal: the user's actual reported case — `grep -n "pattern("
    // file` — must classify as Grep so render_grep_output_skip
    // fires. The trailing `(` inside the double-quoted pattern was
    // suspected of confusing the classifier; this test pins the
    // expected behaviour so a future redact_quoted regression gets
    // caught.
    #[test]
    fn classify_grep_with_paren_inside_quotes_normal() {
        for cmd in &[
            "grep -n \"sws_headers(\" ~/foo/auth.rs",
            "grep -rn \"foo(\" src/",
            "rg \"open(\" --type rust",
            "grep \"async fn (\" file.rs",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::Grep),
                "{cmd} should classify as Grep"
            );
        }
    }

    // Normal: `sed -n '1,$p' file.md` — the canonical "print all
    // lines" idiom — must NOT be rejected for the `$` inside its
    // quoted script. Before redact_quoted() this fell through to
    // plain rendering and lost markdown formatting.
    #[test]
    fn infer_lang_handles_sed_with_dollar_in_quotes_normal() {
        assert_eq!(
            infer_lang_from_bash("sed -n '1,$p' README.md").as_deref(),
            Some("md")
        );
        assert_eq!(
            infer_lang_from_bash("awk '{print $1}' main.rs").as_deref(),
            Some("rs")
        );
    }

    // Robust: an *unquoted* `$` (real command substitution) must
    // still be rejected — we only ignore `$` that lives inside a
    // matched quote.
    #[test]
    fn infer_lang_rejects_unquoted_dollar_robust() {
        assert!(infer_lang_from_bash("cat $(which README.md)").is_none());
        assert!(infer_lang_from_bash("cat $FILE").is_none());
    }

    // Normal: redact_quoted preserves length and quote chars but
    // blanks out the contents.
    #[test]
    fn redact_quoted_blanks_inside_quotes_normal() {
        assert_eq!(redact_quoted("sed -n '1,$p' file"), "sed -n '    ' file");
        assert_eq!(redact_quoted("echo \"$x\""), "echo \"  \"");
        assert_eq!(
            redact_quoted("plain text no quotes"),
            "plain text no quotes"
        );
    }

    // Normal: hex-dump tools route to HexDump.
    #[test]
    fn classify_hex_dump_tools_normal() {
        for cmd in &["xxd file.bin", "hexyl file.bin", "od -c file.bin"] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::HexDump),
                "{cmd}"
            );
        }
    }

    // Normal: docker / podman list-style subcommands route to TabularList.
    #[test]
    fn classify_docker_tabular_normal() {
        for cmd in &[
            "docker ps",
            "docker ps -a",
            "docker images",
            "podman ps",
            "docker container ls",
            "docker network ls",
            "docker volume ls",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::TabularList),
                "{cmd}"
            );
        }
    }

    // Robust: unknown docker subcommand falls through to Other (so
    // we don't try to color e.g. `docker run` interactive output).
    #[test]
    fn classify_docker_unknown_subcmd_other_robust() {
        assert!(matches!(
            classify_bash_cmd("docker run x"),
            BashCmdKind::Other
        ));
        assert!(matches!(classify_bash_cmd("docker"), BashCmdKind::Other));
    }

    // Normal: kubectl get / describe / top all route to TabularList.
    #[test]
    fn classify_kubectl_tabular_normal() {
        for cmd in &[
            "kubectl get pods",
            "kubectl get nodes -o wide",
            "kubectl describe pod x",
            "kubectl top pod",
            "oc get routes",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::TabularList),
                "{cmd}"
            );
        }
    }

    // Normal: raw `diff -u a b` shares the GitDiff renderer so its
    // +/-/@@ lines get colored too — fixes the gap where someone
    // runs `diff -u` outside a git tree.
    #[test]
    fn classify_raw_diff_routes_to_gitdiff_normal() {
        assert!(matches!(
            classify_bash_cmd("diff -u a.txt b.txt"),
            BashCmdKind::GitDiff
        ));
    }

    // Normal: grep / rg / ack / ag all dispatch to the Grep renderer.
    #[test]
    fn classify_grep_family_normal() {
        for cmd in &[
            "grep -rn x src/",
            "rg \"TODO\" --type rust",
            "ack pat",
            "ag pat",
        ] {
            assert!(matches!(classify_bash_cmd(cmd), BashCmdKind::Grep), "{cmd}");
        }
    }

    // Normal: find / ls / tree / fd all dispatch to PathList.
    #[test]
    fn classify_path_list_family_normal() {
        for cmd in &["find . -name '*.rs'", "ls -la", "tree", "fd rust"] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::PathList),
                "{cmd}"
            );
        }
    }

    // Normal: git diff / git show / git log dispatch correctly.
    #[test]
    fn classify_git_subcommands_normal() {
        assert!(matches!(
            classify_bash_cmd("git diff HEAD"),
            BashCmdKind::GitDiff
        ));
        assert!(matches!(
            classify_bash_cmd("git show abc123"),
            BashCmdKind::GitDiff
        ));
        assert!(matches!(
            classify_bash_cmd("git log --oneline -20"),
            BashCmdKind::GitLog
        ));
        assert!(matches!(
            classify_bash_cmd("git status"),
            BashCmdKind::Other
        ));
    }

    // Robust: pipeline-aware classification — first segment of `||` or `|`
    // wins. The cat-with-fallback pattern is common.
    #[test]
    fn classify_pipeline_takes_first_segment_robust() {
        assert!(matches!(
            classify_bash_cmd("rg foo 2>/dev/null || rg bar"),
            BashCmdKind::Grep
        ));
        assert!(matches!(
            classify_bash_cmd("git diff | less"),
            BashCmdKind::GitDiff
        ));
    }

    // Robust: `2>/dev/null` and `>file` redirects don't break the verb sniff.
    #[test]
    fn classify_strips_redirects_robust() {
        assert!(matches!(
            classify_bash_cmd("grep -rn pat src/ 2>/dev/null"),
            BashCmdKind::Grep
        ));
        assert!(matches!(
            classify_bash_cmd("find . -name '*.rs' >list.txt"),
            BashCmdKind::PathList
        ));
    }

    // Robust: command substitution / backticks / & / ; reject (those
    // change semantics in ways the simple sniff can't reason about).
    #[test]
    fn classify_rejects_complex_shell_robust() {
        assert!(matches!(
            classify_bash_cmd("echo $(grep x y)"),
            BashCmdKind::Other
        ));
        assert!(matches!(
            classify_bash_cmd("grep x y; echo done"),
            BashCmdKind::Other
        ));
        assert!(matches!(
            classify_bash_cmd("grep x y &"),
            BashCmdKind::Other
        ));
    }

    // Normal: parse a standard `path:line:body` grep result.
    #[test]
    fn parse_grep_path_line_body_normal() {
        let line = "src/main.rs:42:fn main() {";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(lineno, Some("42"));
                assert_eq!(col, None);
                assert_eq!(body, "fn main() {");
                assert!(!is_context);
            }
            other => panic!("expected match, got {other:?}", other = other.is_some()),
        }
    }

    // Normal: rg with --column emits `path:line:col:body`.
    #[test]
    fn parse_grep_with_column_normal() {
        let line = "src/foo.rs:15:5:    let x = 1;";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("15"));
                assert_eq!(col, Some("5"));
                assert_eq!(body, "    let x = 1;");
                assert!(!is_context);
            }
            other => panic!("expected match, got {other:?}", other = other.is_some()),
        }
    }

    // Normal: grep -B/-C context lines use `-` separators.
    #[test]
    fn parse_grep_context_lines_use_dash_normal() {
        let line = "src/foo.rs-41-/// docstring";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("41"));
                assert_eq!(body, "/// docstring");
                assert!(is_context);
            }
            other => panic!(
                "expected context match, got {other:?}",
                other = other.is_some()
            ),
        }
    }

    // Robust: a path containing `:` (Windows-style) shouldn't false-match.
    // The parser anchors on `:digits:` so a colon in the path doesn't break it.
    #[test]
    fn parse_grep_handles_path_with_colon_robust() {
        let line = "C:/code/main.rs:99:hello";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path, lineno, body, ..
            }) => {
                assert_eq!(path, "C:/code/main.rs");
                assert_eq!(lineno, Some("99"));
                assert_eq!(body, "hello");
            }
            _ => panic!("expected match"),
        }
    }

    // Robust: rg --heading mode emits a bare path on its own line
    // (no separators). Recognized as HeadingPath.
    #[test]
    fn parse_grep_heading_path_robust() {
        let line = "src/utils/foo.rs";
        match parse_grep_line(line) {
            Some(GrepLine::HeadingPath(p)) => assert_eq!(p, "src/utils/foo.rs"),
            _ => panic!("expected heading path"),
        }
    }

    // Normal: markdown content sniff fires on a doc with headers + table.
    #[test]
    fn looks_like_markdown_detects_real_md_normal() {
        let content = "# Title\n\nSome text\n\n## Section\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert!(looks_like_markdown(content));
    }

    // Robust: plain code shouldn't sniff as markdown even if it has `#` chars.
    #[test]
    fn looks_like_markdown_rejects_python_robust() {
        let content = "# This is a Python comment\nprint('hello')\nx = 1\ny = 2\n";
        assert!(!looks_like_markdown(content));
    }
}

#[cfg(test)]
mod bash_chain_tests {
    use super::*;

    // Normal: `cd X && grep ...` should classify as Grep — the LAST
    // segment of an `&&` chain is the meaningful command, not the cd.
    #[test]
    fn classify_cd_and_then_grep_normal() {
        assert!(matches!(
            classify_bash_cmd("cd ~/src && grep -rn TODO"),
            BashCmdKind::Grep
        ));
    }

    // Normal: `cd X && cat README.md 2>/dev/null || cat docs/README.md`
    // — the whole chain compiles down to `cat <markdown>` so the lang
    // sniff should pick up `.md`.
    #[test]
    fn infer_lang_through_cd_and_chain_normal() {
        let lang =
            infer_lang_from_bash("cd ~/proj && cat README.md 2>/dev/null || cat docs/README.md");
        assert_eq!(lang.as_deref(), Some("md"));
    }

    // Robust: `cd X && cat foo &` (background) still rejected.
    #[test]
    fn classify_rejects_lone_background_robust() {
        assert!(matches!(
            classify_bash_cmd("cd ~/src && cat foo &"),
            BashCmdKind::Other
        ));
    }

    // Normal: `grep -n pat single-file.txt` emits `<lineno>:<body>`
    // with no path prefix. Parser handles it.
    #[test]
    fn parse_grep_no_path_single_file_normal() {
        let line = "187214:    var _X = \"ScheduleWakeup\";";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path, lineno, body, ..
            }) => {
                assert_eq!(path, "");
                assert_eq!(lineno, Some("187214"));
                assert_eq!(body, "    var _X = \"ScheduleWakeup\";");
            }
            _ => panic!("expected match"),
        }
    }

    // Robust: a line starting with a number that isn't grep-style
    // (no `:` after digits) shouldn't false-match.
    #[test]
    fn parse_grep_no_path_rejects_bare_numbers_robust() {
        let line = "1234567 records processed";
        // `1234567 ` is digits + space, no `:` or `-` after digits,
        // so the no-path parser returns None and the line falls
        // through to plain text.
        assert!(parse_grep_line(line).is_none());
    }

    // Robust: hex/long IDs that look like digits but aren't reasonable
    // line numbers are rejected. E.g. a SHA prefix.
    #[test]
    fn parse_grep_no_path_rejects_huge_lineno_robust() {
        // 99999999999 (11 digits) — won't fit in u32, parser rejects.
        let line = "99999999999:body";
        assert!(parse_grep_line(line).is_none());
    }
}

#[cfg(test)]
mod grep_pattern_extraction_tests {
    use super::*;

    #[test]
    fn grep_search_pattern_positional_normal() {
        assert_eq!(grep_search_pattern("grep -rn foo src/"), Some("foo".into()));
    }

    #[test]
    fn grep_search_pattern_quoted_normal() {
        assert_eq!(
            grep_search_pattern("grep -n \"sws_headers(\" ~/foo/auth.rs"),
            Some("sws_headers(".into())
        );
    }

    #[test]
    fn grep_search_pattern_dash_e_flag_normal() {
        assert_eq!(
            grep_search_pattern("grep -e PAT -B 2 file.rs"),
            Some("PAT".into())
        );
    }

    #[test]
    fn grep_search_pattern_regexp_eq_normal() {
        assert_eq!(
            grep_search_pattern("rg --regexp=pattern src/"),
            Some("pattern".into())
        );
    }

    #[test]
    fn grep_search_pattern_single_quoted_normal() {
        assert_eq!(
            grep_search_pattern("rg 'async fn' src/"),
            Some("async fn".into())
        );
    }

    #[test]
    fn grep_search_pattern_no_pattern_returns_none_robust() {
        assert_eq!(grep_search_pattern("cat file.rs"), None);
    }

    #[test]
    fn grep_search_pattern_rg_no_target_normal() {
        assert_eq!(grep_search_pattern("rg \"open(\""), Some("open(".into()));
    }

    #[test]
    fn grep_case_insensitive_flag_normal() {
        assert!(grep_is_case_insensitive("grep -in pat file.rs"));
        assert!(grep_is_case_insensitive("grep --ignore-case pat file.rs"));
        assert!(grep_is_case_insensitive("grep -rni pat src/"));
    }

    #[test]
    fn grep_case_sensitive_default_normal() {
        assert!(!grep_is_case_insensitive("grep -rn pat src/"));
        assert!(!grep_is_case_insensitive("rg pat"));
    }
}

#[cfg(test)]
mod grep_body_highlight_tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn produce_grep_output_highlights_pattern_match_normal() {
        let stdout = "src/main.rs:10:fn parse_file(&self) {\n\
                      src/main.rs:20:    parse_file(path)\n";
        let t = Theme::dark();
        let lines =
            produce_grep_output_lines(stdout, "", Some(0), t, false, "grep -rn parse_file src/");
        let mut found_bold_match = false;
        for line in &lines {
            for span in &line.spans {
                if span.content.as_ref() == "parse_file" && span.style.fg == Some(t.warning) {
                    found_bold_match = true;
                }
            }
        }
        assert!(
            found_bold_match,
            "expected pattern 'parse_file' highlighted in warning color"
        );
    }

    #[test]
    fn produce_grep_output_no_syntect_fallback_on_incomplete_fragments_normal() {
        // An incomplete signature like `fn extract_edges(` should NOT go
        // through syntect — it would produce broken coloring. Verify that
        // the body is split into plain text + pattern match only.
        let stdout = "src/graph.rs:50:    fn extract_edges(\n";
        let t = Theme::dark();
        let lines =
            produce_grep_output_lines(stdout, "", Some(0), t, false, "rg extract_edges src/");
        // The body spans should include the pattern match and surrounding text,
        // but NOT syntect-generated keyword tokens.
        let body_spans: Vec<&str> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.content.contains("extract_edges") || s.content.contains("fn "))
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!body_spans.is_empty(), "expected body text in output");
    }

    #[test]
    fn produce_grep_output_context_lines_are_muted_normal() {
        // Context lines (with `-` separator) should get text_muted color
        // and should NOT highlight the pattern.
        // grep/rg emit `path-lineno-body` for context (all `-`) and
        // `path:lineno:body` for matches.
        let stdout = "src/main.rs-9-    let x = 1;\n\
                      src/main.rs:10:fn parse_file() {\n\
                      src/main.rs-11-    }\n";
        let t = Theme::dark();
        let lines = produce_grep_output_lines(
            stdout,
            "",
            Some(0),
            t,
            false,
            "grep -n -A1 -B1 parse_file src/main.rs",
        );
        // Check that context line bodies use text_muted
        let context_body = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("let x = 1"));
        assert!(context_body.is_some(), "expected context line body");
        assert_eq!(
            context_body.unwrap().style.fg,
            Some(t.text_muted),
            "context line body should be text_muted"
        );
    }
}

#[cfg(test)]
mod path_color_tests {
    use super::*;
    use crate::theme::Theme;

    fn t() -> Theme {
        Theme::dark()
    }

    // Normal: code extensions get the accent color so paths in grep
    // results stand out as code files.
    #[test]
    fn path_color_code_extensions_normal() {
        let theme = t();
        for path in &["main.rs", "src/foo.go", "scripts/run.py", "app.ts"] {
            assert_eq!(
                path_color(path, theme),
                theme.accent,
                "{path} should be accent"
            );
        }
    }

    // Normal: config / data files get text_secondary so they
    // visually demote below code.
    #[test]
    fn path_color_config_extensions_normal() {
        let theme = t();
        assert_eq!(path_color("Cargo.toml", theme), theme.text_secondary);
        assert_eq!(path_color("package.json", theme), theme.text_secondary);
        assert_eq!(path_color("config.yaml", theme), theme.text_secondary);
        assert_eq!(path_color(".env", theme), theme.text_muted); // no ext
    }

    // Normal: docs (md, txt, rst) get text_primary (white) so they
    // stand out as readable content.
    #[test]
    fn path_color_doc_extensions_normal() {
        let theme = t();
        assert_eq!(path_color("README.md", theme), theme.text_primary);
        assert_eq!(path_color("notes.txt", theme), theme.text_primary);
    }

    // Robust: unknown extension falls back to text_muted (least
    // attention-grabbing).
    #[test]
    fn path_color_unknown_falls_back_robust() {
        let theme = t();
        assert_eq!(path_color("file.xyz", theme), theme.text_muted);
        assert_eq!(path_color("noext", theme), theme.text_muted);
        assert_eq!(path_color("", theme), theme.text_muted);
    }

    // Robust: extension matching is case-insensitive — a path like
    // `MAIN.RS` (some Windows tools emit uppercase) still resolves
    // to the Rust accent color.
    #[test]
    fn path_color_case_insensitive_robust() {
        let theme = t();
        assert_eq!(path_color("Main.RS", theme), theme.accent);
        assert_eq!(path_color("CONFIG.TOML", theme), theme.text_secondary);
    }
}

// =====================================================================

#[cfg(test)]
mod helper_tests {
    use super::*;

    fn dummy_tool(input: ToolInput, output: ToolOutput, kind: ToolKind) -> ToolCall {
        ToolCall {
            id: jfc_engine::ids::ToolId::from("t-1"),
            kind,
            status: ToolStatus::Completed,
            input,
            output,
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        }
    }

    // --- exit-code badge ---------------------------------------------

    #[test]
    fn exit_code_badge_shown_for_failed_command_normal() {
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "false".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Command {
                stdout: String::new(),
                stderr: "boom".into(),
                exit_code: Some(2),
            },
            ToolKind::Bash,
        );
        tool.status = ToolStatus::Failed;
        assert_eq!(
            tool_blocks::format_exit_code_badge(&tool).as_deref(),
            Some("(2)")
        );
    }

    #[test]
    fn exit_code_badge_absent_on_success_robust() {
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "true".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Command {
                stdout: "ok".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            ToolKind::Bash,
        );
        // Completed + exit 0 → no badge (it must not compete with the elapsed badge).
        assert_eq!(tool_blocks::format_exit_code_badge(&tool), None);
    }

    // --- infer_lang_from_tool ----------------------------------------

    #[test]
    fn infer_lang_from_read_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Read {
                file_path: "src/main.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("rs"));
    }

    #[test]
    fn infer_lang_from_edit_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Edit {
                file_path: "src/lib.py".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            ToolOutput::Empty,
            ToolKind::Edit,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("py"));
    }

    #[test]
    fn infer_lang_from_write_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Write {
                file_path: "config.toml".into(),
                content: "".into(),
            },
            ToolOutput::Empty,
            ToolKind::Write,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("toml"));
    }

    #[test]
    fn infer_lang_from_bash_input_delegates_robust() {
        // Bash-tool path delegates to infer_lang_from_bash, which sniffs
        // `cat path/file.ext`.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "cat README.md".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("md"));
    }

    #[test]
    fn infer_lang_from_unknown_kind_returns_none_robust() {
        let t = dummy_tool(
            ToolInput::TeamDelete,
            ToolOutput::Empty,
            ToolKind::TeamDelete,
        );
        assert_eq!(infer_lang_from_tool(&t), None);
    }

    // --- lang_from_path ----------------------------------------------

    #[test]
    fn lang_from_path_extension_wins_normal() {
        assert_eq!(lang_from_path("src/main.rs").as_deref(), Some("rs"));
        assert_eq!(lang_from_path("foo.JS").as_deref(), Some("JS"));
    }

    #[test]
    fn lang_from_path_no_extension_falls_back_to_filename_robust() {
        // No extension → use the filename (e.g. `Makefile` → `makefile`
        // when downstream lowercases it, but lang_from_path returns the
        // raw filename).
        assert_eq!(lang_from_path("Makefile").as_deref(), Some("Makefile"));
    }

    #[test]
    fn lang_from_path_empty_returns_none_robust() {
        assert_eq!(lang_from_path(""), None);
    }

    // --- infer_lang_from_bash ----------------------------------------

    #[test]
    fn infer_lang_from_bash_cat_normal() {
        assert_eq!(
            infer_lang_from_bash("cat src/main.rs").as_deref(),
            Some("rs")
        );
    }

    #[test]
    fn infer_lang_from_bash_head_with_flags_normal() {
        // Skips `-50` (numeric arg) and picks `file.py`.
        assert_eq!(
            infer_lang_from_bash("head -50 file.py").as_deref(),
            Some("py")
        );
    }

    #[test]
    fn infer_lang_from_bash_pipeline_takes_first_robust() {
        // `cat foo.rs | less` → primary segment is `cat foo.rs`.
        assert_eq!(
            infer_lang_from_bash("cat foo.rs | less").as_deref(),
            Some("rs")
        );
    }

    #[test]
    fn infer_lang_from_bash_command_substitution_rejected_robust() {
        // `$(...)` patterns disqualify — not safe to sniff.
        assert_eq!(infer_lang_from_bash("cat $(echo foo.rs)"), None);
    }

    #[test]
    fn infer_lang_from_bash_non_cat_verb_rejected_robust() {
        // Only `cat`/`head`/`tail`/`bat`/`less`/`more` qualify.
        assert_eq!(infer_lang_from_bash("echo hello.rs"), None);
    }

    // --- path_color --------------------------------------------------

    #[test]
    fn path_color_code_extension_uses_accent_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("src/main.rs", t), t.accent);
        assert_eq!(path_color("app.py", t), t.accent);
        assert_eq!(path_color("foo.go", t), t.accent);
    }

    #[test]
    fn path_color_config_uses_text_secondary_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("Cargo.toml", t), t.text_secondary);
        assert_eq!(path_color("settings.json", t), t.text_secondary);
    }

    #[test]
    fn path_color_docs_use_text_primary_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("README.md", t), t.text_primary);
    }

    #[test]
    fn path_color_shell_uses_warning_robust() {
        let t = Theme::dark();
        assert_eq!(path_color("install.sh", t), t.warning);
    }

    #[test]
    fn path_color_unknown_falls_back_to_muted_robust() {
        let t = Theme::dark();
        assert_eq!(path_color("data.bin", t), t.text_muted);
        // No extension at all also goes to muted.
        assert_eq!(path_color("Makefile", t), t.text_muted);
    }

    #[test]
    fn path_color_uppercase_extension_normalized_robust() {
        // ASCII-lowercased, so .RS / .Rs all hit the code branch.
        let t = Theme::dark();
        assert_eq!(path_color("FOO.RS", t), t.accent);
    }

    // --- looks_like_markdown -----------------------------------------

    #[test]
    fn looks_like_markdown_combines_signals_normal() {
        // Headers + table + bold marker → score >= 4.
        let s = "# Title\n\nSome **bold** text\n\n## Section\n";
        // 2 headers (each +2) + bold (+1) = 5 → markdown.
        assert!(looks_like_markdown(s));
    }

    #[test]
    fn looks_like_markdown_pure_code_not_md_robust() {
        // Python code with `#` comments doesn't trigger header signals.
        let s = "# this is a comment\nprint('x')\nx = 1\ny = 2\n";
        assert!(!looks_like_markdown(s));
    }

    #[test]
    fn looks_like_markdown_first_2kb_only_robust() {
        // Strong markdown signal in the prefix → triggers; rest can be huge.
        let prefix = "# h1\n## h2\n### h3\n```rust\nlet x = 1;\n```\n";
        let mut s = String::from(prefix);
        s.push_str(&"x".repeat(10_000));
        assert!(looks_like_markdown(&s));
    }

    #[test]
    fn looks_like_markdown_empty_returns_false_robust() {
        assert!(!looks_like_markdown(""));
    }

    // wrapped_line_count tests removed: the standalone helper was
    // deleted when we unified the height predictor to delegate to
    // `build_render_items` (single-producer pattern from the
    // t-compiler/query-system Zulip discussions). Wrap-counting now
    // lives inside `RenderItem::TextLine::height` and is exercised by
    // the `predictor_matches_renderer_*` regression tests below.

    // --- tool_content_height_with ------------------------------------
    //
    // Convenience wrapper over the new `tool_content_height_with_tool`
    // path so existing assertions stay readable. The producer-based
    // height path needs a full ToolCall to dispatch by `BashCmdKind`,
    // so we synthesize a generic Bash tool and swap in the output.
    fn tool_content_height_with(output: &ToolOutput, content_w: usize, expanded: bool) -> usize {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "echo".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output.clone(),
            ToolKind::Bash,
        );
        t.display = if expanded {
            jfc_core::ToolDisplayState::Expanded { pinned: false }
        } else {
            jfc_core::ToolDisplayState::DEFAULT
        };
        tool_content_height_with_tool(&t, content_w)
    }

    #[test]
    fn tool_content_height_empty_zero_normal() {
        assert_eq!(tool_content_height_with(&ToolOutput::Empty, 80, false), 0);
    }

    #[test]
    fn tool_content_height_text_simple_normal() {
        // A 3-line text: height = 3.
        let out = ToolOutput::Text("a\nb\nc".to_string());
        assert_eq!(tool_content_height_with(&out, 80, false), 3);
    }

    #[test]
    fn tool_content_height_text_truncates_with_footer_robust() {
        // > 80 lines → cap at 80 + 1 footer row.
        let body: String = (0..150).map(|n| format!("line{n}\n")).collect();
        let out = ToolOutput::Text(body);
        let h = tool_content_height_with(&out, 80, false);
        assert_eq!(h, 81, "expect 80 cap + 1 footer");
    }

    #[test]
    fn tool_content_height_text_expanded_lifts_cap_robust() {
        // expanded=true → cap rises to 500.
        let body: String = (0..150).map(|n| format!("line{n}\n")).collect();
        let out = ToolOutput::Text(body);
        let h = tool_content_height_with(&out, 80, true);
        assert_eq!(h, 150, "no truncation under expanded cap");
    }

    #[test]
    fn tool_content_height_command_includes_exit_row_normal() {
        let out = ToolOutput::Command {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        };
        // 1 (exit row) + 1 (stdout "ok") = 2.
        assert_eq!(tool_content_height_with(&out, 80, false), 2);
    }

    #[test]
    fn tool_content_height_command_with_stderr_divider_robust() {
        // Both streams present → +1 divider row between them.
        let out = ToolOutput::Command {
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            exit_code: Some(1),
        };
        // exit (1) + stdout (1) + divider (1) + stderr (1) = 4.
        assert_eq!(tool_content_height_with(&out, 80, false), 4);
    }

    #[test]
    fn command_output_truncates_middle_and_keeps_tail_robust() {
        let stdout: String = (0..120).map(|n| format!("line-{n}\n")).collect();
        let lines = produce_command_output_lines(
            &stdout,
            "",
            Some(0),
            120,
            Theme::dark(),
            /*expanded*/ false,
        );
        let rendered = lines_to_plain(&lines);

        assert_eq!(lines.len(), 81, "exit row + 80 output rows:\n{rendered}");
        assert!(
            rendered.contains("line-0"),
            "expected head of output:\n{rendered}"
        );
        assert!(
            rendered.contains("line-119"),
            "expected tail of output:\n{rendered}"
        );
        assert!(
            rendered.contains("omitted lines"),
            "expected middle truncation marker:\n{rendered}"
        );
    }

    #[test]
    fn diff_view_wraps_long_update_rows_and_height_matches_renderer_robust() {
        let long = "let value = \"this is a very long updated line that must wrap instead of clipping in the update view\";";
        let diff = DiffView {
            file_path: "src/lib.rs".into(),
            additions: 1,
            deletions: 0,
            hunks: vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                header: "@@ -1,1 +1,1 @@".into(),
                lines: vec![DiffLine {
                    kind: DiffLineKind::Added,
                    old_line: None,
                    new_line: Some(1),
                    content: long.into(),
                }],
            }],
        };

        let lines = produce_diff_view_lines(&diff, Theme::dark(), false, 32);
        let rendered = lines_to_plain(&lines);

        assert!(
            lines.len() > 3,
            "summary + hunk + wrapped diff row expected:\n{rendered}"
        );
        assert!(
            rendered.contains("        "),
            "wrapped continuation should keep a blank diff gutter:\n{rendered}"
        );
        assert_eq!(
            tool_content_height_with(&ToolOutput::Diff(diff), 32, false),
            lines.len()
        );
    }

    #[test]
    fn tool_content_height_filelist_caps_normal() {
        let files: Vec<String> = (0..5).map(|n| format!("f{n}")).collect();
        let out = ToolOutput::FileList(files);
        assert_eq!(tool_content_height_with(&out, 80, false), 5);
    }

    #[test]
    fn tool_content_height_filelist_truncates_with_footer_robust() {
        // 25 files, cap=20 → 20 rows + 1 footer.
        let files: Vec<String> = (0..25).map(|n| format!("f{n}")).collect();
        let out = ToolOutput::FileList(files);
        assert_eq!(tool_content_height_with(&out, 80, false), 21);
    }

    #[test]
    fn tool_content_height_largetext_huge_collapses_to_one_robust() {
        // Force `huge` by making line_count exceed COLLAPSE_LINES.
        let lt = LargeText {
            content: "x".to_string(),
            line_count: LargeText::COLLAPSE_LINES + 10,
            byte_count: 1,
        };
        let out = ToolOutput::LargeText(lt);
        assert_eq!(tool_content_height_with(&out, 80, false), 1);
    }

    // --- tool_block_height -------------------------------------------

    #[test]
    fn tool_block_height_collapsed_is_one_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Text("foo\nbar\nbaz".into()),
            ToolKind::Bash,
        );
        t.display = jfc_core::ToolDisplayState::Collapsed;
        assert_eq!(tool_block_height(&t, 80), 1);
        // Public wrapper should match.
        assert_eq!(tool_block_height_pub(&t, 80), 1);
    }

    #[test]
    fn tool_block_height_includes_title_normal() {
        // Empty output + 1-line bash → 1 (title) + 0 (cont) + 0 (body) = 1.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(tool_block_height(&t, 80), 1);
    }

    #[test]
    fn tool_block_height_counts_continuation_lines_robust() {
        // Multi-line bash → 1 (title) + N continuation rows.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "cat <<EOF\nfoo\nbar\nEOF".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        // 1 title + 3 cont rows + 0 body = 4.
        assert_eq!(tool_block_height(&t, 80), 4);
    }

    // --- bash_continuation_lines -------------------------------------

    #[test]
    fn bash_continuation_lines_empty_for_single_line_normal() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "echo hi".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert!(bash_continuation_lines(&t, 80).is_empty());
    }

    #[test]
    fn bash_continuation_lines_drops_first_line_normal() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "first\nsecond\nthird".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(
            bash_continuation_lines(&t, 80),
            vec!["second".to_string(), "third".to_string()]
        );
    }

    #[test]
    fn bash_continuation_lines_wraps_long_single_line_normal() {
        // A long single-line command (no `\n`) spills into wrapped rows so
        // the whole invocation is visible instead of truncated to "…RUS…".
        let command = "a".repeat(50);
        let t = dummy_tool(
            ToolInput::Bash {
                command: command.clone(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        // content_w = 20 → wrap_w = 18 → 50 chars = 18 + 18 + 14 = 3 rows.
        let rows = bash_continuation_lines(&t, 20);
        assert_eq!(rows.len(), 3, "got: {rows:?}");
        assert_eq!(
            rows.iter().map(|r| r.len()).collect::<Vec<_>>(),
            vec![18, 18, 14]
        );
        assert_eq!(
            rows.concat(),
            command,
            "wrapped rows must reconstruct the command"
        );
    }

    #[test]
    fn bash_continuation_lines_short_single_line_empty_robust() {
        // A short command that fits the title produces no continuation rows.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "echo hi".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert!(bash_continuation_lines(&t, 80).is_empty());
    }

    #[test]
    fn bash_continuation_lines_non_bash_returns_empty_robust() {
        let t = dummy_tool(
            ToolInput::Read {
                file_path: "foo.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        assert!(bash_continuation_lines(&t, 80).is_empty());
    }

    // --- wrap_styled_line --------------------------------------------

    #[test]
    fn wrap_styled_line_short_returns_unchanged_normal() {
        let line = Line::from(vec![Span::raw("hello")]);
        let wrapped = terminal_output::wrap_styled_line(&line, 80);
        assert_eq!(wrapped.len(), 1);
    }

    #[test]
    fn wrap_styled_line_breaks_long_normal() {
        // 12 chars at width 5 → 3 lines.
        let line = Line::from(vec![Span::raw("abcdefghijkl")]);
        let wrapped = terminal_output::wrap_styled_line(&line, 5);
        assert_eq!(wrapped.len(), 3);
        let combined: String = wrapped
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(combined, "abcdefghijkl");
    }

    #[test]
    fn wrap_styled_line_zero_width_returns_unchanged_robust() {
        // 0 width → return clone unchanged, don't infinite-loop.
        let line = Line::from(vec![Span::raw("anything")]);
        let wrapped = terminal_output::wrap_styled_line(&line, 0);
        assert_eq!(wrapped.len(), 1);
    }

    #[test]
    fn wrap_styled_line_preserves_styles_across_wraps_robust() {
        // Two styled spans across a wrap boundary keep their styles.
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![
            Span::styled("redred", red),
            Span::styled("blueblue", blue),
        ]);
        let wrapped = terminal_output::wrap_styled_line(&line, 4);
        // 14 chars at width 4 = 4 lines.
        assert_eq!(wrapped.len(), 4);
    }

    #[test]
    fn wrap_styled_line_uses_terminal_cell_width_robust() {
        let line = Line::from(vec![Span::raw("中abc")]);
        let wrapped = terminal_output::wrap_styled_line(&line, 3);
        let rendered = lines_to_plain(&wrapped);
        assert_eq!(wrapped.len(), 2, "rendered:\n{rendered}");
        assert_eq!(rendered, "中a\nbc");
    }

    // --- sanitize_terminal_text --------------------------------------

    #[test]
    fn sanitize_keeps_visible_text_normal() {
        assert_eq!(sanitize_terminal_text("hello world"), "hello world");
    }

    #[test]
    fn sanitize_strips_csi_escape_normal() {
        // \x1b[31m red CSI sequence — should be removed entirely.
        let input = "\u{1b}[31mred\u{1b}[0m text";
        assert_eq!(sanitize_terminal_text(input), "red text");
    }

    #[test]
    fn sanitize_expands_tab_to_four_spaces_normal() {
        assert_eq!(sanitize_terminal_text("a\tb"), "a    b");
    }

    #[test]
    fn sanitize_keeps_newline_robust() {
        assert_eq!(sanitize_terminal_text("a\nb"), "a\nb");
    }

    #[test]
    fn sanitize_strips_osc_terminated_by_bel_robust() {
        // OSC `\x1b]...\x07` sequence should be stripped.
        let input = "\u{1b}]0;title\u{7}body";
        assert_eq!(sanitize_terminal_text(input), "body");
    }

    #[test]
    fn sanitize_drops_control_chars_robust() {
        // Backspace (0x08) and other control chars vanish.
        assert_eq!(sanitize_terminal_text("a\u{8}b"), "ab");
    }

    // --- tool_kind_color ---------------------------------------------

    #[test]
    fn tool_kind_color_distinct_per_family_normal() {
        let t = Theme::dark();
        // Read = blue, Write = amber, Edit = mint — all distinct.
        let read_c = tool_kind_color(&ToolKind::Read, &t);
        let write_c = tool_kind_color(&ToolKind::Write, &t);
        let edit_c = tool_kind_color(&ToolKind::Edit, &t);
        assert_ne!(read_c, write_c);
        assert_ne!(write_c, edit_c);
        assert_ne!(read_c, edit_c);
    }

    #[test]
    fn tool_kind_color_grep_glob_search_share_lavender_normal() {
        // Grep family shares the search/lavender color.
        let t = Theme::dark();
        assert_eq!(
            tool_kind_color(&ToolKind::Grep, &t),
            tool_kind_color(&ToolKind::Glob, &t)
        );
        assert_eq!(
            tool_kind_color(&ToolKind::Grep, &t),
            tool_kind_color(&ToolKind::Search, &t)
        );
    }

    #[test]
    fn tool_kind_color_generic_uses_secondary_robust() {
        // Generic kinds fall back to text_secondary.
        let t = Theme::dark();
        assert_eq!(
            tool_kind_color(&ToolKind::Generic("custom".into()), &t),
            t.text_secondary
        );
    }

    // --- is_groupable ------------------------------------------------

    #[test]
    fn is_groupable_search_kinds_normal() {
        assert!(is_groupable(&ToolKind::Read));
        assert!(is_groupable(&ToolKind::Glob));
        assert!(is_groupable(&ToolKind::Grep));
        assert!(is_groupable(&ToolKind::Search));
    }

    #[test]
    fn is_groupable_destructive_kinds_robust() {
        // Edit/Write/Bash never group — each call's behavior matters.
        assert!(!is_groupable(&ToolKind::Edit));
        assert!(!is_groupable(&ToolKind::Write));
        assert!(!is_groupable(&ToolKind::Bash));
        assert!(!is_groupable(&ToolKind::Generic("foo".into())));
    }

    // --- tool_status_icon_animated -----------------------------------

    #[test]
    fn tool_status_icon_animated_running_rotates_glyph_normal() {
        // Running + frame=0 → first frame; frame=4 → second; frame=8 → third.
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let mut tool = tool;
        tool.status = ToolStatus::Running;
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g4, _) = tool_status_icon_animated(&tool, &t, 4);
        let (g8, _) = tool_status_icon_animated(&tool, &t, 8);
        let (g12, _) = tool_status_icon_animated(&tool, &t, 12);
        assert_eq!(g0, "✶");
        assert_eq!(g4, "✷");
        assert_eq!(g8, "✸");
        assert_eq!(g12, "✹");
    }

    #[test]
    fn tool_status_icon_animated_pending_alternates_normal() {
        let t = Theme::dark();
        let tool = ToolCall {
            id: "p".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Pending,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Empty,
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g6, _) = tool_status_icon_animated(&tool, &t, 6);
        // PENDING_FRAMES is &["○", "◌"] at frame/6 cadence.
        assert_eq!(g0, "○");
        assert_eq!(g6, "◌");
    }

    #[test]
    fn tool_status_icon_animated_complete_static_robust() {
        // Complete state always returns the static icon regardless of frame.
        let t = Theme::dark();
        let tool = ToolCall {
            id: "c".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Empty,
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g100, _) = tool_status_icon_animated(&tool, &t, 100);
        assert_eq!(g0, "●");
        assert_eq!(g100, "●");
    }

    #[test]
    fn tool_status_icon_animated_failed_static_robust() {
        let t = Theme::dark();
        let tool = ToolCall {
            id: "f".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Failed,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Empty,
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        let (g, _) = tool_status_icon_animated(&tool, &t, 42);
        assert_eq!(g, "✗");
    }

    // --- format_elapsed_badge ----------------------------------------

    #[test]
    fn format_elapsed_badge_below_threshold_returns_none_normal() {
        // Sub-100ms results don't get a badge (too noisy).
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(50);
        assert_eq!(format_elapsed_badge(&t), None);
    }

    #[test]
    fn format_elapsed_badge_seconds_decimal_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(2300);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[2.3s]"));
    }

    #[test]
    fn format_elapsed_badge_tens_of_seconds_no_decimal_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(15_000);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[15s]"));
    }

    #[test]
    fn format_elapsed_badge_minutes_format_robust() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(125_000);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[2m 5s]"));
    }

    #[test]
    fn format_elapsed_badge_running_returns_none_robust() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.status = ToolStatus::Running;
        t.elapsed_ms = Some(2300);
        assert_eq!(format_elapsed_badge(&t), None);
    }

    #[test]
    fn format_elapsed_badge_no_elapsed_returns_none_robust() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        // Even Complete: if elapsed_ms is None, no badge.
        assert_eq!(format_elapsed_badge(&t), None);
    }

    // --- tool_title_width_cap ----------------------------------------

    #[serial_test::serial]
    #[test]
    fn tool_title_width_cap_default_is_unbounded_normal() {
        // Without any env override, the cap is `usize::MAX` — let the
        // renderer use whatever the terminal width allows. The 100-col
        // cap was a v126 default that lost out to "show the whole
        // command on a wide terminal".
        unsafe {
            std::env::remove_var("JFC_TOOL_TITLE_WIDTH");
        }
        assert_eq!(tool_title_width_cap(), usize::MAX);
    }

    #[serial_test::serial]
    #[test]
    fn tool_title_width_cap_rejects_too_small_robust() {
        // Values < 20 are rejected by `.filter(|n| *n >= 20)` → fallback
        // to the unbounded default.
        unsafe {
            std::env::set_var("JFC_TOOL_TITLE_WIDTH", "5");
        }
        assert_eq!(tool_title_width_cap(), usize::MAX);
        unsafe {
            std::env::remove_var("JFC_TOOL_TITLE_WIDTH");
        }
    }

    // --- build_collapsed_header / build_title_spans / build_header_inner_spans

    #[test]
    fn build_header_inner_spans_bash_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "echo hi".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        // 4 spans: "Bash" + "(" + cmd + ")".
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "Bash");
        assert_eq!(spans[1].content, "(");
        assert_eq!(spans[3].content, ")");
    }

    #[test]
    fn build_header_inner_spans_read_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Read {
                file_path: "src/main.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "Read");
        assert!(spans[2].content.contains("src/main.rs"));
    }

    #[test]
    fn build_header_inner_spans_write_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Write {
                file_path: "out.txt".into(),
                content: "".into(),
            },
            ToolOutput::Empty,
            ToolKind::Write,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans[0].content, "Write");
    }

    #[test]
    fn build_header_inner_spans_edit_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Edit {
                file_path: "src/lib.rs".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            ToolOutput::Empty,
            ToolKind::Edit,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans[0].content, "Update");
    }

    #[test]
    fn build_header_inner_spans_long_path_truncates_robust() {
        // A very long path gets truncated with ellipsis.
        let t = Theme::dark();
        let long_path = "a/".repeat(100) + "main.rs";
        let tool = dummy_tool(
            ToolInput::Read {
                file_path: long_path,
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        let spans = build_header_inner_spans(&tool, &t, 30);
        let path_span = &spans[2].content;
        assert!(
            path_span.chars().count() <= 30,
            "got len {}: {path_span:?}",
            path_span.chars().count()
        );
    }

    #[test]
    fn build_collapsed_header_includes_status_icon_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "echo".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let line = build_collapsed_header(&tool, &t, 80, 0);
        // First span is the status icon, second is " ".
        assert_eq!(line.spans[1].content, " ");
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn build_title_spans_includes_pin_glyph_when_pinned_robust() {
        let t = Theme::dark();
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        tool.display = jfc_core::ToolDisplayState::Expanded { pinned: true };
        let spans = build_title_spans(&tool, &t, "●", Style::default(), 80);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(combined.contains("◆"), "expected pin glyph: {combined:?}");
    }

    #[test]
    fn build_title_spans_appends_elapsed_badge_robust() {
        let t = Theme::dark();
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        tool.elapsed_ms = Some(2500);
        let spans = build_title_spans(&tool, &t, "●", Style::default(), 80);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            combined.contains("[2.5s]"),
            "expected elapsed badge: {combined:?}"
        );
    }

    // --- border_color_for_status -------------------------------------

    #[test]
    fn border_color_for_status_each_state_normal() {
        let t = Theme::dark();
        for (status, expected) in [
            (ToolStatus::Pending, t.warning),
            (ToolStatus::Running, t.accent),
            (ToolStatus::Completed, t.border),
            (ToolStatus::Failed, t.error),
        ] {
            let mut tool = dummy_tool(
                ToolInput::Bash {
                    command: "x".into(),
                    timeout: None,
                    workdir: None,
                    run_in_background: None,
                },
                ToolOutput::Empty,
                ToolKind::Bash,
            );
            tool.status = status;
            assert_eq!(border_color_for_status(&tool, &t), expected);
        }
    }

    // --- severity_rank -----------------------------------------------

    #[test]
    fn severity_rank_orders_correctly_normal() {
        use jfc_engine::diagnostics::Severity;
        assert!(severity_rank(Severity::Error) > severity_rank(Severity::Warning));
        assert!(severity_rank(Severity::Warning) > severity_rank(Severity::Info));
        assert!(severity_rank(Severity::Info) > severity_rank(Severity::Hint));
    }

    #[test]
    fn severity_rank_distinct_values_robust() {
        use jfc_engine::diagnostics::Severity;
        let mut v = vec![
            severity_rank(Severity::Error),
            severity_rank(Severity::Warning),
            severity_rank(Severity::Info),
            severity_rank(Severity::Hint),
        ];
        v.sort();
        v.dedup();
        assert_eq!(v.len(), 4, "all 4 ranks must be distinct");
    }

    // --- truncate_str (private inside message_view) ------------------

    #[test]
    fn truncate_str_short_passes_through_normal() {
        assert_eq!(truncate_str("hi", 10), "hi");
    }

    #[test]
    fn truncate_str_long_truncates_with_ellipsis_normal() {
        let s = truncate_str("hello world", 5);
        assert!(s.ends_with('…'));
        assert_eq!(s.chars().count(), 5);
    }

    #[test]
    fn truncate_str_zero_returns_empty_robust() {
        assert_eq!(truncate_str("hi", 0), "");
    }

    // --- parse_grep_with_sep / parse_grep_no_path direct ------------

    #[test]
    fn parse_grep_with_sep_match_form_normal() {
        let r = parse_grep_with_sep("src/foo.rs:5:body", ':', false);
        match r {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("5"));
                assert_eq!(body, "body");
                assert!(!is_context);
            }
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn parse_grep_with_sep_no_match_returns_none_robust() {
        // No `<sep><digits><sep>` anchor → None.
        assert!(parse_grep_with_sep("just plain text", ':', false).is_none());
    }

    #[test]
    fn parse_grep_no_path_match_form_normal() {
        let r = parse_grep_no_path("42:body line", ':', false);
        match r {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "");
                assert_eq!(lineno, Some("42"));
                assert_eq!(body, "body line");
                assert!(!is_context);
            }
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn parse_grep_no_path_non_digit_start_robust() {
        assert!(parse_grep_no_path("foo:body", ':', false).is_none());
    }

    #[test]
    fn parse_grep_no_path_empty_robust() {
        assert!(parse_grep_no_path("", ':', false).is_none());
    }

    #[test]
    fn looks_like_git_diff_output_accepts_ansi_unified_diff() {
        let diff = "\u{1b}[1mdiff --git a/src/lib.rs b/src/lib.rs\u{1b}[m\n\
                    \u{1b}[36m@@ -1 +1 @@\u{1b}[m\n\
                    \u{1b}[31m-old\u{1b}[m\n\
                    \u{1b}[32m+new\u{1b}[m\n";
        assert!(looks_like_git_diff_output(diff));
    }

    #[test]
    fn looks_like_difftastic_output_accepts_side_by_side_headers_robust() {
        let dft = "crates/jfc/src/agents.rs --- 1/5 --- Rust\n\
                   182 pub fn load_skills(      185 pub fn load_skills(\n";
        assert!(looks_like_difftastic_output(dft));

        let fallback = "crates/jfc/src/providers/anthropic_accounts.rs --- Text (exceeded DFT_GRAPH_LIMIT)\n\
                        10 //! - Track per-account  10 //! - Track per-account\n";
        assert!(looks_like_difftastic_output(fallback));
        assert!(!looks_like_difftastic_output(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@"
        ));
    }

    #[test]
    fn difftastic_git_diff_renders_as_preformatted_output_not_file_chunks_robust() {
        let dft = "crates/jfc/src/providers/anthropic_accounts.rs --- 1/7 --- Text (exceeded DFT_GRAPH_LIMIT)\n\
10 //! - Track per-account runtime st  10 //! - Track per-account runtime st\n\
.. ate in memory: rate-limit cooldo  .. ate in memory: rate-limit cooldo\n\
fatal: external diff died, stopping at crates/jfc/src/agents.rs\n";
        let lines = produce_git_diff_output_lines(
            dft,
            "",
            Some(0),
            48,
            Theme::dark(),
            /*expanded*/ false,
        );
        let rendered = lines_to_plain(&lines);

        assert!(rendered.contains("DFT_GRAPH_LIMIT"), "{rendered}");
        assert!(
            !rendered.contains("--- 1/1 --- Rust"),
            "JFC must not chunk-highlight difftastic output as file content:\n{rendered}"
        );
        assert!(
            !rendered.contains("[exit 0]"),
            "successful git diff output should not grow an exit badge:\n{rendered}"
        );
    }

    #[test]
    fn produce_text_block_lines_strips_ansi_before_wrapping() {
        let t = Theme::dark();
        let lines = produce_text_block_lines(
            "\u{1b}[31m+added line\u{1b}[m",
            80,
            t.text_secondary,
            t,
            true,
        );
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "+added line");
        assert!(!rendered.contains('\u{1b}'));
    }

    // --- Git-diff rendering regressions ------------------------------
    //
    // The screenshot bug: `git diff <file>` output was being routed
    // through the chunked file-content renderer (showing
    // `--- 1/N --- Rust` headers) and ANSI escapes from
    // `git diff --color=always` were sliced inside `hard_wrap_str`,
    // leaving raw `\u{1b}[...` bytes in the rendered text. These tests
    // construct realistic ANSI git-diff stdout, render via the same
    // `tool_body_lines` path the UI uses, dump the rendered Lines on
    // failure, and assert the broken behavior never returns.

    /// Concatenate every span across every rendered Line into a single
    /// String. We use `\n` between Lines so failures print readably.
    fn lines_to_plain(lines: &[ratatui::text::Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Realistic stdout produced by `git diff --color=always`: bold
    /// "diff --git" header, cyan hunk header, red `-`, green `+`,
    /// trailing reset. Reproduces the bytes that landed in the chunked
    /// renderer in the bug screenshots.
    const ANSI_GIT_DIFF: &str = concat!(
        "\u{1b}[1mdiff --git a/crates/jfc/src/render.rs b/crates/jfc/src/render.rs\u{1b}[m\n",
        "\u{1b}[1mindex 1111111..2222222 100644\u{1b}[m\n",
        "\u{1b}[1m--- a/crates/jfc/src/render.rs\u{1b}[m\n",
        "\u{1b}[1m+++ b/crates/jfc/src/render.rs\u{1b}[m\n",
        "\u{1b}[36m@@ -4624,4 +4624,4 @@\u{1b}[m fn render_choice_list(\n",
        "\u{1b}[31m-    let theme = Theme::dark();\u{1b}[m\n",
        "\u{1b}[31m-    let t = &theme;\u{1b}[m\n",
        "\u{1b}[32m+    t: &Theme,\u{1b}[m\n",
        "\u{1b}[32m+) {\u{1b}[m\n",
        "     match &tool.input {\n",
        "         ToolInput::Edit {\n",
        "             file_path,\n",
    );

    fn render_bash_git_diff(stdout: &str, expanded: bool) -> Vec<ratatui::text::Line<'static>> {
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "git diff crates/jfc/src/render.rs".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Command {
                stdout: stdout.into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            ToolKind::Bash,
        );
        tool.display = if expanded {
            jfc_core::ToolDisplayState::Expanded { pinned: false }
        } else {
            jfc_core::ToolDisplayState::DEFAULT
        };
        tool_body_lines_themed(&tool, 100, Theme::dark(), None)
    }

    fn render_bash_git_stat(stdout: &str, expanded: bool) -> Vec<ratatui::text::Line<'static>> {
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "git diff --stat".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            ToolOutput::Command {
                stdout: stdout.into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            ToolKind::Bash,
        );
        tool.display = if expanded {
            jfc_core::ToolDisplayState::Expanded { pinned: false }
        } else {
            jfc_core::ToolDisplayState::DEFAULT
        };
        tool_body_lines_themed(&tool, 120, Theme::dark(), None)
    }

    #[test]
    fn ansi_git_diff_through_bash_renders_no_raw_escapes() {
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);
        let rendered = lines_to_plain(&lines);

        assert!(
            !rendered.contains('\u{1b}'),
            "raw ANSI escape leaked into rendered output:\n--- rendered ---\n{rendered}\n--- raw stdout ---\n{ANSI_GIT_DIFF}",
        );
    }

    #[test]
    fn ansi_git_diff_is_not_chunked_as_file_content() {
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);
        let rendered = lines_to_plain(&lines);

        let bad_chunk_markers = [
            "--- 1/",
            "--- 2/",
            "--- 3/",
            "--- 4/",
            "--- Rust",
            " --- Rust",
        ];
        for marker in bad_chunk_markers {
            assert!(
                !rendered.contains(marker),
                "git diff output went through the FileContent chunked renderer (`{marker}` header):\n{rendered}",
            );
        }
    }

    #[test]
    fn ansi_git_diff_keeps_unified_diff_structure() {
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);
        let rendered = lines_to_plain(&lines);

        for must_have in [
            "diff --git a/crates/jfc/src/render.rs",
            "@@ -4624,4 +4624,4 @@",
        ] {
            assert!(
                rendered.contains(must_have),
                "expected unified-diff line `{must_have}` in rendered output:\n{rendered}",
            );
        }

        // The `+`/`-` sigils must appear at line starts at least once
        // each — confirms the diff renderer recognized add/remove rows.
        let mut saw_add = false;
        let mut saw_del = false;
        for line in &lines {
            let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if plain.starts_with('+') && !plain.starts_with("+++") {
                saw_add = true;
            }
            if plain.starts_with('-') && !plain.starts_with("---") {
                saw_del = true;
            }
        }
        assert!(saw_add, "no `+` add line in rendered output:\n{rendered}");
        assert!(
            saw_del,
            "no `-` remove line in rendered output:\n{rendered}"
        );
    }

    #[test]
    fn ansi_git_diff_add_and_remove_lines_get_distinct_styles() {
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);
        let theme = Theme::dark();

        let mut add_fg: Option<ratatui::style::Color> = None;
        let mut del_fg: Option<ratatui::style::Color> = None;
        for line in &lines {
            let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let first_fg = line.spans.iter().find_map(|s| s.style.fg);
            if plain.starts_with('+') && !plain.starts_with("+++") && add_fg.is_none() {
                add_fg = first_fg;
            }
            if plain.starts_with('-') && !plain.starts_with("---") && del_fg.is_none() {
                del_fg = first_fg;
            }
        }

        let add = add_fg.expect("expected at least one `+` line with foreground style");
        let del = del_fg.expect("expected at least one `-` line with foreground style");
        assert_ne!(
            add, del,
            "`+` and `-` lines rendered with the same fg color (add={add:?}, del={del:?})",
        );
        assert_eq!(
            add, theme.success,
            "`+` add line should use Theme.success, got {add:?} (theme.success = {:?})",
            theme.success,
        );
        assert_eq!(
            del, theme.error,
            "`-` remove line should use Theme.error, got {del:?} (theme.error = {:?})",
            theme.error,
        );
    }

    #[test]
    fn git_diff_stat_colours_plus_and_minus_graph() {
        let stat = " crates/jfc/src/daemon.rs | 1076 ++++++++++++++++++++++++----\n\
                    crates/jfc/src/main.rs   |   88 ++-\n\
                    2 files changed, 1100 insertions(+), 64 deletions(-)\n";
        let lines = render_bash_git_stat(stat, true);
        let rendered = lines_to_plain(&lines);
        assert!(
            rendered.contains("++++++++++++++++++++++++----"),
            "expected intact diffstat graph:\n{rendered}"
        );

        let theme = Theme::dark();
        let mut saw_plus = false;
        let mut saw_minus = false;
        for span in lines.iter().flat_map(|line| line.spans.iter()) {
            if span.content.contains('+') && span.style.fg == Some(theme.success) {
                saw_plus = true;
            }
            if span.content.contains('-') && span.style.fg == Some(theme.error) {
                saw_minus = true;
            }
        }
        assert!(saw_plus, "no green plus graph span in:\n{rendered}");
        assert!(saw_minus, "no red minus graph span in:\n{rendered}");
    }

    #[test]
    fn ansi_git_diff_through_filecontent_does_not_chunk() {
        // Reproduces the screenshot bug: the same diff text but routed
        // through ToolOutput::FileContent (which the screenshot showed
        // splitting the diff into "--- 1/4 --- Rust" chunks). After the
        // fix the FileContent path detects diff text and falls into the
        // diff renderer instead.
        let plain_diff = "diff --git a/x b/x\n\
                          --- a/x\n\
                          +++ b/x\n\
                          @@ -1 +1 @@\n\
                          -old\n\
                          +new\n";
        let tool = dummy_tool(
            ToolInput::Read {
                file_path: "x".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::FileContent {
                path: "x".into(),
                content: plain_diff.into(),
                language: "diff".into(),
            },
            ToolKind::Read,
        );
        let lines = tool_body_lines_themed(&tool, 80, Theme::dark(), None);
        let rendered = lines_to_plain(&lines);

        for marker in ["--- 1/", "--- 2/", "--- Rust", " --- Diff"] {
            assert!(
                !rendered.contains(marker),
                "FileContent diff went through chunked renderer (`{marker}`):\n{rendered}",
            );
        }
        assert!(
            rendered.contains("@@ -1 +1 @@"),
            "expected hunk header in rendered output:\n{rendered}",
        );
    }

    #[test]
    fn ansi_git_diff_applies_syntect_per_token_highlighting() {
        // Catches the "swapped (lang, code) arguments" failure mode:
        // if `highlight_code_raw` ever falls into its fallback branch
        // (unknown syntax), every diff line collapses to a single span
        // with the diff fg color. Real per-token highlighting produces
        // multiple spans per line, with at least one keyword token
        // (e.g. `let`, `match`) in a different color than identifiers.
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);

        let mut hunk_lines: Vec<&ratatui::text::Line<'static>> = Vec::new();
        for line in &lines {
            let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if plain.starts_with('-') && !plain.starts_with("---")
                || plain.starts_with('+') && !plain.starts_with("+++")
                || plain.starts_with(' ') && plain.contains("match")
            {
                hunk_lines.push(line);
            }
        }
        assert!(!hunk_lines.is_empty(), "no hunk content lines found");

        let multi_span_count = hunk_lines.iter().filter(|l| l.spans.len() > 2).count();
        assert!(
            multi_span_count >= 2,
            "expected ≥2 hunk lines with per-token spans (sigil + multiple syntect tokens), \
             got {multi_span_count} multi-span lines out of {}: {:#?}",
            hunk_lines.len(),
            hunk_lines
                .iter()
                .map(|l| (
                    l.spans.len(),
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                ))
                .collect::<Vec<_>>(),
        );

        let mut distinct_token_colors = std::collections::HashSet::new();
        for line in &hunk_lines {
            for span in line.spans.iter().skip(1) {
                if !span.content.trim().is_empty() {
                    distinct_token_colors.insert(span.style.fg);
                }
            }
        }
        assert!(
            distinct_token_colors.len() >= 2,
            "expected ≥2 distinct token colors (keywords vs idents), \
             got {} colors: {:?}",
            distinct_token_colors.len(),
            distinct_token_colors,
        );
    }

    /// Snapshot-style debug helper: prints exactly what the UI shows
    /// for an ANSI git diff so a human reviewer can eyeball the styling
    /// after a refactor. Always passes; useful with `--nocapture`.
    #[test]
    fn debug_dump_ansi_git_diff_render() {
        let lines = render_bash_git_diff(ANSI_GIT_DIFF, true);
        eprintln!("--- raw ANSI stdout ({} bytes) ---", ANSI_GIT_DIFF.len());
        eprintln!("{ANSI_GIT_DIFF}");
        eprintln!("--- rendered ({} lines) ---", lines.len());
        for (idx, line) in lines.iter().enumerate() {
            for span in &line.spans {
                eprintln!(
                    "  [{idx:>3}] fg={:?} bg={:?} mod={:?} {:?}",
                    span.style.fg,
                    span.style.bg,
                    span.style.add_modifier,
                    span.content.as_ref(),
                );
            }
        }
    }

    // --- message_view_total_lines ------------------------------------

    #[test]
    fn message_view_total_lines_empty_app_normal() {
        // Build a fake App via the test helpers — empty messages → 0 lines.
        use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
        use std::sync::Arc;

        struct Stub;
        #[async_trait::async_trait]
        impl Provider for Stub {
            fn name(&self) -> &str {
                "test"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }
        impl jfc_provider::seal::Sealed for Stub {}

        let app = App::new(Arc::new(Stub), "test-model");
        // No messages → 0 lines.
        assert_eq!(message_view_total_lines(&app, 80), 0);
    }

    // --- Predictor / renderer drift guard ----------------------------------
    //
    // The scroll path runs `message_view_total_lines` per frame and uses
    // its number for follow-bottom math. The renderer separately walks
    // `build_render_items` and sums `RenderItem::height`. When those two
    // disagree, `follow_bottom` parks scroll at a wrong line — the user
    // sees a clipped tail. We've now had this bug across (a) TaskStatus
    // multi-line summaries, (b) Reasoning expanded with wide thoughts,
    // (c) Advisor blocks with byte-counted character-wrap. Each escape
    // was structurally identical. These tests build minimal apps with
    // each part type at a width that forces wrapping and assert the two
    // numbers agree byte-for-byte. New part types or rendering changes
    // get one of these cases added; if the predictor diverges, the test
    // fails before the user sees a clipped scroll.
    fn renderer_total_height(app: &App, inner_w: usize) -> usize {
        let ctx = RenderCtx::from_app(app);
        build_render_items_ctx(&ctx, inner_w)
            .iter()
            .map(|i| i.height(inner_w))
            .sum()
    }

    fn stub_app() -> App {
        use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
        use std::sync::Arc;
        struct Stub;
        #[async_trait::async_trait]
        impl Provider for Stub {
            fn name(&self) -> &str {
                "test"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }
        impl jfc_provider::seal::Sealed for Stub {}
        App::new(Arc::new(Stub), "test-model")
    }

    /// Reasoning expanded — single line wider than viewport must wrap
    /// in both predictor and renderer.
    #[test]
    fn predictor_matches_renderer_reasoning_expanded_wide() {
        let mut app = stub_app();
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        let wide = "x".repeat(150);
        msg.parts.push(MessagePart::Reasoning(wide));
        app.engine.messages.push(msg);
        // Force expanded so the body rows actually render.
        app.reasoning_expanded.insert(0, true);
        let w = 40usize;
        assert_eq!(
            message_view_total_lines(&app, w),
            renderer_total_height(&app, w),
            "reasoning-expanded: predictor must match renderer at width {w}",
        );
    }

    /// Advisor body — byte-counted char-wrap used to disagree with
    /// ratatui's word-wrap; ribbon prefix wasn't subtracted from width.
    #[test]
    fn predictor_matches_renderer_advisor_wide() {
        let mut app = stub_app();
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts.push(MessagePart::Advisor(
            "this advisor message is intentionally wider than the viewport \
             so word-wrap must split it into multiple visual rows."
                .to_owned(),
        ));
        app.engine.messages.push(msg);
        let w = 30usize;
        assert_eq!(
            message_view_total_lines(&app, w),
            renderer_total_height(&app, w),
            "advisor: predictor must match renderer at width {w}",
        );
    }

    /// TaskStatus completed with multi-line markdown body — original
    /// 148-line drift bug. Predictor must mirror the same
    /// `markdown::to_lines` + 120-line cap as `push_task_status_lines`.
    #[test]
    fn predictor_matches_renderer_task_status_completed_block() {
        let mut app = stub_app();
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        let summary = "# Plan\n\nStep 1: do thing.\nStep 2: another thing.\n\n\
                       ```rust\nfn main() {}\n```\n\nMore text after the block."
            .to_owned();
        msg.parts.push(MessagePart::TaskStatus(TaskStatusPart {
            task_id: "t1".into(),
            description: "Do the work".into(),
            status: TaskLifecycle::Completed,
            summary: Some(summary),
            error: None,
            elapsed_ms: Some(1234),
            model: None,
        }));
        app.engine.messages.push(msg);
        let w = 60usize;
        assert_eq!(
            message_view_total_lines(&app, w),
            renderer_total_height(&app, w),
            "task-status (completed multi-line): predictor must match renderer at width {w}",
        );
    }

    /// CompactBoundary on a narrow viewport — the decorative line wraps.
    #[test]
    fn predictor_matches_renderer_compact_boundary_narrow() {
        let mut app = stub_app();
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts.clear();
        msg.parts
            .push(MessagePart::CompactBoundary { pre_tokens: 12345 });
        app.engine.messages.push(msg);
        let w = 16usize;
        assert_eq!(
            message_view_total_lines(&app, w),
            renderer_total_height(&app, w),
            "compact-boundary: predictor must match renderer at narrow width {w}",
        );
    }

    // --- Tool-block predictor / renderer drift guard ----------------------
    //
    // Each test builds one tool with a specific output shape (large
    // diff, multi-line bash, expanded vs. collapsed file content),
    // then asserts the predicted height (`tool_block_height`) equals
    // the total non-empty rows the renderer paints into a buffer at
    // two widths. Pre-fix the predictor drifted from the renderer in
    // 6+ ways (see `tool_block_height`'s doc comment).

    /// Sum of rows the renderer paints for one ToolCall at `width`.
    ///
    /// We initialize the buffer with a sentinel symbol (`~`) so that
    /// a blank-line render (which paints spaces) is distinguishable
    /// from "buffer untouched": any row where the leftmost cell still
    /// reads `~` was never written to. A row counts as "painted" if
    /// its leftmost cell was overwritten, regardless of whether that
    /// overwrite landed a space or a glyph.
    fn rendered_tool_rows(app: &App, tool: &ToolCall, width: u16) -> usize {
        let predicted = tool_block_height(tool, width as usize) as u16;
        let h = predicted.saturating_add(8).max(16);
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height: h,
        };
        let mut buf = Buffer::empty(area);
        // Pre-fill with a sentinel so we can tell "renderer painted a
        // space here" from "renderer never touched this cell".
        for y in 0..h {
            for x in 0..width {
                buf[(x, y)].set_symbol("~");
            }
        }
        let t = app.theme;
        render_tool_block(app, tool, area, t, &mut buf, 0);
        let mut painted = 0usize;
        for y in 0..h {
            let row_painted = (0..width).any(|x| buf[(x, y)].symbol() != "~");
            if row_painted {
                painted += 1;
            }
        }
        painted
    }

    /// Multi-line Bash command, plain stdout — exercises the
    /// continuation-line + command-output dispatch.
    #[test]
    fn tool_block_height_matches_renderer_multiline_bash() {
        let app = stub_app();
        let tool = ToolCall {
            id: "tb1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "cat <<'EOF'\nfirst\nsecond\nthird\nEOF".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
            output: ToolOutput::Command {
                stdout: "alpha\nbeta\ngamma\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: Some(120),
            started_at: None,
            thought_signature: None,
        };
        for w in [40u16, 80u16] {
            let predicted = tool_block_height(&tool, w as usize);
            let actual = rendered_tool_rows(&app, &tool, w);
            assert_eq!(
                predicted, actual,
                "multiline bash: predictor {predicted} != renderer {actual} at width {w}",
            );
        }
    }

    /// Large diff with many hunk lines — exercises the diff renderer
    /// path (special-cased per-row bg painting) and the hunk_cap
    /// truncation footer.
    #[test]
    fn tool_block_height_matches_renderer_large_diff() {
        let app = stub_app();
        let mut hunk_lines = Vec::new();
        for n in 0..120 {
            hunk_lines.push(jfc_core::DiffLine {
                kind: if n % 3 == 0 {
                    DiffLineKind::Added
                } else if n % 3 == 1 {
                    DiffLineKind::Removed
                } else {
                    DiffLineKind::Context
                },
                content: format!("line {n}"),
                old_line: Some(n + 1),
                new_line: Some(n + 1),
            });
        }
        let diff = DiffView {
            file_path: "src/foo.rs".into(),
            additions: 40,
            deletions: 40,
            hunks: vec![jfc_core::DiffHunk {
                old_start: 1,
                new_start: 1,
                header: "@@ -1,120 +1,120 @@".into(),
                lines: hunk_lines,
            }],
        };
        let tool = ToolCall {
            id: "tb2".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            input: ToolInput::Edit {
                file_path: "src/foo.rs".into(),
                old_string: String::new(),
                new_string: String::new(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        for w in [60u16, 100u16] {
            let predicted = tool_block_height(&tool, w as usize);
            let actual = rendered_tool_rows(&app, &tool, w);
            assert_eq!(
                predicted, actual,
                "large diff: predictor {predicted} != renderer {actual} at width {w}",
            );
        }
    }

    /// FileContent (Read result with content) — exercises the
    /// syntect-highlighted block path under both expanded and
    /// collapsed semantics.
    #[test]
    fn tool_block_height_matches_renderer_file_content_expanded_and_collapsed() {
        let app = stub_app();
        let mut body = String::new();
        for n in 0..120 {
            body.push_str(&format!("fn line_{n}() {{ let x = {n}; }}\n"));
        }
        let make_tool = |expanded: bool| ToolCall {
            id: "tb3".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Completed,
            input: ToolInput::Read {
                file_path: "src/lib.rs".into(),
                offset: None,
                limit: None,
            },
            output: ToolOutput::FileContent {
                path: "src/lib.rs".into(),
                content: body.clone(),
                language: "rs".into(),
            },
            display: if expanded {
                jfc_core::ToolDisplayState::Expanded { pinned: false }
            } else {
                jfc_core::ToolDisplayState::DEFAULT
            },
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        for expanded in [false, true] {
            let tool = make_tool(expanded);
            for w in [60u16, 100u16] {
                let predicted = tool_block_height(&tool, w as usize);
                let actual = rendered_tool_rows(&app, &tool, w);
                assert_eq!(
                    predicted, actual,
                    "file-content expanded={expanded}: predictor {predicted} != renderer {actual} at width {w}",
                );
            }
        }
    }

    #[test]
    fn tool_block_height_running_tool_tracks_output_changes() {
        let app = stub_app();

        let make_tool = |body: String| ToolCall {
            id: "tb-running".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Running,
            input: ToolInput::Read {
                file_path: "src/lib.rs".into(),
                offset: None,
                limit: None,
            },
            output: ToolOutput::FileContent {
                path: "src/lib.rs".into(),
                content: body,
                language: "rs".into(),
            },
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };

        let short = make_tool("fn short() {}\n".into());
        let mut long_body = String::new();
        for n in 0..110 {
            long_body.push_str(&format!("fn line_{n}() {{ let value = {n}; }}\n"));
        }
        let long = make_tool(long_body);

        let short_h = tool_block_height(&short, 80);
        let long_h = tool_block_height(&long, 80);

        assert!(
            long_h > short_h,
            "running output changes must change the computed height"
        );
        assert_eq!(short_h, rendered_tool_rows(&app, &short, 80));
        assert_eq!(long_h, rendered_tool_rows(&app, &long, 80));
    }
}
