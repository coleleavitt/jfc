use itertools::{Itertools, Position};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options as ParseOptions, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
};

use ansi_to_tui::IntoText;
use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::SyntaxSet,
    util::{LinesWithEndings, as_24_bit_terminal_escaped},
};

use crate::theme::Theme;

static SYNTAX_SET: std::sync::LazyLock<SyntaxSet> =
    std::sync::LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: std::sync::LazyLock<ThemeSet> = std::sync::LazyLock::new(ThemeSet::load_defaults);

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

pub fn to_lines(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    // v126 disables strikethrough because the GFM `~~text~~` syntax collides
    // with `~~~` fenced code blocks: a stray paragraph containing `~~~` would
    // open a code block on the next round-trip. We follow the same call.
    let mut opts = ParseOptions::empty();
    opts.insert(ParseOptions::ENABLE_TASKLISTS);
    opts.insert(ParseOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(text, opts);

    let mut w = MdWriter::new(parser, theme);
    w.code_wrap_width = width;
    w.run();
    w.text.lines
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

    fn render(self, theme: &Theme) -> Vec<Line<'static>> {
        let ncols = self.alignments.len().max(
            self.head_row
                .len()
                .max(self.body_rows.iter().map(|r| r.len()).max().unwrap_or(0)),
        );
        if ncols == 0 {
            return Vec::new();
        }

        let mut widths = vec![0usize; ncols];
        for (i, cell) in self.head_row.iter().enumerate() {
            if i < ncols {
                widths[i] = widths[i].max(Self::cell_text(cell).len());
            }
        }
        for row in &self.body_rows {
            for (i, cell) in row.iter().enumerate() {
                if i < ncols {
                    widths[i] = widths[i].max(Self::cell_text(cell).len());
                }
            }
        }
        for w in &mut widths {
            *w = (*w).max(3);
        }

        let border_style = Style::default().fg(theme.border);
        let head_style = Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD);

        let mut lines = Vec::new();

        if !self.head_row.is_empty() {
            let mut spans = vec![Span::styled("│ ", border_style)];
            for (i, cell) in self.head_row.iter().enumerate() {
                let text = Self::cell_text(cell);
                let w = widths.get(i).copied().unwrap_or(3);
                let padded = format!("{:<width$}", text, width = w);
                spans.push(Span::styled(padded, head_style));
                spans.push(Span::styled(" │ ", border_style));
            }
            lines.push(Line::from(spans));

            let mut sep_parts = vec![Span::styled("├─", border_style)];
            for (i, &w) in widths.iter().enumerate() {
                sep_parts.push(Span::styled("─".repeat(w), border_style));
                if i + 1 < ncols {
                    sep_parts.push(Span::styled("─┼─", border_style));
                }
            }
            sep_parts.push(Span::styled("─┤", border_style));
            lines.push(Line::from(sep_parts));
        }

        let body_style = Style::default().fg(theme.text_primary);
        for row in &self.body_rows {
            let mut spans = vec![Span::styled("│ ", border_style)];
            for (i, cell) in row.iter().enumerate() {
                let text = Self::cell_text(cell);
                let w = widths.get(i).copied().unwrap_or(3);
                let padded = format!("{:<width$}", text, width = w);
                spans.push(Span::styled(padded, body_style));
                spans.push(Span::styled(" │ ", border_style));
            }
            lines.push(Line::from(spans));
        }

        lines
    }
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
                self.push_line(Line::styled(
                    "---",
                    Style::default().fg(self.theme.text_muted),
                ));
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
                self.line_prefixes.push(Span::styled(
                    "> ",
                    Style::default().fg(self.theme.text_muted),
                ));
                self.line_styles
                    .push(Style::default().fg(self.theme.text_secondary).italic());
            }
            Tag::CodeBlock(kind) => {
                if !self.text.lines.is_empty() {
                    self.push_line(Line::default());
                }
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) => l.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                self.set_code_highlighter(lang);

                // Frame the block with the same `┌─ … │ … └─` chrome the tool
                // renderer uses, so code blocks read as distinct visual units
                // instead of trailing inline against prose. v126 SOP step 5
                // (`Fs` component) does the same — header row, left gutter,
                // matching close.
                let header_label = if lang.is_empty() {
                    "▸ code".to_string()
                } else {
                    format!("▸ {lang}")
                };
                let header_style = Style::default().fg(self.theme.accent);
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
                    self.push_span(Span::styled(
                        prefix,
                        Style::default().fg(self.theme.text_muted),
                    ));
                }
                self.needs_newline = false;
            }
            Tag::Emphasis => self.push_inline_style(Style::new().italic()),
            Tag::Strong => self.push_inline_style(Style::new().bold()),
            Tag::Strikethrough => self.push_inline_style(Style::new().crossed_out()),
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url);
                self.push_inline_style(Style::default().fg(self.theme.accent));
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
                    Style::default().fg(self.theme.border),
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
                        Style::default().fg(self.theme.text_muted),
                    ));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let rendered = table.render(self.theme);
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
                .push(Span::styled(text.to_string(), style));
            return;
        }

        if let Some(highlighter) = &mut self.code_highlighter {
            // Hard-wrap each highlighted line to viewport width *minus the
            // gutter* (the `│ ` prefix we prepend below) so ratatui's
            // word-wrap doesn't mangle ASCII trees. Then prefix each line
            // with the gutter span, matching the tool-call visual frame.
            let inner_width = self.code_wrap_width.saturating_sub(2).max(20);
            let highlighted: Text = LinesWithEndings::from(text.as_ref())
                .filter_map(|line| highlighter.highlight_line(line, &SYNTAX_SET).ok())
                .filter_map(|part| as_24_bit_terminal_escaped(&part, false).into_text().ok())
                .flatten()
                .collect();

            let gutter_style = Style::default().fg(self.theme.border);
            for line in highlighted.lines {
                for chunk in hard_wrap_line(line, inner_width) {
                    let mut spans = vec![Span::styled("│ ".to_string(), gutter_style)];
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
            let gutter_style = Style::default().fg(self.theme.border);
            for line in text.lines() {
                let style = Style::default().fg(self.theme.text_secondary);
                for chunk in hard_wrap_str(line, inner_width) {
                    self.push_line(Line::from(vec![
                        Span::styled("│ ".to_string(), gutter_style),
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
                .unwrap_or(Style::default().fg(self.theme.text_primary));
            self.push_span(Span::styled(line.to_string(), style));
        }
        self.needs_newline = false;
    }

    fn on_code(&mut self, code: CowStr<'a>) {
        if let Some(ref mut table) = self.table {
            table
                .current_cell
                .push(Span::styled(code.to_string(), self.theme.inline_code()));
            return;
        }
        self.push_span(Span::styled(code.to_string(), self.theme.inline_code()));
    }

    fn heading_style(&self, level: usize) -> Style {
        match level {
            1 => Style::default()
                .fg(self.theme.text_primary)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::default()
                .fg(self.theme.text_primary)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .fg(self.theme.text_secondary)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn set_code_highlighter(&mut self, lang: &str) {
        if let Some(syntax) = SYNTAX_SET.find_syntax_by_token(lang) {
            let ts = &THEME_SET.themes["base16-ocean.dark"];
            self.code_highlighter = Some(HighlightLines::new(syntax, ts));
        }
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
fn hard_wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
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

/// Hard-wrap a plain string to `width` columns, returning owned chunks. Used
/// for non-highlighted code blocks where we don't have styled spans yet.
pub(crate) fn hard_wrap_str(s: &str, width: usize) -> Vec<String> {
    if width == 0 || s.chars().count() <= width {
        return vec![s.to_owned()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut col = 0usize;
    for ch in s.chars() {
        buf.push(ch);
        col += 1;
        if col >= width {
            out.push(std::mem::take(&mut buf));
            col = 0;
        }
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
            texts.iter().any(|t| t.starts_with("┌─ ▸ rust")),
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
            texts.iter().any(|t| t.starts_with("┌─ ▸ code")),
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
