#![allow(dead_code)]

use itertools::{Itertools, Position};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options as ParseOptions, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

/// Convert one syntect `(Style, &str)` range into a ratatui `Span`.
/// Replaces the previous `as_24_bit_terminal_escaped` → `IntoText`
/// round-trip, which was the dominant cost in markdown rendering
/// (~10ms per visible message). syntect's `Style` already gives us
/// fg color + font flags; mapping straight to `ratatui::Style` skips
/// ANSI serialization and re-parsing entirely.
fn syntect_span_to_ratatui(style: SyntectStyle, text: &str) -> Span<'static> {
    let mut s = Style::default().fg(ratatui::style::Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(FontStyle::BOLD) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    Span::styled(text.to_owned(), s)
}

use crate::theme::Theme;

// ── Inline color swatch detection ────────────────────────────────────────────
//
// Detects hex (#rrggbb, #rgb) and CSS rgb(r, g, b) color literals in prose
// and emits a colored swatch character (█) so the user sees the actual color
// inline. False-positive prevention:
//   - Hex codes must be preceded by a word boundary or start-of-string (rejects
//     #define, #include, anchor links like #section-name that contain non-hex
//     chars).
//   - 3-digit hex requires ALL digits to be duplicated pairs when expanded
//     (i.e. we accept #fff, #a0c but NOT #if0 which looks like a preprocessor
//     directive fragment).
//   - rgb() requires exactly three numeric components 0-255.

static COLOR_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?x)
        # 6-digit hex: #rrggbb
        (\#[0-9a-fA-F]{6})
        |
        # 3-digit hex: #rgb
        (\#[0-9a-fA-F]{3})
        |
        # CSS rgb(r, g, b)
        (rgb\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*\))
        ",
    )
    .expect("color regex")
});

/// Validate boundary: char before match must be non-alphanumeric (or start of string),
/// and char after must not extend the hex sequence.
fn valid_hex_boundary(text: &str, start: usize, end: usize, hex_len: usize) -> bool {
    // Check preceding char: reject if it's alphanumeric (catches "foo#aabbcc")
    if start > 0 {
        let before = text[..start].chars().last().unwrap();
        if before.is_alphanumeric() || before == '_' {
            return false;
        }
    }
    // Check following char: reject if it's a hex digit (catches truncated git hashes like #0c45357a)
    // For 3-digit, also reject if followed by any word char (catches #if0-like patterns)
    if end < text.len() {
        let after = text[end..].chars().next().unwrap();
        if hex_len == 6 && after.is_ascii_hexdigit() {
            return false;
        }
        if hex_len == 3 && (after.is_alphanumeric() || after == '_') {
            return false;
        }
    }
    true
}

/// Parse a color literal match into an RGB Color. Returns None if values
/// are out of range (>255) or if the match is a likely false positive.
fn parse_color_match(text: &str, m: &regex::Match<'_>) -> Option<Color> {
    let s = m.as_str();
    let start = m.start();
    let end = m.end();

    if s.starts_with('#') && s.len() == 7 {
        // 6-digit hex
        if !valid_hex_boundary(text, start, end, 6) {
            return None;
        }
        let r = u8::from_str_radix(&s[1..3], 16).ok()?;
        let g = u8::from_str_radix(&s[3..5], 16).ok()?;
        let b = u8::from_str_radix(&s[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    if s.starts_with('#') && s.len() == 4 {
        // 3-digit hex
        if !valid_hex_boundary(text, start, end, 3) {
            return None;
        }
        let chars: &[u8] = s.as_bytes();
        let r = u8::from_str_radix(&format!("{0}{0}", chars[1] as char), 16).ok()?;
        let g = u8::from_str_radix(&format!("{0}{0}", chars[2] as char), 16).ok()?;
        let b = u8::from_str_radix(&format!("{0}{0}", chars[3] as char), 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    if s.starts_with("rgb(") {
        // Extract the three numbers manually
        let inner = &s[4..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 3 {
            return None;
        }
        let r: u16 = parts[0].trim().parse().ok()?;
        let g: u16 = parts[1].trim().parse().ok()?;
        let b: u16 = parts[2].trim().parse().ok()?;
        if r > 255 || g > 255 || b > 255 {
            return None;
        }
        return Some(Color::Rgb(r as u8, g as u8, b as u8));
    }
    None
}

/// Split a text string into spans, inserting a colored swatch (█) before each
/// detected color literal. The `base_style` is applied to non-color text.
/// Returns None if no color literals are found (caller uses fast path).
fn colorize_inline_colors(text: &str, base_style: Style) -> Option<Vec<Span<'static>>> {
    let mut matches_found = false;
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_end = 0;

    for m in COLOR_RE.find_iter(text) {
        let color = match parse_color_match(text, &m) {
            Some(c) => c,
            None => continue,
        };
        matches_found = true;

        // Push any preceding text
        if m.start() > last_end {
            spans.push(Span::styled(
                text[last_end..m.start()].to_owned(),
                base_style,
            ));
        }
        // Push the swatch character in the detected color
        spans.push(Span::styled("█ ".to_owned(), Style::default().fg(color)));
        // Push the color code text itself with base style
        spans.push(Span::styled(m.as_str().to_owned(), base_style));
        last_end = m.end();
    }

    if !matches_found {
        return None;
    }

    // Push trailing text
    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_owned(), base_style));
    }
    Some(spans)
}

/// Primary syntax set — `two_face::syntax::extra_newlines()` on top of
/// syntect's defaults. ~250 grammars covering Zig, Nix, Fish, Hare, Roc,
/// and the rest of the long tail. Mirrors codex CLI's primitive (research
/// at `research/openai-codex/codex-rs/tui/src/render/highlight.rs:33-44`).
static SYNTAX_SET: std::sync::LazyLock<SyntaxSet> =
    std::sync::LazyLock::new(two_face::syntax::extra_newlines);

/// Build-time side-loaded syntaxes from `crates/jfc-ui/syntaxes/` —
/// kept as the third lookup tier behind the two-face bundle so a project
/// can drop in a custom `.sublime-syntax` for an obscure DSL without
/// touching the upstream grammars.
static EXTRA_SYNTAX_SET: std::sync::LazyLock<Option<SyntaxSet>> = std::sync::LazyLock::new(|| {
    let path = option_env!("EXTRA_SYNTAXES_PACK")?;
    let bytes = std::fs::read(path).ok()?;
    syntect::dumps::from_reader::<SyntaxSet, _>(&mut std::io::Cursor::new(bytes)).ok()
});

/// Theme set — two-face bundles 32 themes (Catppuccin variants, Dracula,
/// Nord, Solarized, Tokyo Night, etc.) on top of syntect's defaults.
/// Each theme is registered under its `as_name()` so callers can do
/// `THEME_SET.themes.get("Catppuccin Mocha")` like with stock syntect.
static THEME_SET: std::sync::LazyLock<ThemeSet> = std::sync::LazyLock::new(|| {
    let extra = two_face::theme::extra();
    let mut ts = ThemeSet::load_defaults();
    for name in two_face::theme::EmbeddedLazyThemeSet::theme_names() {
        ts.themes
            .insert(name.as_name().to_owned(), extra.get(*name).clone());
    }
    ts
});

fn find_syntax_in_sets<'a>(
    lang: &str,
    lower: &str,
    primary: &'a SyntaxSet,
    extra: Option<&'a SyntaxSet>,
) -> (&'a syntect::parsing::SyntaxReference, &'a SyntaxSet) {
    let lookup = |ss: &'a SyntaxSet| -> Option<&'a syntect::parsing::SyntaxReference> {
        ss.find_syntax_by_token(lang)
            .or_else(|| ss.find_syntax_by_extension(lang))
            .or_else(|| ss.find_syntax_by_name(lang))
            .or_else(|| {
                ss.syntaxes()
                    .iter()
                    .find(|s| s.name.to_lowercase() == lower)
            })
    };

    if let Some(s) = lookup(primary) {
        return (s, primary);
    }
    if let Some(extra_set) = extra {
        if let Some(s) = lookup(extra_set) {
            return (s, extra_set);
        }
    }
    (primary.find_syntax_plain_text(), primary)
}

pub fn highlight_code(
    lang: &str,
    code: &str,
    inner_width: usize,
    theme: &crate::theme::Theme,
) -> Vec<Line<'static>> {
    highlight_code_inner(lang, code, inner_width, theme, true)
}

pub fn highlight_code_raw(
    lang: &str,
    code: &str,
    inner_width: usize,
    theme: &crate::theme::Theme,
) -> Vec<Line<'static>> {
    highlight_code_inner(lang, code, inner_width, theme, false)
}

/// Bounded memo of `highlight_code_inner` results.
///
/// Why: `tool_body_lines_themed` is invoked per-frame from `RenderItem::height`
/// (for scroll math) AND again to actually paint, so every visible code block
/// in tool output ran the full syntect/onig regex pipeline twice per frame.
/// With a busy conversation that pinned the main thread on `match_at` /
/// `onig_search_with_param` even while the UI was idle (observed at 190% CPU
/// via gdb stack sampling). This cache turns repeat calls into a hash lookup.
///
/// Key components:
/// - `lang_lower`: grammar selection input (already lowercased once at the call site).
/// - `code_hash`: DefaultHasher of the input text — same inputs collide, different
///   inputs miss naturally as files grow during streaming.
/// - `wrap_w`: hard-wrap column; different widths must produce different cached vecs.
/// - `with_gutter`: gates the leading `│ ` span (only the markdown code-fence path
///   uses it; tool bodies pass false).
/// - `border` + `text_secondary`: the only theme-dependent colors used inside
///   `highlight_code_inner`. The actual syntect highlighting theme is hardcoded
///   to `base16-ocean.dark`, so we only key on what the user's `Theme` actually
///   contributes.
struct HighlightCacheKey {
    lang_lower: String,
    code_hash: u64,
    wrap_w: usize,
    with_gutter: bool,
    border: Color,
    text_secondary: Color,
}

impl PartialEq for HighlightCacheKey {
    fn eq(&self, o: &Self) -> bool {
        self.code_hash == o.code_hash
            && self.wrap_w == o.wrap_w
            && self.with_gutter == o.with_gutter
            && self.border == o.border
            && self.text_secondary == o.text_secondary
            && self.lang_lower == o.lang_lower
    }
}
impl Eq for HighlightCacheKey {}
impl Hash for HighlightCacheKey {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.code_hash.hash(h);
        self.wrap_w.hash(h);
        self.with_gutter.hash(h);
        self.lang_lower.hash(h);
    }
}

struct HighlightCacheEntry {
    lines: Vec<Line<'static>>,
    generation: u64,
}

struct HighlightCache {
    map: HashMap<HighlightCacheKey, HighlightCacheEntry>,
    generation: u64,
}

const HIGHLIGHT_CACHE_MAX: usize = 512;

static HIGHLIGHT_CACHE: LazyLock<Mutex<HighlightCache>> = LazyLock::new(|| {
    Mutex::new(HighlightCache {
        map: HashMap::with_capacity(128),
        generation: 0,
    })
});

/// Drop every memoized highlight result. Call when something changes that the
/// cache key cannot encode — e.g. a fresh syntax set is loaded. Theme/width
/// switches are already covered by the key, but `RenderCache::clear()` callers
/// invoke this too for symmetry so neither cache outlives the other.
pub fn clear_highlight_cache() {
    let mut c = HIGHLIGHT_CACHE.lock().expect("highlight cache poisoned");
    c.map.clear();
    c.generation = 0;
}

fn hash_code(code: &str) -> u64 {
    let mut h = DefaultHasher::new();
    code.hash(&mut h);
    h.finish()
}

fn highlight_code_inner(
    lang: &str,
    code: &str,
    inner_width: usize,
    theme: &crate::theme::Theme,
    with_gutter: bool,
) -> Vec<Line<'static>> {
    use syntect::easy::HighlightLines;

    let gutter_style = Style::default().fg(theme.border);
    let fallback_style = Style::default().fg(theme.text_secondary);
    // `inner_width == 0` means "don't wrap" — used by the diff
    // renderer to keep a 1:1 input-line-to-output-line mapping
    // so per-row tinting (the green/red bg) lines up with the
    // hunk's diff lines. Callers that want wrapping pass a real
    // column width; we floor that at 20 cells so a comically-narrow
    // input still produces *something* readable.
    let wrap_w = if inner_width == 0 {
        0
    } else {
        inner_width.max(20)
    };

    let lower = lang.to_lowercase();
    let cache_key = HighlightCacheKey {
        lang_lower: lower.clone(),
        code_hash: hash_code(code),
        wrap_w,
        with_gutter,
        border: theme.border,
        text_secondary: theme.text_secondary,
    };
    {
        let mut cache = HIGHLIGHT_CACHE.lock().expect("highlight cache poisoned");
        cache.generation = cache.generation.wrapping_add(1);
        let gen_now = cache.generation;
        if let Some(entry) = cache.map.get_mut(&cache_key) {
            entry.generation = gen_now;
            return entry.lines.clone();
        }
    }

    let (syntax, active_set) =
        find_syntax_in_sets(lang, &lower, &SYNTAX_SET, EXTRA_SYNTAX_SET.as_ref());

    let theme_name = "base16-ocean.dark";
    let hl_theme = THEME_SET
        .themes
        .get(theme_name)
        .or_else(|| THEME_SET.themes.values().next())
        .unwrap();

    let mut highlighter = HighlightLines::new(syntax, hl_theme);
    let mut out: Vec<Line<'static>> = Vec::new();

    for raw_line in LinesWithEndings::from(code) {
        let sanitized: String = raw_line
            .chars()
            .map(|c| {
                if c == '\n' {
                    '\n'
                } else if c == '\t' {
                    ' '
                } else if c.is_control() {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let ranges = match highlighter.highlight_line(&sanitized, active_set) {
            Ok(ranges) => ranges,
            Err(_) => {
                let clean = sanitized.trim_end_matches('\n');
                for chunk in hard_wrap_str(clean, wrap_w) {
                    let mut spans = Vec::new();
                    if with_gutter {
                        spans.push(Span::styled("│ ", gutter_style));
                    }
                    spans.push(Span::styled(chunk, fallback_style));
                    out.push(Line::from(spans));
                }
                continue;
            }
        };

        let mut line_spans: Vec<Span<'static>> = Vec::with_capacity(ranges.len());
        for (style, text) in &ranges {
            let trimmed = text.trim_end_matches('\n');
            if trimmed.is_empty() {
                continue;
            }
            line_spans.push(syntect_span_to_ratatui(*style, trimmed));
        }
        let line = Line::from(line_spans);

        for chunk in hard_wrap_line(line, wrap_w) {
            let mut spans = Vec::new();
            if with_gutter {
                spans.push(Span::styled("│ ", gutter_style));
            }
            spans.extend(chunk.spans);
            out.push(Line::from(spans));
        }
    }

    {
        let mut cache = HIGHLIGHT_CACHE.lock().expect("highlight cache poisoned");
        if cache.map.len() >= HIGHLIGHT_CACHE_MAX {
            let target = HIGHLIGHT_CACHE_MAX * 3 / 4;
            let mut gens: Vec<u64> = cache.map.values().map(|e| e.generation).collect();
            gens.sort_unstable();
            let cutoff_idx = cache.map.len().saturating_sub(target);
            let cutoff = gens.get(cutoff_idx).copied().unwrap_or(u64::MAX);
            cache.map.retain(|_, e| e.generation > cutoff);
        }
        let gen_now = cache.generation;
        cache.map.insert(
            cache_key,
            HighlightCacheEntry {
                lines: out.clone(),
                generation: gen_now,
            },
        );
    }

    out
}

/// Returns true if `text` has an unclosed code fence at the end.
///
/// Used during streaming to avoid rendering half-written code blocks as broken
/// markdown. When this returns true, the caller should buffer rather than render.
pub fn has_unclosed_fence(text: &str) -> bool {
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            inside = !inside;
        }
    }
    inside
}

/// Strip raw `<tool_call>…</tool_call>` and `<tool_result>…</tool_result>`
/// blocks before markdown rendering. OpenWebUI's Bedrock proxy occasionally
/// fails to translate Anthropic-style tool calls into OpenAI function-call
/// SSE events and instead inlines them as XML inside the text stream — a
/// 300+ line wall of `<tool_call>{"name":"bash",...}</tool_call><tool_result>…`
/// that buries the actual prose. The fix at the provider layer is bigger
/// (parse the XML and re-emit StreamEvent::ToolDone); this sanitizer is the
/// belt-and-suspenders for whatever leaks through.
///
/// Behavior: any matched block is replaced with a single ⟪tool⟫ marker so
/// the user can see *something* happened without the wall of JSON. We don't
/// attempt to balance nested tags — the model never emits them.
pub fn strip_inline_tool_xml(text: &str) -> String {
    fn drop_block(input: &str, open: &str, close: &str, marker: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find(open) {
            out.push_str(&rest[..start]);
            let after_open = &rest[start + open.len()..];
            match after_open.find(close) {
                Some(end) => {
                    out.push_str(marker);
                    rest = &after_open[end + close.len()..];
                }
                None => {
                    // Unterminated block — drop everything from <open> to EOF.
                    // Better than rendering half a JSON blob.
                    out.push_str(marker);
                    return out;
                }
            }
        }
        out.push_str(rest);
        out
    }
    let s = drop_block(text, "<tool_call>", "</tool_call>", "⟪tool_call⟫");
    drop_block(&s, "<tool_result>", "</tool_result>", "⟪tool_result⟫")
}

pub fn to_lines(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let cleaned = strip_inline_tool_xml(text);
    // v126 disables strikethrough because the GFM `~~text~~` syntax collides
    // with `~~~` fenced code blocks: a stray paragraph containing `~~~` would
    // open a code block on the next round-trip. We follow the same call.
    let mut opts = ParseOptions::empty();
    opts.insert(ParseOptions::ENABLE_TASKLISTS);
    opts.insert(ParseOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(&cleaned, opts);

    let mut w = MdWriter::new(parser, theme);
    w.code_wrap_width = width;
    w.run();
    w.text.lines
}

/// Streaming-optimized markdown renderer. Produces the same structural output
/// as `to_lines` (headings, lists, blockquotes, inline emphasis, tables) but
/// replaces syntect-based code highlighting with plain monospace rendering.
///
/// Cost profile: pulldown-cmark parsing is O(n) and fast (~5µs/KB); syntect
/// highlighting is O(n × grammar_complexity) and dominates at ~200µs/KB for
/// complex grammars (Rust, C++). By skipping syntect, streaming frames render
/// in <1ms even for 10KB messages with multiple code blocks.
///
/// The visual difference is minimal during streaming: code blocks appear in a
/// uniform secondary color with the same `┌─ lang / │ / └─` chrome. Once
/// `StreamDone` fires, the caller switches to `to_lines` for the final render
/// which applies full syntax highlighting.
pub fn to_lines_streaming(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let cleaned = strip_inline_tool_xml(text);
    let mut opts = ParseOptions::empty();
    opts.insert(ParseOptions::ENABLE_TASKLISTS);
    opts.insert(ParseOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(&cleaned, opts);

    let mut w = MdWriter::new(parser, theme);
    w.code_wrap_width = width;
    w.skip_syntect = true;
    w.run();
    w.text.lines
}

#[cfg(test)]
mod tool_xml_strip_tests {
    use super::strip_inline_tool_xml;

    #[test]
    fn drops_single_tool_call_normal() {
        let s = strip_inline_tool_xml("Before <tool_call>{\"name\":\"bash\"}</tool_call> after");
        assert_eq!(s, "Before ⟪tool_call⟫ after");
    }

    #[test]
    fn drops_chained_call_then_result_normal() {
        let s = strip_inline_tool_xml(concat!(
            "Hi <tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"ls\"}}</tool_call>",
            "<tool_result>file1\nfile2</tool_result> done"
        ));
        assert_eq!(s, "Hi ⟪tool_call⟫⟪tool_result⟫ done");
    }

    #[test]
    fn drops_many_pairs_normal() {
        let s = strip_inline_tool_xml(concat!(
            "<tool_call>a</tool_call><tool_result>b</tool_result>",
            "<tool_call>c</tool_call><tool_result>d</tool_result>",
        ));
        assert_eq!(s, "⟪tool_call⟫⟪tool_result⟫⟪tool_call⟫⟪tool_result⟫");
    }

    #[test]
    fn handles_unterminated_block_robust() {
        // OWUI sometimes truncates mid-stream — better to drop everything
        // from the unterminated open than render half-JSON.
        let s = strip_inline_tool_xml("Hi <tool_call>{\"name\":");
        assert_eq!(s, "Hi ⟪tool_call⟫");
    }

    #[test]
    fn passes_clean_text_through_robust() {
        let s = strip_inline_tool_xml("Just a normal sentence with `code` and *emphasis*.");
        assert_eq!(s, "Just a normal sentence with `code` and *emphasis*.");
    }

    #[test]
    fn does_not_strip_lookalike_text_robust() {
        // Substring "<tool_" inside a code block or prose shouldn't trigger
        // unless the full opening tag is present.
        let s = strip_inline_tool_xml("Use the <tool_calls> block in the API");
        assert_eq!(s, "Use the <tool_calls> block in the API");
    }

    #[test]
    fn handles_huge_inline_block_robust() {
        // A 5000-byte tool_call block (representative of the screenshot
        // wall) should reduce to the marker without quadratic explosion.
        let big = "x".repeat(5000);
        let input = format!("Before<tool_call>{big}</tool_call>After");
        let s = strip_inline_tool_xml(&input);
        assert_eq!(s, "Before⟪tool_call⟫After");
    }
}

struct TableState {
    alignments: Vec<pulldown_cmark::Alignment>,
    head_row: Vec<Vec<Span<'static>>>,
    body_rows: Vec<Vec<Vec<Span<'static>>>>,
    current_row: Vec<Vec<Span<'static>>>,
    current_cell: Vec<Span<'static>>,
    in_head: bool,
}

impl TableState {
    fn new(alignments: Vec<pulldown_cmark::Alignment>) -> Self {
        Self {
            alignments,
            head_row: Vec::new(),
            body_rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: Vec::new(),
            in_head: false,
        }
    }

    fn flush_cell(&mut self) {
        let cell = std::mem::take(&mut self.current_cell);
        self.current_row.push(cell);
    }

    fn flush_row(&mut self) {
        let row = std::mem::take(&mut self.current_row);
        if self.in_head {
            self.head_row = row;
        } else {
            self.body_rows.push(row);
        }
    }

    fn cell_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Render the table as a `Vec<Line>`. `target_width` is the
    /// maximum total width (in cells) the table can occupy. When the
    /// natural column widths fit within `target_width`, columns
    /// keep their natural sizes. When they overflow, columns are
    /// proportionally shrunk and per-cell content wraps to multiple
    /// rows. `target_width = 0` disables wrap (uses natural widths
    /// — useful for tests).
    ///
    /// Width math is *cell-aware*: uses `unicode-width` so CJK,
    /// emoji, and fullwidth chars don't push columns wider than
    /// their visual cell count. Earlier this used `text.len()`
    /// (bytes) which over-counted UTF-8 and made tables wider than
    /// they needed to be.
    fn render(self, theme: &Theme, target_width: usize) -> Vec<Line<'static>> {
        use unicode_width::UnicodeWidthStr;

        let ncols = self.alignments.len().max(
            self.head_row
                .len()
                .max(self.body_rows.iter().map(|r| r.len()).max().unwrap_or(0)),
        );
        if ncols == 0 {
            return Vec::new();
        }

        // Natural cell widths (in display cells, not bytes).
        let cell_w = |cell: &Vec<Span<'_>>| -> usize {
            UnicodeWidthStr::width(Self::cell_text(cell).as_str())
        };
        let mut natural = vec![0usize; ncols];
        for (i, cell) in self.head_row.iter().enumerate() {
            if i < ncols {
                natural[i] = natural[i].max(cell_w(cell));
            }
        }
        for row in &self.body_rows {
            for (i, cell) in row.iter().enumerate() {
                if i < ncols {
                    natural[i] = natural[i].max(cell_w(cell));
                }
            }
        }
        // Min column width = 3 cells so a column is always at least
        // visible; min content + 2 chrome cells per column for `│ `.
        for w in &mut natural {
            *w = (*w).max(3);
        }

        // Chrome cost per column = 3 cells: `│ ` (left border + pad)
        // + 1 trailing pad before the next divider. Plus 1 for the
        // closing `│` at the right edge.
        //   total = sum(content_widths) + 3 * ncols + 1
        let chrome_cost = 3 * ncols + 1;
        let widths = if target_width == 0
            || natural.iter().sum::<usize>() + chrome_cost <= target_width
        {
            // Fits naturally — no wrap needed.
            natural.clone()
        } else {
            // Overflow — distribute available cells across columns
            // proportionally to their natural widths, with a min of
            // 3 cells per column. If even the mins don't fit (very
            // narrow terminal), the table will overflow but at
            // least every column gets something.
            let avail = target_width.saturating_sub(chrome_cost);
            let total_natural: usize = natural.iter().sum();
            if total_natural == 0 {
                vec![3usize; ncols]
            } else {
                let mut out = vec![0usize; ncols];
                let mut allocated = 0usize;
                for (i, &n) in natural.iter().enumerate() {
                    let share = (n as f64 / total_natural as f64 * avail as f64).round() as usize;
                    out[i] = share.max(3);
                    allocated += out[i];
                }
                // Fix-up: if rounding overshoots, trim from the
                // widest column. Better than the table sneaking 1
                // cell past the target.
                while allocated > avail.max(3 * ncols) {
                    if let Some((widest_i, _)) = out.iter().enumerate().max_by_key(|(_, w)| **w) {
                        if out[widest_i] > 3 {
                            out[widest_i] -= 1;
                            allocated -= 1;
                            continue;
                        }
                    }
                    break;
                }
                out
            }
        };

        let border_style = Style::default().fg(theme.border);
        let head_style = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(theme.text_primary);

        let mut lines = Vec::new();

        // Top border `┌──┬──┐` so the table is enclosed on all four
        // sides. Without this the table read as "free-standing rows
        // with internal dividers" — easy to miss in dense prose.
        lines.push(Line::from(border_row(
            &widths,
            "┌─",
            "─┬─",
            "─┐",
            border_style,
        )));

        // Render head row (may span multiple visual lines if any
        // header cell wraps).
        if !self.head_row.is_empty() {
            for visual_row in render_row(&self.head_row, &widths, head_style, border_style) {
                lines.push(visual_row);
            }

            // Header/body separator `├──┼──┤`.
            lines.push(Line::from(border_row(
                &widths,
                "├─",
                "─┼─",
                "─┤",
                border_style,
            )));
        }

        // Body rows — each row may produce multiple visual rows due
        // to per-cell wrapping.
        for row in &self.body_rows {
            for visual_row in render_row(row, &widths, body_style, border_style) {
                lines.push(visual_row);
            }
        }

        // Bottom border `└──┴──┘`.
        lines.push(Line::from(border_row(
            &widths,
            "└─",
            "─┴─",
            "─┘",
            border_style,
        )));

        lines
    }
}

/// Build a horizontal border row (top, separator, or bottom). The
/// three glyph args are the left corner, the inner divider (between
/// columns), and the right corner.
fn border_row(
    widths: &[usize],
    left: &str,
    div: &str,
    right: &str,
    style: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![Span::styled(left.to_owned(), style)];
    for (i, &w) in widths.iter().enumerate() {
        spans.push(Span::styled("─".repeat(w), style));
        if i + 1 < widths.len() {
            spans.push(Span::styled(div.to_owned(), style));
        }
    }
    spans.push(Span::styled(right.to_owned(), style));
    spans
}

/// Render one logical table row as N visual rows (where N is the
/// max wrapped-line count across cells). Each cell's text is wrapped
/// to its allocated column width using cell-aware
/// `hard_wrap_str`, then padded to fill the column. Cells that wrap
/// to fewer rows than the row's max get blank padding lines so all
/// cells stay aligned.
fn render_row(
    cells: &[Vec<Span<'static>>],
    widths: &[usize],
    text_style: Style,
    border_style: Style,
) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;

    let ncols = widths.len();
    // Pre-wrap each cell to its column width. Pad cell vec to ncols
    // so rows with missing cells still render with empty placeholders.
    let wrapped: Vec<Vec<String>> = (0..ncols)
        .map(|i| {
            let text = cells
                .get(i)
                .map(|c| TableState::cell_text(c))
                .unwrap_or_default();
            let w = widths[i];
            hard_wrap_str(&text, w)
        })
        .collect();
    let row_height = wrapped.iter().map(|w| w.len()).max().unwrap_or(1).max(1);

    let mut out: Vec<Line<'static>> = Vec::with_capacity(row_height);
    for line_idx in 0..row_height {
        let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", border_style)];
        for (i, col_lines) in wrapped.iter().enumerate() {
            let w = widths[i];
            let chunk = col_lines.get(line_idx).cloned().unwrap_or_default();
            let chunk_w = UnicodeWidthStr::width(chunk.as_str());
            let pad = w.saturating_sub(chunk_w);
            let padded = format!("{}{}", chunk, " ".repeat(pad));
            spans.push(Span::styled(padded, text_style));
            spans.push(Span::styled(" │ ".to_owned(), border_style));
        }
        out.push(Line::from(spans));
    }
    out
}

struct MdWriter<'a, I> {
    iter: I,
    text: Text<'static>,
    theme: &'a Theme,
    inline_styles: Vec<Style>,
    line_styles: Vec<Style>,
    line_prefixes: Vec<Span<'static>>,
    list_indices: Vec<Option<u64>>,
    code_highlighter: Option<HighlightLines<'a>>,
    link: Option<CowStr<'a>>,
    table: Option<TableState>,
    needs_newline: bool,
    /// Width for hard-wrapping code-block content. ratatui's `Paragraph::wrap`
    /// word-wraps everything, which mangles ASCII trees inside fenced blocks
    /// (the screenshots showed this). When `code_wrap_width > 0` we hard-wrap
    /// each code line to this width here so the paragraph never has to.
    code_wrap_width: usize,
    /// True while we're inside a `Tag::CodeBlock` — gates hard-wrapping.
    in_code_block: bool,
    /// When true, code blocks render as plain monospace text without invoking
    /// syntect. Used by `to_lines_streaming` to eliminate the dominant cost
    /// center (grammar-based highlighting) during per-chunk streaming renders.
    skip_syntect: bool,
}

impl<'a, I> MdWriter<'a, I>
where
    I: Iterator<Item = Event<'a>>,
{
    fn new(iter: I, theme: &'a Theme) -> Self {
        Self {
            iter,
            text: Text::default(),
            theme,
            inline_styles: Vec::new(),
            line_styles: Vec::new(),
            line_prefixes: Vec::new(),
            list_indices: Vec::new(),
            code_highlighter: None,
            link: None,
            table: None,
            needs_newline: false,
            code_wrap_width: 0,
            in_code_block: false,
            skip_syntect: false,
        }
    }

    fn run(&mut self) {
        while let Some(event) = self.iter.next() {
            self.handle(event);
        }
    }

    fn handle(&mut self, event: Event<'a>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.on_text(text),
            Event::Code(code) => self.on_code(code),
            Event::SoftBreak => self.push_span(Span::raw(" ")),
            Event::HardBreak => self.push_line(Line::default()),
            Event::Rule => {
                // CC v126 parity: HR renders as the literal `---` string, not
                // a full-width line. The visual separator comes from the
                // dashes plus surrounding blank lines, matching what Marked
                // emits.
                if self.needs_newline {
                    self.push_line(Line::default());
                }
                self.push_line(Line::styled("---", self.theme.style_text_muted));
                self.needs_newline = true;
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(Span::raw(marker.to_string()));
            }
            _ => {} // Html, footnotes, math — skip silently
        }
    }

    fn start(&mut self, tag: Tag<'a>) {
        match tag {
            Tag::Paragraph => {
                if self.table.is_some() {
                    return; // Don't emit paragraph markers inside table cells
                }
                if self.needs_newline {
                    self.push_line(Line::default());
                }
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            Tag::Heading { level, .. } => {
                if self.needs_newline {
                    self.push_line(Line::default());
                }
                let lvl = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    pulldown_cmark::HeadingLevel::H3 => 3,
                    pulldown_cmark::HeadingLevel::H4 => 4,
                    pulldown_cmark::HeadingLevel::H5 => 5,
                    pulldown_cmark::HeadingLevel::H6 => 6,
                };
                // CC v126 parity: heading is just bold styled text, no `### `
                // prefix, no left-edge bar, no underline. The `# `s are
                // consumed by the parser and only the title content remains,
                // styled bold (and underlined for H1 only). Hierarchy comes
                // from the styling + newlines, not visible markers.
                let style = self.heading_style(lvl);
                self.push_line(Line::default());
                self.push_inline_style(style);
                self.needs_newline = false;
            }
            Tag::BlockQuote(_) => {
                if self.needs_newline {
                    self.push_line(Line::default());
                    self.needs_newline = false;
                }
                self.line_prefixes
                    .push(Span::styled("> ", self.theme.style_text_muted));
                self.line_styles
                    .push(self.theme.style_text_secondary.italic());
            }
            Tag::CodeBlock(kind) => {
                if !self.text.lines.is_empty() {
                    self.push_line(Line::default());
                }
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) => l.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                if !self.skip_syntect {
                    self.set_code_highlighter(lang);
                }

                // Frame the block with the same `┌─ … │ … └─` chrome the tool
                // renderer uses, so code blocks read as distinct visual units
                // instead of trailing inline against prose. v126 SOP step 5
                // (`Fs` component) does the same — header row, left gutter,
                // matching close.
                // Just the lang tag — the leading `▸` triangle was
                // redundant decoration. The `┌─` border chrome
                // already marks the start of the block; tagging the
                // language ("rust", "bash", "code") is the only
                // info the header needs to carry.
                let header_label = if lang.is_empty() {
                    "code".to_string()
                } else {
                    lang.to_owned()
                };
                let header_style = self.theme.style_accent.add_modifier(Modifier::BOLD);
                // Emphasize the language tag a bit more — the previous
                // bare accent color blended into prose. Bold + the
                // small lang badge makes code blocks pop visually.
                self.push_line(Line::from(vec![
                    Span::styled("┌─ ", Style::default().fg(self.theme.border)),
                    Span::styled(header_label, header_style),
                ]));
                self.needs_newline = false;
                self.in_code_block = true;
            }
            Tag::List(start_index) => {
                if self.list_indices.is_empty() && self.needs_newline {
                    self.push_line(Line::default());
                }
                self.list_indices.push(start_index);
            }
            Tag::Item => {
                // CC v126 parity: unordered list items use `-` (dash), not
                // `•` (bullet). Ordered list items use depth-aware ordinals
                // (1./a./i.) per `formatOrdinal(depth, ordinal)`.
                self.push_line(Line::default());
                let depth = self.list_indices.len();
                let indent = "  ".repeat(depth.saturating_sub(1));
                if let Some(last) = self.list_indices.last_mut() {
                    let prefix = match last {
                        None => format!("{indent}- "),
                        Some(idx) => {
                            let label = format_ordinal(*idx, depth);
                            *idx += 1;
                            format!("{indent}{label}. ")
                        }
                    };
                    self.push_span(Span::styled(prefix, self.theme.style_text_muted));
                }
                self.needs_newline = false;
            }
            Tag::Emphasis => self.push_inline_style(Style::new().italic()),
            Tag::Strong => self.push_inline_style(Style::new().bold()),
            Tag::Strikethrough => self.push_inline_style(Style::new().crossed_out()),
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url);
                self.push_inline_style(self.theme.style_accent);
            }
            Tag::Table(alignments) => {
                if self.needs_newline {
                    self.push_line(Line::default());
                }
                self.table = Some(TableState::new(alignments));
                self.needs_newline = false;
            }
            Tag::TableHead => {
                if let Some(ref mut t) = self.table {
                    t.in_head = true;
                }
            }
            Tag::TableRow => {}
            Tag::TableCell => {
                if let Some(ref mut t) = self.table {
                    t.current_cell.clear();
                }
            }
            _ => {} // Image, footnotes, metadata, definitions — skip
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.table.is_none() {
                    self.needs_newline = true;
                }
            }
            TagEnd::Heading(_) => {
                self.pop_inline_style();
                self.needs_newline = true;
            }
            TagEnd::BlockQuote(_) => {
                self.line_prefixes.pop();
                self.line_styles.pop();
                self.needs_newline = true;
            }
            TagEnd::CodeBlock => {
                self.push_line(Line::from(Span::styled(
                    "└─".to_string(),
                    self.theme.style_border,
                )));
                self.needs_newline = true;
                self.code_highlighter = None;
                self.in_code_block = false;
            }
            TagEnd::List(_) => {
                self.list_indices.pop();
                self.needs_newline = true;
            }
            TagEnd::Item => {}
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.pop_inline_style();
            }
            TagEnd::Link => {
                self.pop_inline_style();
                if let Some(url) = self.link.take() {
                    // OSC 8 hyperlink wrapping. Mirrors v126's `CB(url, text)`.
                    // Terminals that support OSC 8 (iTerm2, kitty, WezTerm,
                    // Windows Terminal, recent gnome-terminal) render the text
                    // as clickable; others ignore the escape sequences and
                    // show plain text. We append ` (url)` after the closer so
                    // non-OSC-8 users still see the destination.
                    //
                    // Note: pulldown-cmark fires Tag::Link at the START, so
                    // the *previous* spans on this line are the link label.
                    // We retroactively wrap them by closing the OSC8 here and
                    // hoping the renderer kept them on this line — for now we
                    // just emit the inert hint.
                    self.push_span(Span::styled(
                        format!(" ({url})"),
                        self.theme.style_text_muted,
                    ));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    // Use `code_wrap_width` (which is just the
                    // markdown render width passed to to_lines) as
                    // the table's target width. When the natural
                    // table width exceeds this, columns shrink and
                    // long cell content wraps. Cell-aware widths
                    // mean CJK / emoji / fullwidth cells are sized
                    // correctly.
                    let target_w = self.code_wrap_width;
                    let rendered = table.render(self.theme, target_w);
                    for line in rendered {
                        self.text.lines.push(line);
                    }
                    self.needs_newline = true;
                }
            }
            TagEnd::TableHead => {
                if let Some(ref mut t) = self.table {
                    t.flush_row();
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(ref mut t) = self.table {
                    t.flush_row();
                }
            }
            TagEnd::TableCell => {
                if let Some(ref mut t) = self.table {
                    t.flush_cell();
                }
            }
            _ => {}
        }
    }

    fn on_text(&mut self, text: CowStr<'a>) {
        // If we're inside a table cell, accumulate into table state
        if let Some(ref mut table) = self.table {
            let style = self.inline_styles.last().copied().unwrap_or_default();
            table
                .current_cell
                .push(Span::styled(text.into_string(), style));
            return;
        }

        if let Some(highlighter) = &mut self.code_highlighter {
            // Hard-wrap each highlighted line to viewport width *minus the
            // gutter* (the `│ ` prefix we prepend below) so ratatui's
            // word-wrap doesn't mangle ASCII trees. Then prefix each line
            // with the gutter span, matching the tool-call visual frame.
            let inner_width = self.code_wrap_width.saturating_sub(2).max(20);
            let gutter_style = self.theme.style_border;
            for raw_line in LinesWithEndings::from(text.as_ref()) {
                let Ok(ranges) = highlighter.highlight_line(raw_line, &SYNTAX_SET) else {
                    continue;
                };
                let mut line_spans: Vec<Span<'static>> = Vec::with_capacity(ranges.len());
                for (style, slice) in &ranges {
                    let trimmed = slice.trim_end_matches('\n');
                    if trimmed.is_empty() {
                        continue;
                    }
                    line_spans.push(syntect_span_to_ratatui(*style, trimmed));
                }
                let line = Line::from(line_spans);
                for chunk in hard_wrap_line(line, inner_width) {
                    let mut spans = vec![Span::styled("│ ", gutter_style)];
                    spans.extend(chunk.spans);
                    self.text.push_line(Line::from(spans));
                }
            }
            self.needs_newline = false;
            return;
        }
        if self.in_code_block {
            // Code block with no syntax highlighter (unknown language) — still
            // gutter-prefix and hard-wrap so it doesn't word-wrap to oblivion.
            let inner_width = self.code_wrap_width.saturating_sub(2).max(20);
            let gutter_style = self.theme.style_border;
            for line in text.lines() {
                let style = self.theme.style_text_secondary;
                for chunk in hard_wrap_str(line, inner_width) {
                    self.push_line(Line::from(vec![
                        Span::styled("│ ", gutter_style),
                        Span::styled(chunk, style),
                    ]));
                }
            }
            self.needs_newline = false;
            return;
        }

        for (position, line) in text.lines().with_position() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if matches!(position, Position::Middle | Position::Last) {
                self.push_line(Line::default());
            }
            let style = self
                .inline_styles
                .last()
                .copied()
                .unwrap_or(self.theme.style_text_primary);
            // Detect inline color literals and render swatches
            if let Some(color_spans) = colorize_inline_colors(line, style) {
                for span in color_spans {
                    self.push_span(span);
                }
            } else {
                self.push_span(Span::styled(line.to_string(), style));
            }
        }
        self.needs_newline = false;
    }

    fn on_code(&mut self, code: CowStr<'a>) {
        let code_style = self.theme.inline_code();
        if let Some(ref mut table) = self.table {
            // Inside a table cell: try swatch, otherwise plain
            if let Some(spans) = colorize_inline_colors(code.as_ref(), code_style) {
                for span in spans {
                    table.current_cell.push(span);
                }
            } else {
                table
                    .current_cell
                    .push(Span::styled(code.into_string(), code_style));
            }
            return;
        }
        // Inline code outside tables: detect color literals and render swatch
        if let Some(spans) = colorize_inline_colors(code.as_ref(), code_style) {
            for span in spans {
                self.push_span(span);
            }
        } else {
            self.push_span(Span::styled(code.into_string(), code_style));
        }
    }

    fn heading_style(&self, level: usize) -> Style {
        match level {
            1 => self
                .theme
                .style_text_primary
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => self.theme.style_text_primary.add_modifier(Modifier::BOLD),
            _ => self.theme.style_text_secondary.add_modifier(Modifier::BOLD),
        }
    }

    fn set_code_highlighter(&mut self, lang: &str) {
        let lower = lang.to_lowercase();
        let syntax = SYNTAX_SET
            .find_syntax_by_token(lang)
            .or_else(|| SYNTAX_SET.find_syntax_by_extension(lang))
            .or_else(|| SYNTAX_SET.find_syntax_by_name(lang))
            .or_else(|| {
                SYNTAX_SET
                    .syntaxes()
                    .iter()
                    .find(|s| s.name.to_lowercase() == lower)
            })
            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
        let ts = &THEME_SET.themes["base16-ocean.dark"];
        self.code_highlighter = Some(HighlightLines::new(syntax, ts));
    }

    fn push_inline_style(&mut self, style: Style) {
        let current = self.inline_styles.last().copied().unwrap_or_default();
        self.inline_styles.push(current.patch(style));
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn push_line(&mut self, line: Line<'static>) {
        let style = self.line_styles.last().copied().unwrap_or_default();
        let mut line = line.patch_style(style);
        let prefixes: Vec<Span<'static>> = self.line_prefixes.iter().cloned().collect();
        if !prefixes.is_empty() {
            for (i, prefix) in prefixes.into_iter().rev().enumerate() {
                if i == 0 {
                    line.spans.insert(0, Span::raw(" "));
                }
                line.spans.insert(0, prefix);
            }
        }
        self.text.lines.push(line);
    }

    fn push_span(&mut self, span: Span<'static>) {
        if let Some(line) = self.text.lines.last_mut() {
            line.push_span(span);
        } else {
            self.push_line(Line::from(vec![span]));
        }
    }
}

/// Format an ordinal label given a 1-based index and a list nesting depth.
/// Mirrors v126 cli.js (`formatOrdinal(depth, ordinal)`):
/// - depth 1 (top level) → Arabic (`1`, `2`, …)
/// - depth 2 → lowercase letters (`a`, `b`, …, then `aa`, `ab`, …)
/// - depth 3 → lowercase Roman numerals (`i`, `ii`, `iii`, …)
/// - depth ≥ 4 → wrap back to Arabic so deeply nested lists stay readable
fn format_ordinal(n: u64, depth: usize) -> String {
    match depth {
        2 => to_alpha(n),
        3 => to_roman(n),
        _ => n.to_string(),
    }
}

fn to_alpha(mut n: u64) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut buf = Vec::new();
    while n > 0 {
        let r = ((n - 1) % 26) as u8;
        buf.push((b'a' + r) as char);
        n = (n - 1) / 26;
    }
    buf.reverse();
    buf.into_iter().collect()
}

fn to_roman(n: u64) -> String {
    if n == 0 {
        return "n".into();
    }
    let pairs: &[(u64, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    let mut rem = n;
    for &(v, s) in pairs {
        while rem >= v {
            out.push_str(s);
            rem -= v;
        }
    }
    out
}

/// Hard-wrap a single styled `Line` to `width` columns. When `width == 0` the
/// line passes through unchanged (the renderer didn't supply a budget). Splits
/// at character boundaries — terminal columns are approximated by `chars()`.
pub(crate) fn hard_wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line];
    }
    let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if total <= width {
        return vec![line];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in line.spans {
        let style = span.style;
        let s: String = span.content.into_owned();
        let mut chars = s.chars().peekable();
        let mut buf = String::new();
        while let Some(ch) = chars.next() {
            buf.push(ch);
            col += 1;
            if col >= width && chars.peek().is_some() {
                current.push(Span::styled(std::mem::take(&mut buf), style));
                out.push(Line::from(std::mem::take(&mut current)));
                col = 0;
            }
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, style));
        }
    }
    if !current.is_empty() {
        out.push(Line::from(current));
    }
    out
}

/// Hard-wrap a plain string to `width` *terminal cells*, returning
/// owned chunks. Cell-width aware: counts each char's display
/// width via `unicode-width` so multi-cell glyphs (CJK / emoji /
/// fullwidth punctuation) are accounted for correctly. Earlier this
/// counted characters, which made a 60-char CJK string register as
/// "60 cells" when its real cell width was 120 — the renderer then
/// clipped the right half because the chunk overflowed the textarea.
///
/// Edge: a single char wider than `width` (a 2-cell emoji in a
/// 1-cell-wide window) is emitted on its own line; the next chunk
/// starts after it. Better than infinite-looping on undivisible
/// content.
pub(crate) fn hard_wrap_str(s: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        return vec![s.to_owned()];
    }
    let total: usize = s
        .chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum();
    if total <= width {
        return vec![s.to_owned()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut col = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
        // If adding this char would exceed `width`, emit the
        // current buffer and start fresh. The `cw > 0` check
        // skips zero-width combining marks (they attach to the
        // preceding grapheme rather than starting a new column).
        if cw > 0 && col + cw > width && !buf.is_empty() {
            out.push(std::mem::take(&mut buf));
            col = 0;
        }
        buf.push(ch);
        col += cw;
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Comprehensive markdown-renderer tests — DO-178B normal/robust split.
    //!
    //! Reference behavior cross-checked against:
    //! - marked v18 (`research/marked/src/rules.ts`, `Lexer.ts`) — the JS lib
    //!   the user's research notes call out as the canonical streaming
    //!   tokenizer. marked treats unclosed inline delimiters as literal text
    //!   and unclosed fences as code-to-EOF; we mirror that contract.
    //! - highlight.js (`research/highlight.js/src/highlight.js`) — explicit
    //!   language hint + plaintext fallback.
    //! - Ink (`research/ink/src/wrap-text.ts`) — code blocks should not be
    //!   word-wrapped; we hard-wrap them column-aligned instead.

    use super::*;
    use crate::theme::Theme;

    fn render(input: &str) -> Vec<Line<'static>> {
        to_lines(input, &Theme::dark(), 80)
    }

    fn render_w(input: &str, width: usize) -> Vec<Line<'static>> {
        to_lines(input, &Theme::dark(), width)
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    fn first_line_with(text: &str, lines: &[Line]) -> Option<usize> {
        lines.iter().position(|l| line_text(l).contains(text))
    }

    // ── Plain text ────────────────────────────────────────────────────────

    // Normal: a bare paragraph renders as a single non-empty line of text
    // (paragraph start emits a leading blank, then the content line).
    #[test]
    fn plain_paragraph_normal() {
        let lines = render("hello world");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t == "hello world"), "got {texts:?}");
    }

    // Robust: empty input produces no lines (or only blanks). Must not panic.
    #[test]
    fn empty_input_no_panic_robust() {
        let lines = render("");
        assert!(
            lines.iter().all(|l| line_text(l).is_empty()),
            "expected only blank lines, got {:?}",
            lines.iter().map(line_text).collect::<Vec<_>>()
        );
    }

    // Robust: input that is *only* whitespace doesn't crash and produces blanks.
    #[test]
    fn whitespace_only_robust() {
        let _ = render("   \n   \n  ");
    }

    // ── Headings ──────────────────────────────────────────────────────────

    // Normal: `## Title` no longer emits the literal `## ` prefix — instead
    // the line starts with a left-edge accent bar (`▍`) and the title text.
    // Real markdown renderers strip syntax characters and rely on styling.
    #[test]
    fn heading_h2_separates_from_paragraph_normal() {
        let lines = render("## Codebase Summary\n\nNow I understand the codebase.");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        // The h2 line must contain the heading text, not the literal `##`.
        assert!(
            texts.iter().any(|t| t.contains("Codebase Summary")),
            "heading text lost: {texts:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("##")),
            "literal ## leaked into render: {texts:?}"
        );
        // The paragraph text must be on its own later line.
        assert!(
            texts.iter().any(|t| t == "Now I understand the codebase."),
            "paragraph not rendered separately, got {texts:?}"
        );
    }

    // Normal: H1–H6 each render their title without the literal `#` prefix.
    // Heading hierarchy is communicated via styling + bar variant.
    #[test]
    fn heading_levels_h1_through_h6_normal() {
        for n in 1..=6 {
            let lines = render(&format!("{} title-{n}", "#".repeat(n)));
            let txt: Vec<String> = lines.iter().map(line_text).collect();
            assert!(
                txt.iter().any(|t| t.contains(&format!("title-{n}"))),
                "h{n}: {txt:?}"
            );
            assert!(
                !txt.iter().any(|t| t.contains(&"#".repeat(n))),
                "h{n} leaked literal #s: {txt:?}"
            );
        }
    }

    // Normal: H1 title text is bold AND underlined per `heading_style(1)`.
    #[test]
    fn heading_h1_styled_bold_underlined_normal() {
        let lines = render("# Big");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("Big"))
            .expect("h1");
        let big = line
            .spans
            .iter()
            .find(|s| s.content.contains("Big"))
            .expect("Big span");
        assert!(big.style.add_modifier.contains(Modifier::BOLD));
        assert!(big.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    // Robust: no `#` characters appear in rendered output for any heading
    // level — the syntax char is fully stripped.
    #[test]
    fn heading_no_literal_hash_in_render_robust() {
        for src in [
            "# H1",
            "## H2",
            "### H3",
            "#### H4",
            "##### H5",
            "###### H6",
        ] {
            let lines = render(src);
            let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
            assert!(
                !combined.contains('#'),
                "literal # leaked for {src:?}: {combined:?}"
            );
        }
    }

    // Normal: CC v126 parity — heading is just bold styled text on its own
    // line, no prefix character at all. Hierarchy is communicated purely by
    // styling (H1 bold+underlined, H2 bold, H3+ bold+secondary).
    #[test]
    fn heading_no_prefix_only_styled_text_normal() {
        let lines = render("## Section");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("Section"))
            .expect("h2");
        // The heading line should NOT start with a bar/marker — first
        // non-empty span is the title text itself.
        let first_with_content = line
            .spans
            .iter()
            .find(|s| !s.content.trim().is_empty())
            .expect("title span");
        assert_eq!(first_with_content.content.as_ref(), "Section");
        assert!(
            first_with_content
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    // ── Inline emphasis ───────────────────────────────────────────────────

    // Normal: **bold** and *italic* render with the right modifiers, with
    // surrounding text in plain style.
    #[test]
    fn bold_and_italic_inline_normal() {
        let lines = render("a **b** c *d* e");
        // Locate the line containing the prose.
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("a "))
            .expect("prose line");
        let mut saw_bold = false;
        let mut saw_italic = false;
        for s in &line.spans {
            if s.content.as_ref() == "b" && s.style.add_modifier.contains(Modifier::BOLD) {
                saw_bold = true;
            }
            if s.content.as_ref() == "d" && s.style.add_modifier.contains(Modifier::ITALIC) {
                saw_italic = true;
            }
        }
        assert!(saw_bold, "no bold span: {:?}", line.spans);
        assert!(saw_italic, "no italic span: {:?}", line.spans);
    }

    // Robust: an unclosed `**` at end-of-input renders as literal text rather
    // than entering a permanent bold state. Mirrors marked's emStrong rule
    // (rules.ts:288) which only emits a token when both delimiters match.
    #[test]
    fn unclosed_bold_renders_as_text_robust() {
        let lines = render("hello **world");
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        // The literal "**" must appear in output — i.e. it wasn't consumed
        // as an opening delimiter.
        assert!(
            combined.contains("**"),
            "unclosed bold was eaten: {combined:?}"
        );
    }

    // Robust: a trailing single backtick with no closing pair is treated as
    // text. Mirrors marked's inline-code regex at rules.ts:264 which requires
    // matched `\1`.
    #[test]
    fn trailing_unclosed_backtick_is_text_robust() {
        let lines = render("a literal trailing `");
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains('`'), "lone backtick eaten: {combined:?}");
    }

    // Normal: `inline code` renders with the theme's inline_code style.
    #[test]
    fn inline_code_styled_normal() {
        let theme = Theme::dark();
        let lines = render("use `foo()` here");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("foo"))
            .expect("line with code");
        let code = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "foo()")
            .expect("code span");
        assert_eq!(code.style, theme.inline_code());
    }

    // ── Code blocks ───────────────────────────────────────────────────────

    // Normal: a fenced ```rust block renders as a framed block with a
    // language header and a matching closer — like the tool-call frame.
    // (v126 SOP step 5: `Fs` component frames code with a header + gutter.)
    #[test]
    fn fenced_code_block_has_framed_header_and_closer_normal() {
        let lines = render("```rust\nfn main() {}\n```");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.starts_with("┌─ rust")),
            "no framed header: {texts:?}"
        );
        assert!(texts.iter().any(|t| t == "└─"), "no closer: {texts:?}");
    }

    // Normal: a fenced block with no language label still gets a frame, with
    // the header reading "code" so the user can tell it's a code block.
    #[test]
    fn fenced_code_block_no_language_label_normal() {
        let lines = render("```\nfoo\n```");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.starts_with("┌─ code")),
            "no framed header for unlabeled fence: {texts:?}"
        );
    }

    // Normal: each content line inside the fence carries the `│ ` gutter
    // prefix so it visually nests under the header.
    #[test]
    fn fenced_code_block_lines_have_gutter_normal() {
        let lines = render("```\nfoo\nbar\n```");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let body_lines: Vec<&String> = texts
            .iter()
            .filter(|t| t.contains("foo") || t.contains("bar"))
            .collect();
        assert!(!body_lines.is_empty(), "no body lines: {texts:?}");
        for t in body_lines {
            assert!(t.starts_with("│ "), "body line missing gutter: {t:?}");
        }
    }

    // Normal: code block contents preserve verbatim formatting (no
    // re-flowing). marked's Tokens.Code passes through untouched. We accept
    // a `│ ` gutter prefix in front of each content line.
    #[test]
    fn fenced_code_block_preserves_content_normal() {
        let src = "```\n  indented\n\twith\ttabs\n```";
        let lines = render(src);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.contains("indented")));
        assert!(texts.iter().any(|t| t.contains("with")));
    }

    // Robust: unclosed fence — pulldown-cmark closes it at EOF (matches marked
    // rules.ts:95 alternation `(?: \1[~`]* *(?=\n|$)|$)`). Render must not
    // panic and must include the body content.
    #[test]
    fn unclosed_fence_consumes_to_eof_robust() {
        let lines = render("```\nfn x() {\n  body\n");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.contains("body")),
            "body lost: {texts:?}"
        );
    }

    // Robust: long code-block lines are hard-wrapped to viewport width so
    // ratatui's word-wrap doesn't mangle ASCII trees (the screenshot bug).
    // Each rendered line is `│ ` (2 cols) + content; content must be ≤
    // budget − 2 so the whole line fits the budget.
    #[test]
    fn long_code_line_hard_wraps_to_width_robust() {
        let long: String = "x".repeat(120);
        let lines = render_w(&format!("```\n{long}\n```"), 40);
        // Inside-fence body lines are gutter-prefixed and contain x's.
        let inside_fence: Vec<String> = lines
            .iter()
            .map(line_text)
            .filter(|t| t.starts_with("│ ") && t.contains('x'))
            .collect();
        assert!(!inside_fence.is_empty(), "no body lines emitted: {lines:?}");
        for t in &inside_fence {
            assert!(
                t.chars().count() <= 40,
                "line not wrapped: {} chars: {t:?}",
                t.chars().count()
            );
        }
    }

    // ── Lists ─────────────────────────────────────────────────────────────

    // Normal: CC v126 parity — unordered list items use `-` (dash), not `•`.
    #[test]
    fn bulleted_list_renders_each_item_normal() {
        let lines = render("- one\n- two\n- three");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let bulleted: Vec<&String> = texts.iter().filter(|t| t.contains("- ")).collect();
        assert_eq!(bulleted.len(), 3, "got {texts:?}");
    }

    // Normal: ordered list increments index per item.
    #[test]
    fn ordered_list_indices_increment_normal() {
        let lines = render("1. first\n2. second\n3. third");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.contains("1. first")));
        assert!(texts.iter().any(|t| t.contains("2. second")));
        assert!(texts.iter().any(|t| t.contains("3. third")));
    }

    // Normal: nested list increases indent and uses `-` per CC v126.
    #[test]
    fn nested_list_indents_normal() {
        let lines = render("- top\n  - nested");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let nested = texts
            .iter()
            .find(|t| t.contains("nested"))
            .expect("nested item");
        assert!(
            nested.contains("  - "),
            "no `  - ` indent on nested: {nested:?}"
        );
    }

    // Normal: task list `[ ]` / `[x]` markers render before the item text.
    #[test]
    fn task_list_markers_render_normal() {
        let lines = render("- [x] done\n- [ ] todo");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.contains("[x] done")));
        assert!(texts.iter().any(|t| t.contains("[ ] todo")));
    }

    // ── Block quotes ──────────────────────────────────────────────────────

    // Normal: block quote prefixes `>` on each contained line.
    #[test]
    fn block_quote_prefixes_normal() {
        let lines = render("> quoted text");
        assert!(
            lines.iter().any(|l| line_text(l).starts_with("> ")),
            "{:?}",
            lines.iter().map(line_text).collect::<Vec<_>>()
        );
    }

    // ── Tables ────────────────────────────────────────────────────────────

    // Normal: table renders with header cells and body row.
    #[test]
    fn table_renders_header_and_body_normal() {
        let src = "| Col A | Col B |\n|-------|-------|\n| a1    | b1    |";
        let lines = render(src);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Col A") && t.contains("Col B")),
            "no header: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("a1") && t.contains("b1")),
            "no body: {texts:?}"
        );
    }

    // ── Horizontal rule ───────────────────────────────────────────────────

    // Normal: CC v126 parity — HR renders as literal `---`, not a full-width
    // box-drawing line.
    #[test]
    fn horizontal_rule_emits_three_dashes_normal() {
        let lines = render("before\n\n---\n\nafter");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t == "---"), "no `---` rule: {texts:?}");
    }

    // ── Links ─────────────────────────────────────────────────────────────

    // Normal: `[label](url)` renders the label + ` (url)` suffix so a TUI
    // user can read the destination — this is jfc's deliberate convention.
    #[test]
    fn link_emits_label_and_url_suffix_normal() {
        let lines = render("[click](https://example.com)");
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains("click"));
        assert!(combined.contains("https://example.com"));
    }

    // ── Soft / hard breaks ────────────────────────────────────────────────

    // Normal: a soft break (single `\n` inside paragraph) becomes a space.
    #[test]
    fn soft_break_collapses_to_space_normal() {
        let lines = render("first\nsecond");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts
                .iter()
                .any(|t| t == "first second" || t.contains("first second")),
            "soft-break not space: {texts:?}"
        );
    }

    // ── has_unclosed_fence helper ────────────────────────────────────────

    #[test]
    fn has_unclosed_fence_open_only_normal() {
        assert!(has_unclosed_fence("```rust\nfn x() {"));
    }

    #[test]
    fn has_unclosed_fence_balanced_normal() {
        assert!(!has_unclosed_fence("```\nfoo\n```"));
    }

    #[test]
    fn has_unclosed_fence_two_pairs_balanced_normal() {
        assert!(!has_unclosed_fence("```\nA\n```\n\n```\nB\n```"));
    }

    #[test]
    fn has_unclosed_fence_tilde_variant_normal() {
        assert!(has_unclosed_fence("~~~rust\nfn x() {"));
    }

    // ── hard_wrap helpers ────────────────────────────────────────────────

    #[test]
    fn hard_wrap_str_zero_width_passthrough_robust() {
        assert_eq!(hard_wrap_str("hello", 0), vec!["hello".to_string()]);
    }

    #[test]
    fn hard_wrap_str_under_width_one_chunk_normal() {
        assert_eq!(hard_wrap_str("hi", 5), vec!["hi".to_string()]);
    }

    #[test]
    fn hard_wrap_str_over_width_splits_normal() {
        assert_eq!(
            hard_wrap_str("abcdefghij", 4),
            vec!["abcd".to_string(), "efgh".to_string(), "ij".to_string()]
        );
    }

    #[test]
    fn hard_wrap_str_unicode_chars_count_codepoints_robust() {
        // Each emoji counts as 1 char (not 1 byte) — this is approximate but
        // matches the rest of the renderer's `chars()` width assumption.
        let s = "αβγδε";
        let out = hard_wrap_str(s, 2);
        assert_eq!(
            out,
            vec!["αβ".to_string(), "γδ".to_string(), "ε".to_string()]
        );
    }

    // ── v126 ordinal formatting ───────────────────────────────────────────

    #[test]
    fn ordinal_depth_one_is_arabic_normal() {
        assert_eq!(format_ordinal(1, 1), "1");
        assert_eq!(format_ordinal(7, 1), "7");
    }

    #[test]
    fn ordinal_depth_two_is_lowercase_alpha_normal() {
        assert_eq!(format_ordinal(1, 2), "a");
        assert_eq!(format_ordinal(2, 2), "b");
        assert_eq!(format_ordinal(26, 2), "z");
        // Past z, we wrap with double letters.
        assert_eq!(format_ordinal(27, 2), "aa");
        assert_eq!(format_ordinal(28, 2), "ab");
    }

    #[test]
    fn ordinal_depth_three_is_roman_normal() {
        assert_eq!(format_ordinal(1, 3), "i");
        assert_eq!(format_ordinal(2, 3), "ii");
        assert_eq!(format_ordinal(4, 3), "iv");
        assert_eq!(format_ordinal(9, 3), "ix");
        assert_eq!(format_ordinal(40, 3), "xl");
    }

    #[test]
    fn ordinal_depth_four_wraps_to_arabic_robust() {
        // Per v126: depth ≥ 4 reverts to Arabic so deeply nested lists stay
        // readable instead of growing into multi-letter / multi-numeral runs.
        assert_eq!(format_ordinal(1, 4), "1");
        assert_eq!(format_ordinal(99, 5), "99");
    }

    // Robust: zero index doesn't panic — we only emit it as a defensive value.
    #[test]
    fn ordinal_zero_index_does_not_panic_robust() {
        let _ = format_ordinal(0, 1);
        let _ = format_ordinal(0, 2);
        let _ = format_ordinal(0, 3);
    }

    // ── Real-world response edge cases (the device-info screenshot) ──────

    // Normal: heading line `### 🖥️ Your Machine` — emoji + ASCII title both
    // survive into the rendered heading without the literal `### ` prefix.
    #[test]
    fn heading_with_leading_emoji_normal() {
        let lines = render("### 🖥️ Your Machine");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let h = texts
            .iter()
            .find(|t| t.contains("Your Machine"))
            .expect("heading line");
        assert!(h.contains("🖥"), "emoji lost: {h:?}");
        assert!(!h.contains("###"), "literal ### leaked: {h:?}");
    }

    // Normal: a paragraph that mixes bold + inline code on a single line —
    // the screenshot pattern `**Hostname:** ` + `` `gentoo-thinkpad` ``.
    // Both spans must survive distinct from each other and from surrounding
    // prose, with the inline code keeping `theme.inline_code()` styling.
    #[test]
    fn paragraph_mixes_bold_and_inline_code_normal() {
        let theme = Theme::dark();
        let lines = render("**Hostname:** `gentoo-thinkpad` running");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("Hostname"))
            .expect("paragraph");
        let host = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "Hostname:")
            .expect("bold span");
        assert!(host.style.add_modifier.contains(Modifier::BOLD));
        let code = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "gentoo-thinkpad")
            .expect("inline code span");
        assert_eq!(code.style, theme.inline_code());
    }

    // Normal: bullet list with mixed inline styles per row — the screenshot
    // pattern `- **Model:** Intel Core Ultra 9 275HX (Arrow Lake-HX)`. Bullet
    // marker + bold key + plain value, all on the same line.
    #[test]
    fn bullet_list_with_bold_label_and_plain_value_normal() {
        let lines = render("- **RAM:** 128 GB total (39 GB used)");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("RAM"))
            .expect("list item");
        // CC v126 marker is `- ` (dash + space), not `• `.
        assert!(line.spans.iter().any(|s| s.content.contains("- ")));
        // Bold "RAM:" present.
        assert!(
            line.spans
                .iter()
                .any(|s| s.content.as_ref() == "RAM:"
                    && s.style.add_modifier.contains(Modifier::BOLD))
        );
        // Plain "128 GB total" tail present.
        assert!(
            line.spans
                .iter()
                .any(|s| s.content.contains("128 GB total"))
        );
    }

    // Normal: paragraph that ends with an emoji 🚀 — pulldown-cmark treats it
    // as text; renderer must place it on the same line as the prose.
    #[test]
    fn paragraph_with_trailing_emoji_normal() {
        let lines = render("Quite the beast! 🚀");
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains("Quite the beast"));
        assert!(combined.contains("🚀"));
    }

    // Robust: a heading followed immediately by a horizontal rule (the
    // screenshot has `---` between sections). Heading line contains "Section"
    // (no `##` prefix), HR is a row of long dashes, body is its own line.
    #[test]
    fn heading_followed_by_hr_robust() {
        let lines = render("## Section\n\n---\n\nbody");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.contains("Section")));
        assert!(texts.iter().any(|t| t == "---"));
        assert!(texts.iter().any(|t| t == "body"));
    }

    // Robust: deeply nested inline (bold inside list inside paragraph) doesn't
    // collapse styles — each nested inline keeps its own modifier.
    #[test]
    fn nested_inline_styles_preserve_robust() {
        let lines = render("- a **b *c* d** e");
        let line = lines
            .iter()
            .find(|l| line_text(l).contains("- a"))
            .expect("list item");
        // The "c" span should have BOTH bold and italic (inline styles stack).
        let c = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "c")
            .expect("c span");
        assert!(c.style.add_modifier.contains(Modifier::BOLD));
        assert!(c.style.add_modifier.contains(Modifier::ITALIC));
    }

    // ── Strikethrough disabled (v126 contract) ───────────────────────────

    // Robust: GFM `~~text~~` is intentionally NOT styled — `~~~` collides with
    // fenced code blocks, so we mirror v126's decision to leave strikethrough
    // off. The literal tildes pass through as text.
    #[test]
    fn strikethrough_disabled_renders_literal_robust() {
        let lines = render("~~deleted~~");
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains("~~"), "got {combined:?}");
    }

    // ── Mixed / streaming smoke ───────────────────────────────────────────

    // Robust: a long mixed document with headings, lists, code, prose, and
    // tables doesn't panic and produces a non-empty render.
    #[test]
    fn mixed_document_smoke_robust() {
        let src = r#"# Title

Intro paragraph with **bold**, *italic*, and `code`.

## Section

- bullet one
- bullet two

```rust
fn main() {
    println!("hi");
}
```

| A | B |
|---|---|
| 1 | 2 |

> a quote

A trailing paragraph.
"#;
        let lines = render(src);
        assert!(!lines.is_empty());
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        for needle in [
            "Title",
            "Intro",
            "bold",
            "italic",
            "code",
            "Section",
            "bullet one",
            "fn main()",
            "println!",
            "1",
            "2",
            "trailing",
        ] {
            assert!(
                combined.contains(needle),
                "missing {needle:?} in {combined:?}"
            );
        }
    }

    // Robust: streaming partial input that ends mid-codeblock doesn't lose
    // characters or panic. Mirrors what we get during streaming.
    #[test]
    fn streaming_partial_codeblock_no_loss_robust() {
        let prefix = "Here is the code:\n\n```rust\nfn alpha() {\n    let x = ";
        let lines = render(prefix);
        let combined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains("alpha"), "alpha lost: {combined:?}");
        assert!(combined.contains("let x"), "let lost: {combined:?}");
    }
}

#[cfg(test)]
mod hard_wrap_cell_width_tests {
    use super::hard_wrap_str;

    // Normal: ASCII text wraps at character boundary as before — no
    // change for the common case.
    #[test]
    fn ascii_wraps_at_width_normal() {
        let chunks = hard_wrap_str("hello world this is a test", 10);
        // "hello worl" / "d this is " / "a test"
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].chars().count() <= 10);
        assert!(chunks[1].chars().count() <= 10);
    }

    // Robust: CJK text — each char is 2 cells wide. A 5-char string
    // should fit in width=10 (5*2), wrap at width=8 (only 4 chars =
    // 8 cells fit).
    #[test]
    fn cjk_wraps_by_cell_width_robust() {
        // 5 CJK chars × 2 cells each = 10 cells total
        let s = "你好世界吗";
        // Width 10 → fits in one chunk
        assert_eq!(hard_wrap_str(s, 10).len(), 1);
        // Width 8 → 4 chars (8 cells) per chunk + 1 char (2 cells) on the next
        let chunks = hard_wrap_str(s, 8);
        assert_eq!(chunks.len(), 2);
    }

    // Robust: emoji (variable-width, often 2 cells). Mixed with ASCII.
    #[test]
    fn emoji_mixed_ascii_robust() {
        // Each fire emoji is 2 cells; "abc" is 3 cells.
        // "🔥🔥🔥abc" = 2+2+2+3 = 9 cells.
        let s = "🔥🔥🔥abc";
        // Width 9 → fits exactly
        assert_eq!(hard_wrap_str(s, 9).len(), 1);
        // Width 8 → can't fit all 9 cells, wraps. The 3rd 🔥 starts
        // at cell 4, takes through cell 5; "abc" spans cells 6-8 in
        // a width-8 window. So chunk 1 = "🔥🔥🔥a" (8 cells if we're
        // generous, or "🔥🔥🔥" + start of new chunk if strict).
        // Implementation is "emit current buf when next char would
        // overflow" — so 🔥🔥🔥 = 6 cells + 'a' brings us to 7, +b
        // to 8, +c WOULD overflow → emit. chunk 1 = "🔥🔥🔥ab", chunk
        // 2 = "c". Either way wrap happens.
        let chunks = hard_wrap_str(s, 8);
        assert!(chunks.len() >= 2, "expected wrap, got {chunks:?}");
    }

    // Robust: width=0 returns the input unchanged (no infinite loop).
    #[test]
    fn width_zero_returns_input_robust() {
        let chunks = hard_wrap_str("anything", 0);
        assert_eq!(chunks, vec!["anything".to_string()]);
    }

    // Robust: a single char wider than the window doesn't infinite-
    // loop or panic. Gets emitted on its own line.
    #[test]
    fn single_oversize_char_emits_alone_robust() {
        // 🔥 is 2 cells; window of 1 cell can't fit it. Should still
        // produce output (no infinite loop).
        let chunks = hard_wrap_str("🔥a", 1);
        assert!(!chunks.is_empty());
        // Total reconstructed should match input.
        let rejoined: String = chunks.join("");
        assert_eq!(rejoined, "🔥a");
    }
}

#[cfg(test)]
mod table_reflow_tests {
    use super::*;
    use crate::theme::Theme;

    fn t() -> Theme {
        Theme::dark()
    }

    fn render_table_with_width(src: &str, width: usize) -> Vec<Line<'static>> {
        to_lines(src, &t(), width)
    }

    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    // Normal: a table that fits naturally renders at natural widths
    // — no wrap, no surprises.
    #[test]
    fn table_fits_naturally_normal() {
        let src = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        // Pass a generous width so natural sizing wins.
        let lines = render_table_with_width(src, 200);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        // Should have a top border, header, separator, body, bottom border.
        assert!(
            texts.iter().any(|t| t.starts_with("┌")),
            "no top: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("├")),
            "no sep: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.starts_with("└")),
            "no bot: {texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("1") && t.contains("2")));
    }

    // Robust: a table wider than the target width gets columns
    // proportionally shrunk and long content wraps to multiple
    // visual rows. Total rendered width should fit in target.
    #[test]
    fn table_overflow_wraps_robust() {
        let src = "| Short | Long content here that should wrap |\n\
                   |-------|------------------------------------|\n\
                   | x     | this body cell is also fairly long   |\n";
        // Force a narrow target width.
        let lines = render_table_with_width(src, 30);
        let texts: Vec<String> = lines.iter().map(line_text).collect();

        // Every line of the table itself (the box-drawn ones)
        // should be ≤ target width in cells.
        use unicode_width::UnicodeWidthStr;
        for t in texts.iter().filter(|t| {
            t.starts_with("┌") || t.starts_with("├") || t.starts_with("└") || t.starts_with("│")
        }) {
            let w = UnicodeWidthStr::width(t.as_str());
            assert!(
                w <= 32,
                "table line exceeds target+slack: {w} cells in {t:?}"
            );
        }
    }

    // Robust: a table with CJK content widths cells correctly using
    // cell width, not byte length. The earlier `text.len()` would
    // make a 5-char CJK cell think it was 15 bytes wide, blowing
    // out the column.
    #[test]
    fn table_cjk_cell_width_robust() {
        let src = "| A | B |\n|---|---|\n| 你好 | 世界 |\n";
        let lines = render_table_with_width(src, 60);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.contains("你好")));
        assert!(texts.iter().any(|t| t.contains("世界")));
    }

    // Robust: empty cells don't panic, render as blank padding.
    #[test]
    fn table_empty_cells_robust() {
        let src = "| A | B |\n|---|---|\n|   |   |\n";
        let lines = render_table_with_width(src, 60);
        // Just verify no panic and we got at least the chrome rows.
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.starts_with("┌")));
    }

    // Robust: target_width=0 means "no wrap" — table renders at
    // natural widths regardless. Useful for tests + the to_lines
    // path before we knew the terminal width.
    #[test]
    fn table_width_zero_no_wrap_robust() {
        let src = "| Col | Long content here |\n|-----|-------------------|\n| 1 | 2 |\n";
        let lines = render_table_with_width(src, 0);
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        // Natural width includes the long header content.
        assert!(
            texts.iter().any(|t| t.contains("Long content here")),
            "long content should fit on one line at width=0: {texts:?}"
        );
    }
}

#[cfg(test)]
mod color_swatch_tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn detects_6_digit_hex_normal() {
        let style = Style::default();
        let result = colorize_inline_colors("color is #ff7ab2 here", style);
        assert!(result.is_some());
        let spans = result.unwrap();
        // Should be: "color is " + swatch + "#ff7ab2" + " here"
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content.as_ref(), "color is ");
        assert_eq!(spans[1].content.as_ref(), "\u{2588} ");
        assert_eq!(spans[1].style.fg, Some(Color::Rgb(255, 122, 178)));
        assert_eq!(spans[2].content.as_ref(), "#ff7ab2");
        assert_eq!(spans[3].content.as_ref(), " here");
    }

    #[test]
    fn detects_3_digit_hex_normal() {
        let style = Style::default();
        let result = colorize_inline_colors("try #fff please", style);
        assert!(result.is_some());
        let spans = result.unwrap();
        assert_eq!(spans[1].style.fg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(spans[2].content.as_ref(), "#fff");
    }

    #[test]
    fn detects_rgb_function_normal() {
        let style = Style::default();
        let result = colorize_inline_colors("use rgb(100, 200, 50) for green", style);
        assert!(result.is_some());
        let spans = result.unwrap();
        assert_eq!(spans[1].style.fg, Some(Color::Rgb(100, 200, 50)));
        assert_eq!(spans[2].content.as_ref(), "rgb(100, 200, 50)");
    }

    #[test]
    fn rejects_preprocessor_directives_robust() {
        let style = Style::default();
        // #define, #include, #ifdef should NOT trigger
        assert!(colorize_inline_colors("#define FOO", style).is_none());
        assert!(colorize_inline_colors("#include <stdio.h>", style).is_none());
        assert!(colorize_inline_colors("#ifdef DEBUG", style).is_none());
    }

    #[test]
    fn rejects_anchor_links_robust() {
        let style = Style::default();
        // Markdown anchor links like #section-name have non-hex chars
        assert!(colorize_inline_colors("#section-title", style).is_none());
        assert!(colorize_inline_colors("#my-heading", style).is_none());
    }

    #[test]
    fn rejects_git_hashes_robust() {
        let style = Style::default();
        // Git short hashes are 7+ hex chars — our regex requires exactly 6
        // and the 7th char being hex triggers the negative lookahead
        assert!(colorize_inline_colors("#0c45357a", style).is_none());
        assert!(colorize_inline_colors("#b9cf926d", style).is_none());
    }

    #[test]
    fn rejects_rgb_out_of_range_robust() {
        let style = Style::default();
        // rgb values > 255 should not match as valid colors
        assert!(colorize_inline_colors("rgb(300, 200, 50)", style).is_none());
        assert!(colorize_inline_colors("rgb(100, 256, 50)", style).is_none());
    }

    #[test]
    fn multiple_colors_in_one_line_normal() {
        let style = Style::default();
        let result = colorize_inline_colors("#ff0000 and #00ff00", style);
        assert!(result.is_some());
        let spans = result.unwrap();
        // swatch + "#ff0000" + " and " + swatch + "#00ff00"
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].style.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(spans[1].content.as_ref(), "#ff0000");
        assert_eq!(spans[2].content.as_ref(), " and ");
        assert_eq!(spans[3].style.fg, Some(Color::Rgb(0, 255, 0)));
        assert_eq!(spans[4].content.as_ref(), "#00ff00");
    }

    #[test]
    fn no_colors_returns_none_normal() {
        let style = Style::default();
        assert!(colorize_inline_colors("just plain text", style).is_none());
        assert!(colorize_inline_colors("code `foo` here", style).is_none());
    }

    #[test]
    fn hex_mid_word_rejected_robust() {
        let style = Style::default();
        // A hex code glued to an alphanumeric char shouldn't match
        assert!(colorize_inline_colors("foo#aabbcc", style).is_none());
        assert!(colorize_inline_colors("x#fff", style).is_none());
    }

    #[test]
    fn hex_after_punctuation_accepted_normal() {
        let style = Style::default();
        // Should work after (, [, comma, colon, quote, etc.
        let result = colorize_inline_colors("color:#ff0000;", style);
        assert!(result.is_some());
        let result = colorize_inline_colors("(#aabb00)", style);
        assert!(result.is_some());
        let result = colorize_inline_colors("[#112233]", style);
        assert!(result.is_some());
    }

    #[test]
    fn preserves_base_style_normal() {
        let style = Style::default()
            .fg(Color::Rgb(100, 100, 100))
            .add_modifier(Modifier::BOLD);
        let result = colorize_inline_colors("see #ff0000 here", style).unwrap();
        // Non-swatch spans should carry the base style
        assert_eq!(result[0].style, style);
        assert_eq!(result[2].style, style);
        assert_eq!(result[3].style, style);
    }

    #[test]
    fn shadotheme_hex_codes_detected_normal() {
        // Real hex codes from the shadotheme implementation that should
        // produce color swatches in prose/inline-code contexts.
        let style = Style::default();
        let codes = &[
            ("#111119", Color::Rgb(0x11, 0x11, 0x19)),
            ("#1b1b29", Color::Rgb(0x1b, 0x1b, 0x29)),
            ("#505079", Color::Rgb(0x50, 0x50, 0x79)),
            ("#dfb7e8", Color::Rgb(0xdf, 0xb7, 0xe8)),
            ("#bd93f9", Color::Rgb(0xbd, 0x93, 0xf9)),
            ("#37d4a7", Color::Rgb(0x37, 0xd4, 0xa7)),
            ("#B52A5B", Color::Rgb(0xB5, 0x2A, 0x5B)),
            ("#ff7ab2", Color::Rgb(0xff, 0x7a, 0xb2)),
            ("#8677d9", Color::Rgb(0x86, 0x77, 0xd9)),
        ];
        for (hex, expected_color) in codes {
            let result = colorize_inline_colors(hex, style)
                .unwrap_or_else(|| panic!("should detect color in {hex}"));
            // swatch span should have the correct fg color
            let swatch = &result[0];
            assert_eq!(
                swatch.style.fg,
                Some(*expected_color),
                "wrong color for {hex}"
            );
            assert_eq!(swatch.content.as_ref(), "\u{2588} ");
            // The code text itself follows
            assert_eq!(result[1].content.as_ref(), *hex);
        }
    }

    #[test]
    fn inline_code_backtick_gets_swatch_via_to_lines_normal() {
        // When markdown contains `#ff7ab2` (backtick-wrapped), the rendered
        // output should include a swatch span with the color's fg.
        let theme = crate::theme::Theme::dark();
        let md = "The color is `#ff7ab2` in the theme.";
        let lines = super::to_lines(md, &theme, 120);
        // Flatten all spans
        let all_spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        // At least one span should be the swatch character with fg=#ff7ab2
        let has_swatch = all_spans.iter().any(|s| {
            s.content.contains('\u{2588}') && s.style.fg == Some(Color::Rgb(0xff, 0x7a, 0xb2))
        });
        assert!(
            has_swatch,
            "expected a swatch span with fg=#ff7ab2 in rendered output, got: {all_spans:?}"
        );
    }

    #[test]
    fn inline_code_rgb_gets_swatch_via_to_lines_normal() {
        let theme = crate::theme::Theme::dark();
        let md = "Use `rgb(55, 212, 167)` for green.";
        let lines = super::to_lines(md, &theme, 120);
        let all_spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        let has_swatch = all_spans.iter().any(|s| {
            s.content.contains('\u{2588}') && s.style.fg == Some(Color::Rgb(55, 212, 167))
        });
        assert!(
            has_swatch,
            "expected a swatch span with fg=rgb(55,212,167) in rendered output, got: {all_spans:?}"
        );
    }
}
