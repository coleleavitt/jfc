#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

pub enum MessagePart {
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
}

pub struct ToolCall {
    pub id: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub input: ToolInput,
    pub output: ToolOutput,
    pub is_collapsed: bool,
}

pub enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
    Search,
    ApplyPatch,
    Generic(String),
}

#[derive(Clone, Copy)]
pub enum ToolStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

pub enum ToolInput {
    Edit {
        file_path: String,
        old_string: String,
        new_string: String,
    },
    Write {
        file_path: String,
        content: String,
    },
    Read {
        file_path: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    Bash {
        command: String,
        workdir: Option<String>,
    },
    Search {
        query: String,
        path: Option<String>,
    },
    ApplyPatch {
        patch: String,
    },
    Generic {
        summary: String,
    },
}

pub enum ToolOutput {
    Text(String),
    Diff(DiffView),
    FileContent {
        path: String,
        content: String,
        language: String,
    },
    Command {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    FileList(Vec<String>),
    Empty,
}

pub struct DiffView {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

#[derive(Clone, Copy)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

pub struct ChatMessage {
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub agent_name: Option<String>,
    pub model_name: Option<String>,
    pub cost_tier: Option<String>,
    pub elapsed: Option<String>,
}

impl ChatMessage {
    pub fn user(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
        }
    }

    pub fn assistant(content: String) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![MessagePart::Text(content)],
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
        }
    }

    pub fn assistant_parts(parts: Vec<MessagePart>) -> Self {
        Self {
            role: Role::Assistant,
            parts,
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
        }
    }
}

impl ToolKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Search => "Search",
            Self::ApplyPatch => "Patch",
            Self::Generic(name) => name.as_str(),
        }
    }
}

impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Complete => "done",
            Self::Failed => "failed",
        }
    }
}

impl ToolInput {
    pub fn summary(&self) -> String {
        match self {
            Self::Edit {
                file_path,
                old_string,
                new_string,
            } => format!(
                "{} ({} → {} chars)",
                file_path,
                old_string.len(),
                new_string.len()
            ),
            Self::Write { file_path, content } => {
                format!("{} ({} bytes)", file_path, content.len())
            }
            Self::Read {
                file_path,
                offset,
                limit,
            } => match (offset, limit) {
                (Some(offset), Some(limit)) => format!("{file_path}:{offset} (+{limit})"),
                _ => file_path.clone(),
            },
            Self::Bash { command, workdir } => match workdir {
                Some(workdir) => format!("{command} in {workdir}"),
                None => command.clone(),
            },
            Self::Search { query, path } => match path {
                Some(path) => format!("{query} in {path}"),
                None => query.clone(),
            },
            Self::ApplyPatch { patch } => format!("apply patch ({} bytes)", patch.len()),
            Self::Generic { summary } => summary.clone(),
        }
    }
}

pub fn sample_tool_harness_message() -> ChatMessage {
    let diff = parse_unified_diff(
        "references/wgpui/crates/gpui_linux/src/linux/wayland/window.rs",
        r#"@@ -1502,2 +1502,2 @@
-let w = state.bounds.size.width.0 as i32;
-let h = state.bounds.size.height.0 as i32;
+let w = f32::from(state.bounds.size.width) as i32;
+let h = f32::from(state.bounds.size.height) as i32;
"#,
    );

    ChatMessage::assistant_parts(vec![
        MessagePart::Reasoning("Pixels has private fields. Use the same f32::from pattern.".into()),
        MessagePart::Tool(ToolCall {
            id: "edit-1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Complete,
            input: ToolInput::Edit {
                file_path: "references/wgpui/crates/gpui_linux/src/linux/wayland/window.rs".into(),
                old_string: "let w = state.bounds.size.width.0 as i32;".into(),
                new_string: "let w = f32::from(state.bounds.size.width) as i32;".into(),
            },
            output: ToolOutput::Diff(diff),
            is_collapsed: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "bash-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Complete,
            input: ToolInput::Bash {
                command: "cargo check -p gpui_linux".into(),
                workdir: Some("references/wgpui".into()),
            },
            output: ToolOutput::Command {
                stdout: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.38s"
                    .into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            is_collapsed: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "read-1".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Complete,
            input: ToolInput::Read {
                file_path: "crates/jfc-ui/src/main.rs".into(),
                offset: Some(1),
                limit: Some(80),
            },
            output: ToolOutput::FileContent {
                path: "crates/jfc-ui/src/main.rs".into(),
                language: "rust".into(),
                content: "mod text_input;\nmod theme;\n\nuse gpui::*;\nuse theme::Theme;".into(),
            },
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "write-1".into(),
            kind: ToolKind::Write,
            status: ToolStatus::Pending,
            input: ToolInput::Write {
                file_path: "crates/jfc-ui/src/tool_harness.rs".into(),
                content: "pub enum MessagePart { Text(String), Tool(ToolCall) }".into(),
            },
            output: ToolOutput::Text("Waiting for approval".into()),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "search-1".into(),
            kind: ToolKind::Search,
            status: ToolStatus::Running,
            input: ToolInput::Search {
                query: "ToolRegistry|DiffChanges|tool_result".into(),
                path: Some("research/opencode".into()),
            },
            output: ToolOutput::FileList(vec![
                "packages/ui/src/components/message-part.tsx".into(),
                "packages/ui/src/components/diff-changes.tsx".into(),
                "packages/opencode/src/tool/edit.ts".into(),
            ]),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "patch-1".into(),
            kind: ToolKind::ApplyPatch,
            status: ToolStatus::Complete,
            input: ToolInput::ApplyPatch {
                patch: "*** Begin Patch\n*** Update File: crates/jfc-ui/src/main.rs".into(),
            },
            output: ToolOutput::Diff(parse_unified_diff(
                "crates/jfc-ui/src/main.rs",
                r#"@@ -10,1 +10,1 @@
-struct ChatMessage;
+enum MessagePart;
"#,
            )),
            is_collapsed: true,
        }),
        MessagePart::Tool(ToolCall {
            id: "generic-1".into(),
            kind: ToolKind::Generic("Delegate".into()),
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "OpenClaude remote lookup".into(),
            },
            output: ToolOutput::Empty,
            is_collapsed: true,
        }),
    ])
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
