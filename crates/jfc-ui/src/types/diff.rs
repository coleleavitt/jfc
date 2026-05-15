#[derive(Clone, Debug)]
pub struct DiffView {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

#[derive(Clone, Copy, Debug)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

pub fn parse_unified_diff(file_path: &str, patch: &str) -> DiffView {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for raw_line in patch.lines() {
        if raw_line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }

            let (old_start, new_start, header) = parse_hunk_header(raw_line);
            old_line = old_start;
            new_line = new_start;
            current = Some(DiffHunk {
                old_start,
                new_start,
                header,
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        let (kind, content) = match raw_line.chars().next() {
            Some('+') => (DiffLineKind::Added, &raw_line[1..]),
            Some('-') => (DiffLineKind::Removed, &raw_line[1..]),
            Some(' ') => (DiffLineKind::Context, &raw_line[1..]),
            _ => (DiffLineKind::Context, raw_line),
        };

        match kind {
            DiffLineKind::Added => {
                additions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: None,
                    new_line: Some(new_line),
                    content: content.into(),
                });
                new_line += 1;
            }
            DiffLineKind::Removed => {
                deletions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: None,
                    content: content.into(),
                });
                old_line += 1;
            }
            DiffLineKind::Context => {
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                    content: content.into(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let Some(hunk) = current {
        hunks.push(hunk);
    }

    DiffView {
        file_path: file_path.into(),
        hunks,
        additions,
        deletions,
    }
}

pub fn parse_hunk_header(header: &str) -> (usize, usize, String) {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old = parts.next().unwrap_or("-1");
    let new = parts.next().unwrap_or("+1");
    let tail = parts.collect::<Vec<_>>().join(" ");
    (parse_hunk_start(old), parse_hunk_start(new), tail)
}

pub fn parse_hunk_start(token: &str) -> usize {
    token
        .trim_start_matches(['-', '+'])
        .split(',')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
}

pub fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<_> = text.lines().collect();
    let mut result = lines
        .iter()
        .take(max_lines)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > max_lines {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("… {} more lines", lines.len() - max_lines));
    }
    result
}
