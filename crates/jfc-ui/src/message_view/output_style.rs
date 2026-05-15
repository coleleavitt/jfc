use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

use crate::theme::Theme;

/// Colorize a line of `git diff` output by its leading character.
pub(super) fn colorize_git_diff_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    if line.is_empty() {
        return None;
    }
    if line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
    {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]);
    }
    if line.starts_with("--- ") || line.starts_with("+++ ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]);
    }
    if line.starts_with("@@") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.accent),
        )]);
    }
    if line.starts_with("commit ") && line.len() >= 47 {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.warning),
        )]);
    }
    if let Some(rest) = line.strip_prefix('+') {
        return Some(vec![
            Span::styled("+".to_owned(), Style::default().fg(t.success)),
            Span::styled(rest.to_owned(), Style::default().fg(t.success)),
        ]);
    }
    if let Some(rest) = line.strip_prefix('-') {
        return Some(vec![
            Span::styled("-".to_owned(), Style::default().fg(t.error)),
            Span::styled(rest.to_owned(), Style::default().fg(t.error)),
        ]);
    }
    let _ = fallback;
    None
}

/// Colorize a `git status --porcelain` row.
pub(super) fn colorize_git_status_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    let bytes = line.as_bytes();
    if bytes.len() < 4 || bytes[2] != b' ' {
        return None;
    }
    let staged = bytes[0] as char;
    let unstaged = bytes[1] as char;
    if !is_status_char(staged) || !is_status_char(unstaged) {
        return None;
    }
    let staged_style = if staged == ' ' {
        Style::default().fg(fallback)
    } else if matches!(staged, 'M' | 'A' | 'R' | 'C' | 'D' | 'T') {
        Style::default().fg(t.success).add_modifier(Modifier::BOLD)
    } else if staged == '?' {
        Style::default().fg(t.error)
    } else {
        Style::default().fg(fallback)
    };
    let unstaged_style = if unstaged == ' ' {
        Style::default().fg(fallback)
    } else if matches!(unstaged, 'M' | 'D' | 'T' | 'U' | '?') {
        Style::default().fg(t.error).add_modifier(Modifier::BOLD)
    } else if matches!(unstaged, 'A' | 'R' | 'C') {
        Style::default().fg(t.success).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fallback)
    };
    let path = &line[3..];
    Some(vec![
        Span::styled(staged.to_string(), staged_style),
        Span::styled(unstaged.to_string(), unstaged_style),
        Span::styled(" ".to_owned(), Style::default().fg(fallback)),
        Span::styled(path.to_owned(), Style::default().fg(fallback)),
    ])
}

fn is_status_char(c: char) -> bool {
    matches!(c, ' ' | 'M' | 'A' | 'D' | 'R' | 'C' | 'U' | 'T' | '?' | '!')
}

/// Colorize `git log --oneline` rows: `<hash> [refs] subject`.
pub(super) fn colorize_git_log_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    let mut chars = line.chars();
    let mut hash_len = 0;
    let mut saw_space = false;
    for c in chars.by_ref() {
        if c.is_ascii_hexdigit() {
            hash_len += 1;
            if hash_len > 40 {
                return None;
            }
        } else if c == ' ' {
            saw_space = true;
            break;
        } else {
            return None;
        }
    }
    if hash_len < 7 || !saw_space {
        return None;
    }
    let hash = &line[..hash_len];
    let rest = &line[hash_len + 1..];
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            hash.to_owned(),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".to_owned(), Style::default().fg(fallback)),
    ];
    if let Some(rest2) = rest.strip_prefix('(') {
        if let Some(end) = rest2.find(')') {
            let refs = &rest2[..end];
            spans.push(Span::styled("(".to_owned(), Style::default().fg(t.warning)));
            for (i, part) in refs.split(", ").enumerate() {
                if i > 0 {
                    spans.push(Span::styled(
                        ", ".to_owned(),
                        Style::default().fg(t.warning),
                    ));
                }
                let style = if part.starts_with("HEAD") {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else if part.starts_with("origin/") || part.starts_with("upstream/") {
                    Style::default().fg(t.error)
                } else if part.starts_with("tag:") {
                    Style::default().fg(t.warning)
                } else {
                    Style::default().fg(t.success)
                };
                spans.push(Span::styled(part.to_owned(), style));
            }
            spans.push(Span::styled(")".to_owned(), Style::default().fg(t.warning)));
            spans.push(Span::styled(
                rest2[end + 1..].to_owned(),
                Style::default().fg(fallback),
            ));
            return Some(spans);
        }
    }
    spans.push(Span::styled(rest.to_owned(), Style::default().fg(fallback)));
    Some(spans)
}

/// Colorize the rows that `git commit` emits after a successful commit.
pub(super) fn colorize_git_commit_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    if line.starts_with('[') {
        let close = line.find(']')?;
        let inside = &line[1..close];
        let (branch, hash) = inside.split_once(' ')?;

        let subject = &line[close + 1..];
        if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        return Some(vec![
            Span::styled("[".to_owned(), Style::default().fg(fallback)),
            Span::styled(
                branch.to_owned(),
                Style::default().fg(t.success).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".to_owned(), Style::default().fg(fallback)),
            Span::styled(
                hash.to_owned(),
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled("]".to_owned(), Style::default().fg(fallback)),
            Span::styled(subject.to_owned(), Style::default().fg(fallback)),
        ]);
    }
    if line.trim_start().starts_with(|c: char| c.is_ascii_digit()) && line.contains("files changed")
        || line.contains("file changed")
    {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '(' && matches!(chars.peek(), Some('+') | Some('-')) {
                if !buf.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut buf),
                        Style::default().fg(fallback),
                    ));
                }
                let sign = chars.next().unwrap();
                let style = if sign == '+' {
                    Style::default().fg(t.success)
                } else {
                    Style::default().fg(t.error)
                };
                spans.push(Span::styled(format!("({sign})"), style));
                if matches!(chars.peek(), Some(')')) {
                    chars.next();
                }
                continue;
            }
            buf.push(c);
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, Style::default().fg(fallback)));
        }
        return Some(spans);
    }
    if line.starts_with(" create mode ") || line.starts_with("create mode ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.success),
        )]);
    }
    if line.starts_with(" delete mode ") || line.starts_with("delete mode ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.error),
        )]);
    }
    if line.starts_with(" rename ") || line.starts_with("rename ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.accent),
        )]);
    }
    if line.starts_with(" mode change ") || line.starts_with("mode change ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.warning),
        )]);
    }
    if line.starts_with(" copy ") || line.starts_with("copy ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default().fg(t.accent),
        )]);
    }
    None
}

/// Colorize `git push` / `git fetch` output.
pub(super) fn colorize_git_push_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    if line.starts_with("To ") || line.starts_with("From ") {
        return Some(vec![Span::styled(
            line.to_owned(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]);
    }
    if line.contains(" -> ") && !line.starts_with("rename ") {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*')
            || trimmed.starts_with('+')
            || trimmed.starts_with('-')
            || trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_hexdigit())
                .unwrap_or(false)
        {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut chars = line.char_indices().peekable();
            let mut last_end = 0usize;
            while let Some((i, c)) = chars.next() {
                if c == '[' {
                    if i > last_end {
                        spans.push(Span::styled(
                            line[last_end..i].to_owned(),
                            Style::default().fg(fallback),
                        ));
                    }
                    let tag_start = i;
                    let mut tag_end = i;
                    for (j, c2) in chars.by_ref() {
                        if c2 == ']' {
                            tag_end = j + 1;
                            break;
                        }
                    }
                    let tag = &line[tag_start..tag_end];
                    let style = if tag.contains("new") {
                        Style::default().fg(t.success).add_modifier(Modifier::BOLD)
                    } else if tag.contains("deleted") || tag.contains("rejected") {
                        Style::default().fg(t.error).add_modifier(Modifier::BOLD)
                    } else if tag.contains("forced") {
                        Style::default().fg(t.warning).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.accent)
                    };
                    spans.push(Span::styled(tag.to_owned(), style));
                    last_end = tag_end;
                }
            }
            let tail = &line[last_end..];
            if let Some(arrow) = tail.find(" -> ") {
                let before = &tail[..arrow];
                let after = &tail[arrow + 4..];
                spans.push(Span::styled(
                    before.to_owned(),
                    Style::default().fg(t.success),
                ));
                spans.push(Span::styled(
                    " -> ".to_owned(),
                    Style::default().fg(fallback),
                ));
                spans.push(Span::styled(
                    after.to_owned(),
                    Style::default().fg(t.success),
                ));
            } else {
                spans.push(Span::styled(tail.to_owned(), Style::default().fg(fallback)));
            }
            return Some(spans);
        }
    }
    None
}

/// Colorize lines that begin with a diagnostic prefix git, rustc, cargo, npm,
/// and most Unix CLIs use.
pub(super) fn colorize_diagnostic_prefix(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    let trimmed = line.trim_start();
    let leading_ws = &line[..line.len() - trimmed.len()];

    if trimmed.starts_with("error[") {
        let close = trimmed.find(']')?;
        let colon = trimmed[close..].find(':')?;
        let head_end = (close + colon + 2).min(trimmed.len());
        let head = &trimmed[..head_end];
        let rest = &trimmed[head_end..];
        return Some(vec![
            Span::styled(leading_ws.to_owned(), Style::default().fg(fallback)),
            Span::styled(
                head.to_owned(),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(rest.to_owned(), Style::default().fg(fallback)),
        ]);
    }

    let (label, label_style, rest) = if let Some(r) = trimmed.strip_prefix("error: ") {
        (
            "error: ",
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            r,
        )
    } else if let Some(r) = trimmed.strip_prefix("fatal: ") {
        (
            "fatal: ",
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            r,
        )
    } else if let Some(r) = trimmed.strip_prefix("warning: ") {
        (
            "warning: ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            r,
        )
    } else if let Some(r) = trimmed.strip_prefix("hint: ") {
        ("hint: ", Style::default().fg(t.warning), r)
    } else if let Some(r) = trimmed.strip_prefix("note: ") {
        ("note: ", Style::default().fg(t.accent), r)
    } else if let Some(r) = trimmed.strip_prefix("help: ") {
        ("help: ", Style::default().fg(t.success), r)
    } else {
        let r = trimmed.strip_prefix("usage: ")?;
        ("usage: ", Style::default().fg(t.warning), r)
    };
    Some(vec![
        Span::styled(leading_ws.to_owned(), Style::default().fg(fallback)),
        Span::styled(label.to_owned(), label_style),
        Span::styled(rest.to_owned(), Style::default().fg(fallback)),
    ])
}

pub(super) fn path_color(path: &str, t: Theme) -> Color {
    let ext = std::path::Path::new(path.trim())
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" | "go" | "py" | "js" | "ts" | "tsx" | "jsx" | "rb" | "java" | "c" | "cpp" | "h"
        | "hpp" | "swift" | "kt" | "lua" | "zig" | "ml" | "hs" | "ex" | "exs" => t.accent,
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "env" | "lock" => {
            t.text_secondary
        }
        "md" | "mdx" | "rst" | "txt" | "adoc" => t.text_primary,
        "html" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" => t.success,
        "sh" | "bash" | "zsh" | "fish" => t.warning,
        _ => t.text_muted,
    }
}
