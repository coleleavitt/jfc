mod text_input;
mod theme;

use gpui::*;
use gpui_platform::application;
use text_input::{SubmitEvent, TextInput};
use theme::Theme;

actions!(
    jfc,
    [
        ToggleCommandPalette,
        DismissCommandPalette,
        ToggleSidebar,
        SubmitPrompt,
        Quit
    ]
);

#[derive(Clone, Copy, PartialEq)]
enum Role {
    User,
    Assistant,
}

enum MessagePart {
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
}

struct ToolCall {
    id: String,
    kind: ToolKind,
    status: ToolStatus,
    input: ToolInput,
    output: ToolOutput,
    is_collapsed: bool,
}

enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
    Search,
    ApplyPatch,
    Generic(String),
}

#[derive(Clone, Copy)]
enum ToolStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

enum ToolInput {
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

enum ToolOutput {
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

struct DiffView {
    file_path: String,
    hunks: Vec<DiffHunk>,
    additions: usize,
    deletions: usize,
}

struct DiffHunk {
    old_start: usize,
    new_start: usize,
    header: String,
    lines: Vec<DiffLine>,
}

struct DiffLine {
    kind: DiffLineKind,
    old_line: Option<usize>,
    new_line: Option<usize>,
    content: String,
}

#[derive(Clone, Copy)]
enum DiffLineKind {
    Context,
    Added,
    Removed,
}

struct ChatMessage {
    role: Role,
    parts: Vec<MessagePart>,
    agent_name: Option<String>,
    model_name: Option<String>,
    cost_tier: Option<String>,
    elapsed: Option<String>,
}

impl ChatMessage {
    fn user(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
        }
    }

    fn assistant(content: String) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![MessagePart::Text(content)],
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
        }
    }

    fn assistant_parts(parts: Vec<MessagePart>) -> Self {
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
    fn label(&self) -> &str {
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
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Complete => "done",
            Self::Failed => "failed",
        }
    }
}

impl ToolInput {
    fn summary(&self) -> String {
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

fn sample_tool_harness_message() -> ChatMessage {
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

fn parse_unified_diff(file_path: &str, patch: &str) -> DiffView {
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

fn parse_hunk_header(header: &str) -> (usize, usize, String) {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old = parts.next().unwrap_or("-1");
    let new = parts.next().unwrap_or("+1");
    let tail = parts.collect::<Vec<_>>().join(" ");
    (parse_hunk_start(old), parse_hunk_start(new), tail)
}

fn parse_hunk_start(token: &str) -> usize {
    token
        .trim_start_matches(['-', '+'])
        .split(',')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
}

fn truncate_lines(text: &str, max_lines: usize) -> String {
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

struct RootView {
    theme: Theme,
    messages: Vec<ChatMessage>,
    text_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    is_command_palette_open: bool,
    is_sidebar_visible: bool,
    command_palette_query: String,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme = Theme::dark();
        let text_input = cx.new(|cx| {
            TextInput::new(
                cx,
                "Type a message... (Enter to send)",
                theme.text_primary,
                theme.text_muted,
                theme.accent,
            )
        });

        let subscription = cx.subscribe(&text_input, Self::on_submit);

        Self {
            theme,
            messages: Vec::new(),
            text_input,
            scroll_handle: ScrollHandle::new(),
            is_command_palette_open: false,
            is_sidebar_visible: true,
            command_palette_query: String::new(),
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![subscription],
        }
    }

    fn on_submit(
        &mut self,
        _input: Entity<TextInput>,
        event: &SubmitEvent,
        _cx: &mut Context<Self>,
    ) {
        let user_content = event.content.clone();
        self.messages.push(ChatMessage::user(user_content.clone()));

        let truncated = if user_content.len() > 50 {
            format!("{}...", &user_content[..50])
        } else {
            user_content
        };
        let response = format!(
            "I received your message: \"{}\". This is a **placeholder** response with `inline code` and *italic text*.\n\n### Example\n```rust\nfn main() {{\n    let greeting = \"Hello, world!\";\n    println!(\"{{}}{{}}\", greeting);\n}}\n```\nLet me know if you need anything else!",
            truncated
        );
        self.messages.push(sample_tool_harness_message());
        self.messages.push(ChatMessage::assistant(response));

        self.scroll_handle.scroll_to_bottom();
    }

    fn toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_command_palette_open = !self.is_command_palette_open;
        self.command_palette_query.clear();
        cx.notify();
    }

    fn dismiss_command_palette(
        &mut self,
        _: &DismissCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_command_palette_open = false;
        self.command_palette_query.clear();
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_sidebar_visible = !self.is_sidebar_visible;
        cx.notify();
    }

    fn toggle_tool_collapsed(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(message) = self.messages.get_mut(index) {
            for part in &mut message.parts {
                if let MessagePart::Tool(tool) = part {
                    tool.is_collapsed = !tool.is_collapsed;
                    cx.notify();
                    break;
                }
            }
        }
    }

    fn submit_prompt(&mut self, _: &SubmitPrompt, _window: &mut Window, cx: &mut Context<Self>) {
        self.text_input.update(cx, |input, cx| {
            input.submit_current(cx);
        });
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let input = self.text_input.read(cx);
        tracing::debug!(
            target: "jfc::ui",
            viewport = ?window.viewport_size(),
            assistant_messages = self.messages.len(),
            input_len = input.content_len(),
            input_focused = input.focus_handle_ref().is_focused(window),
            "render root"
        );
        let _ = input;

        let main_content = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.text_primary)
            .text_size(px(14.0))
            .child(self.render_header())
            .child(self.render_message_area(cx))
            .child(self.render_input_area())
            .child(self.render_status_bar());

        let layout = div()
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
            .bg(theme.background)
            .key_context("RootView")
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::dismiss_command_palette))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::submit_prompt))
            .child(main_content);

        let layout = if self.is_sidebar_visible {
            layout.child(self.render_sidebar())
        } else {
            layout
        };

        if self.is_command_palette_open {
            layout.child(self.render_command_palette(cx))
        } else {
            layout
        }
    }
}

impl Focusable for RootView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl RootView {
    fn render_header(&self) -> impl IntoElement {
        let theme = &self.theme;
        let message_count = self.messages.len();

        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_color(theme.accent)
                            .font_weight(FontWeight::BOLD)
                            .child("jfc"),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(12.0))
                            .child("v0.1.0"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(16.0))
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .text_size(px(12.0))
                            .child("claude-opus-4-6"),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(12.0))
                            .child(format!("{} messages", message_count)),
                    ),
            )
    }

    fn render_message_area(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = &self.theme;

        if self.messages.is_empty() {
            return div()
                .id("message-area")
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_color(theme.text_muted)
                        .text_size(px(20.0))
                        .child("What can I help you with?"),
                );
        }

        let mut message_list = div().flex().flex_col().gap(px(12.0)).p(px(16.0));

        for (index, message) in self.messages.iter().enumerate() {
            message_list = message_list.child(self.render_message(message, index, cx));
        }

        div()
            .id("message-area")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .child(message_list)
    }

    fn render_message(&self, message: &ChatMessage, index: usize, cx: &Context<Self>) -> Div {
        let theme = &self.theme;
        let (role_label, role_color, bubble_bg) = match message.role {
            Role::User => ("you", theme.accent, theme.user_bubble),
            Role::Assistant => ("assistant", theme.text_secondary, theme.assistant_bubble),
        };

        let content_elements = self.render_message_parts(&message.parts, index, cx);

        let mut container = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(role_color)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(role_label.to_string()),
            )
            .child(
                div()
                    .w_full()
                    .bg(bubble_bg)
                    .rounded(px(8.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_color(theme.text_primary)
                    .text_size(px(14.0))
                    .children(content_elements),
            );

        if message.role == Role::Assistant {
            if let (Some(agent), Some(model)) = (&message.agent_name, &message.model_name) {
                let mut meta_text = format!("■ {} · {}", agent, model);
                if let Some(cost) = &message.cost_tier {
                    meta_text.push_str(&format!(" ({})", cost));
                }
                if let Some(elapsed) = &message.elapsed {
                    meta_text.push_str(&format!(" · {}", elapsed));
                }
                container = container.child(
                    div()
                        .text_color(theme.text_muted)
                        .text_size(px(11.0))
                        .pt(px(2.0))
                        .child(meta_text),
                );
            }
        }

        container
    }

    fn render_message_parts(
        &self,
        parts: &[MessagePart],
        message_index: usize,
        cx: &Context<Self>,
    ) -> Vec<Div> {
        let mut elements = Vec::new();

        for part in parts {
            match part {
                MessagePart::Text(text) => elements.extend(render_markdown(text, &self.theme)),
                MessagePart::Reasoning(text) => elements.push(self.render_reasoning_part(text)),
                MessagePart::Tool(tool) => {
                    elements.push(self.render_tool_call(tool, message_index, cx));
                }
            }
        }

        elements
    }

    fn render_reasoning_part(&self, text: &str) -> Div {
        div()
            .w_full()
            .border_l_2()
            .border_color(self.theme.border)
            .pl(px(10.0))
            .py(px(4.0))
            .text_color(self.theme.text_muted)
            .text_size(px(12.0))
            .child(text.to_string())
    }

    fn render_tool_call(&self, tool: &ToolCall, message_index: usize, cx: &Context<Self>) -> Div {
        let theme = &self.theme;
        let status_color = match tool.status {
            ToolStatus::Pending => theme.warning,
            ToolStatus::Running => theme.accent,
            ToolStatus::Complete => theme.success,
            ToolStatus::Failed => theme.error,
        };
        let arrow = if tool.is_collapsed { "▶" } else { "▼" };
        let title = format!("{} {} {}", arrow, tool.kind.label(), tool.input.summary());

        let mut container = div()
            .w_full()
            .flex()
            .flex_col()
            .my(px(6.0))
            .border_1()
            .border_color(theme.border)
            .rounded(px(6.0))
            .overflow_hidden()
            .child(
                div()
                    .id(ElementId::Name(
                        format!("tool-{}-{}", message_index, tool.id).into(),
                    ))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .cursor(CursorStyle::PointingHand)
                    .px(px(12.0))
                    .py(px(7.0))
                    .bg(theme.surface)
                    .hover(|style| style.bg(theme.surface_raised))
                    .on_click(cx.listener(move |this, _event, _window, _cx| {
                        this.toggle_tool_collapsed(message_index, _cx);
                    }))
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .text_size(px(12.0))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_color(status_color)
                            .text_size(px(11.0))
                            .child(tool.status.label()),
                    ),
            );

        if !tool.is_collapsed {
            container = container.child(self.render_tool_output(tool));
        }

        container
    }

    fn render_tool_output(&self, tool: &ToolCall) -> Div {
        match &tool.output {
            ToolOutput::Diff(diff) => self.render_diff_view(diff),
            ToolOutput::FileContent {
                path,
                content,
                language,
            } => self.render_file_content(path, content, language),
            ToolOutput::Command {
                stdout,
                stderr,
                exit_code,
            } => self.render_command_output(stdout, stderr, *exit_code),
            ToolOutput::FileList(files) => self.render_file_list(files),
            ToolOutput::Text(text) => self.render_plain_tool_output(text),
            ToolOutput::Empty => self.render_plain_tool_output("No output"),
        }
    }

    fn render_diff_view(&self, diff: &DiffView) -> Div {
        let theme = &self.theme;
        let mut body = div().w_full().flex().flex_col();

        for hunk in &diff.hunks {
            body = body.child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(theme.surface)
                    .text_color(theme.text_muted)
                    .text_size(px(11.0))
                    .child(format!(
                        "@@ -{} +{} @@ {}",
                        hunk.old_start, hunk.new_start, hunk.header
                    )),
            );

            for line in &hunk.lines {
                body = body.child(self.render_diff_line(line));
            }
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .bg(theme.surface_code)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .text_size(px(12.0))
                            .child(diff.file_path.clone()),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(11.0))
                            .child(format!("+{} -{}", diff.additions, diff.deletions)),
                    ),
            )
            .child(body)
    }

    fn render_diff_line(&self, line: &DiffLine) -> Div {
        let theme = &self.theme;
        let (prefix, bg, text_color) = match line.kind {
            DiffLineKind::Context => (" ", theme.surface_code, theme.text_secondary),
            DiffLineKind::Added => ("+", theme.diff_added_bg, theme.diff_added_text),
            DiffLineKind::Removed => ("-", theme.diff_removed_bg, theme.diff_removed_text),
        };
        let old_line = line
            .old_line
            .map_or(String::from("   "), |n| format!("{n:>3}"));
        let new_line = line
            .new_line
            .map_or(String::from("   "), |n| format!("{n:>3}"));

        div()
            .w_full()
            .flex()
            .flex_row()
            .bg(bg)
            .px(px(10.0))
            .py(px(1.0))
            .text_size(px(12.0))
            .text_color(text_color)
            .child(
                div()
                    .w(px(74.0))
                    .text_color(theme.text_muted)
                    .child(format!("{old_line} {new_line} {prefix}")),
            )
            .child(div().child(line.content.clone()))
    }

    fn render_file_content(&self, path: &str, content: &str, language: &str) -> Div {
        let header = if language.is_empty() {
            path.to_string()
        } else {
            format!("{} ({})", path, language)
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .bg(self.theme.surface_code)
            .child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(self.theme.border)
                    .text_color(self.theme.text_secondary)
                    .text_size(px(12.0))
                    .child(header),
            )
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .text_color(self.theme.text_secondary)
                    .text_size(px(12.0))
                    .child(truncate_lines(content, 16)),
            )
    }

    fn render_command_output(&self, stdout: &str, stderr: &str, exit_code: Option<i32>) -> Div {
        let mut output = String::new();
        if !stdout.is_empty() {
            output.push_str(stdout);
        }
        if !stderr.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(stderr);
        }
        if output.is_empty() {
            output.push_str("Command produced no output");
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .bg(self.theme.surface_code)
            .child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(self.theme.border)
                    .text_color(self.theme.text_muted)
                    .text_size(px(11.0))
                    .child(format!(
                        "exit {}",
                        exit_code.map_or(String::from("?"), |c| c.to_string())
                    )),
            )
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .text_color(self.theme.text_secondary)
                    .text_size(px(12.0))
                    .child(truncate_lines(&output, 20)),
            )
    }

    fn render_file_list(&self, files: &[String]) -> Div {
        let mut list = div().w_full().flex().flex_col().gap(px(2.0));
        for file in files.iter().take(24) {
            list = list.child(
                div()
                    .text_color(self.theme.text_secondary)
                    .text_size(px(12.0))
                    .child(format!("• {file}")),
            );
        }
        if files.len() > 24 {
            list = list.child(
                div()
                    .text_color(self.theme.text_muted)
                    .text_size(px(12.0))
                    .child(format!("… {} more", files.len() - 24)),
            );
        }

        div()
            .w_full()
            .bg(self.theme.surface_code)
            .px(px(10.0))
            .py(px(8.0))
            .child(list)
    }

    fn render_plain_tool_output(&self, text: &str) -> Div {
        div()
            .w_full()
            .bg(self.theme.surface_code)
            .px(px(10.0))
            .py(px(8.0))
            .text_color(self.theme.text_secondary)
            .text_size(px(12.0))
            .child(text.to_string())
    }

    fn render_input_area(&self) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .w_full()
            .flex_none()
            .p(px(16.0))
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_full()
                    .bg(theme.surface_raised)
                    .border_1()
                    .border_color(theme.border_focus)
                    .rounded(px(8.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .h(px(44.0))
                    .text_size(px(14.0))
                    .child(self.text_input.clone()),
            )
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .py(px(4.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .text_size(px(11.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .bg(theme.accent_muted)
                            .text_color(theme.accent)
                            .rounded(px(3.0))
                            .px(px(6.0))
                            .py(px(1.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("NORMAL"),
                    )
                    .child(div().text_color(theme.text_muted).child("session: default")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().text_color(theme.text_muted).child("0 tokens"))
                    .child(div().text_color(theme.text_muted).child("$0.00"))
                    .child(div().text_color(theme.success).child("ready")),
            )
    }

    fn render_sidebar(&self) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .w(px(280.0))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .overflow_hidden()
            .child(self.render_sidebar_section("Quick note", vec![("Session", "ses_default")]))
            .child(self.render_sidebar_context())
            .child(self.render_sidebar_mcp())
            .child(self.render_sidebar_lsp())
            .child(self.render_sidebar_footer())
    }

    fn render_sidebar_section(&self, title: &str, items: Vec<(&str, &str)>) -> impl IntoElement {
        let theme = &self.theme;

        let mut section = div()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_uppercase()),
            );

        for (label, value) in items {
            section = section.child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .text_size(px(12.0))
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .text_color(theme.text_primary)
                            .child(value.to_string()),
                    ),
            );
        }

        section
    }

    fn render_sidebar_context(&self) -> impl IntoElement {
        let theme = &self.theme;
        let context_percent: f32 = 0.12;

        div()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("CONTEXT"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .text_size(px(12.0))
                    .child(div().text_color(theme.text_secondary).child("Tokens"))
                    .child(div().text_color(theme.text_primary).child("24,150")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .text_size(px(12.0))
                            .child(div().text_color(theme.text_secondary).child("Context"))
                            .child(
                                div()
                                    .text_color(theme.text_primary)
                                    .child(format!("{:.0}%", context_percent * 100.0)),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(4.0))
                            .bg(theme.border)
                            .rounded(px(2.0))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(context_percent))
                                    .bg(theme.success)
                                    .rounded(px(2.0)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .text_size(px(12.0))
                    .child(div().text_color(theme.text_secondary).child("Output"))
                    .child(div().text_color(theme.text_primary).child("0")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .text_size(px(12.0))
                    .child(div().text_color(theme.text_secondary).child("Cache hit"))
                    .child(div().text_color(theme.text_primary).child("0%")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .text_size(px(12.0))
                    .child(div().text_color(theme.text_secondary).child("Cost"))
                    .child(div().text_color(theme.text_primary).child("$0.00")),
            )
    }

    fn render_sidebar_mcp(&self) -> impl IntoElement {
        let theme = &self.theme;

        let providers = vec![
            ("context7", true),
            ("playwright", false),
            ("filesystem", true),
        ];

        let mut section = div()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("MCP"),
            );

        for (name, connected) in providers {
            let status_color = if connected {
                theme.success
            } else {
                theme.text_muted
            };
            let status_label = if connected { "Connected" } else { "Disabled" };

            section = section.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .bg(status_color),
                    )
                    .child(div().text_color(theme.text_primary).child(name.to_string()))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(10.0))
                            .child(status_label.to_string()),
                    ),
            );
        }

        section
    }

    fn render_sidebar_lsp(&self) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("LSP"),
            )
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(11.0))
                    .child("LSPs will activate as files are read"),
            )
    }

    fn render_sidebar_footer(&self) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .flex_1()
            .w_full()
            .flex()
            .flex_col()
            .justify_end()
            .px(px(12.0))
            .py(px(8.0))
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(11.0))
                    .child("~/RustProjects/active/jfc"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .bg(theme.success),
                    )
                    .child(
                        div()
                            .text_color(theme.text_secondary)
                            .text_size(px(11.0))
                            .child("OpenCode local"),
                    ),
            )
    }

    fn render_command_palette(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = &self.theme;

        let commands = vec![
            ("New Session", "Create a new chat session"),
            ("Clear Messages", "Clear all messages"),
            ("Toggle Sidebar", "Show/hide the sidebar"),
            ("Quit", "Exit the application"),
        ];

        let query_lower = self.command_palette_query.to_lowercase();
        let filtered_commands: Vec<_> = commands
            .into_iter()
            .filter(|(name, _)| {
                query_lower.is_empty() || name.to_lowercase().contains(&query_lower)
            })
            .collect();

        let mut command_list = div().flex().flex_col().w_full();
        for (index, (name, description)) in filtered_commands.iter().enumerate() {
            command_list = command_list.child(
                div()
                    .id(ElementId::Name(format!("cmd-{}", index).into()))
                    .w_full()
                    .px(px(12.0))
                    .py(px(8.0))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .hover(|style| style.bg(theme.surface_raised))
                    .cursor(CursorStyle::PointingHand)
                    .on_click({
                        let name = name.to_string();
                        cx.listener(move |this, _event, _window, cx| {
                            this.is_command_palette_open = false;
                            this.command_palette_query.clear();
                            match name.as_str() {
                                "Clear Messages" => {
                                    this.messages.clear();
                                }
                                "Toggle Sidebar" => {
                                    this.is_sidebar_visible = !this.is_sidebar_visible;
                                }
                                "Quit" => {
                                    cx.quit();
                                }
                                _ => {}
                            }
                            cx.notify();
                        })
                    })
                    .child(
                        div()
                            .text_color(theme.text_primary)
                            .text_size(px(14.0))
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .text_size(px(12.0))
                            .child(description.to_string()),
                    ),
            );
        }

        div()
            .id("command-palette-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(theme.overlay_bg)
            .flex()
            .justify_center()
            .pt(px(100.0))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.is_command_palette_open = false;
                this.command_palette_query.clear();
                cx.notify();
            }))
            .child(
                div()
                    .id("command-palette-inner")
                    .w(px(500.0))
                    .max_h(px(400.0))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_click(|_event, _window, _cx| {})
                    .child(
                        div()
                            .w_full()
                            .px(px(12.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(theme.border)
                            .text_color(theme.text_muted)
                            .text_size(px(14.0))
                            .child(if self.command_palette_query.is_empty() {
                                "Type a command...".to_string()
                            } else {
                                self.command_palette_query.clone()
                            }),
                    )
                    .child(command_list),
            )
    }
}

enum MarkdownBlock {
    Paragraph(Vec<MarkdownSpan>),
    Header(u8, String),
    CodeBlock { language: String, code: String },
}

enum MarkdownSpan {
    Plain(String),
    Bold(String),
    Italic(String),
    InlineCode(String),
}

fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        if line.starts_with("```") {
            let language = line.trim_start_matches('`').trim().to_string();
            let mut code_lines = Vec::new();
            while let Some(code_line) = lines.next() {
                if code_line.starts_with("```") {
                    break;
                }
                code_lines.push(code_line);
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language,
                code: code_lines.join("\n"),
            });
        } else if let Some(header_level) = line
            .bytes()
            .take_while(|&b| b == b'#')
            .count()
            .checked_sub(0)
            .filter(|&count| count > 0 && count <= 6)
        {
            let header_text = line.trim_start_matches('#').trim().to_string();
            if !header_text.is_empty() {
                blocks.push(MarkdownBlock::Header(header_level as u8, header_text));
            } else {
                blocks.push(MarkdownBlock::Paragraph(parse_inline_markdown(line)));
            }
        } else if !line.trim().is_empty() {
            blocks.push(MarkdownBlock::Paragraph(parse_inline_markdown(line)));
        }
    }

    blocks
}

fn parse_inline_markdown(text: &str) -> Vec<MarkdownSpan> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.starts_with("**") {
            if let Some(end) = remaining[2..].find("**") {
                let bold_text = &remaining[2..2 + end];
                spans.push(MarkdownSpan::Bold(bold_text.to_string()));
                remaining = &remaining[2 + end + 2..];
                continue;
            }
        }

        if remaining.starts_with('*') && !remaining.starts_with("**") {
            if let Some(end) = remaining[1..].find('*') {
                let italic_text = &remaining[1..1 + end];
                spans.push(MarkdownSpan::Italic(italic_text.to_string()));
                remaining = &remaining[1 + end + 1..];
                continue;
            }
        }

        if remaining.starts_with('`') {
            if let Some(end) = remaining[1..].find('`') {
                let code_text = &remaining[1..1 + end];
                spans.push(MarkdownSpan::InlineCode(code_text.to_string()));
                remaining = &remaining[1 + end + 1..];
                continue;
            }
        }

        let next_special = remaining
            .find(|c: char| c == '*' || c == '`')
            .unwrap_or(remaining.len());
        if next_special > 0 {
            spans.push(MarkdownSpan::Plain(remaining[..next_special].to_string()));
            remaining = &remaining[next_special..];
        } else {
            spans.push(MarkdownSpan::Plain(remaining.to_string()));
            break;
        }
    }

    spans
}

fn render_markdown(text: &str, theme: &Theme) -> Vec<Div> {
    let blocks = parse_markdown(text);
    let mut elements = Vec::new();

    for block in blocks {
        match block {
            MarkdownBlock::Paragraph(spans) => {
                let mut paragraph = div().flex().flex_row().flex_wrap();
                for span in spans {
                    match span {
                        MarkdownSpan::Plain(text) => {
                            paragraph = paragraph.child(div().child(text));
                        }
                        MarkdownSpan::Bold(text) => {
                            paragraph =
                                paragraph.child(div().font_weight(FontWeight::BOLD).child(text));
                        }
                        MarkdownSpan::Italic(text) => {
                            paragraph = paragraph.child(div().italic().child(text));
                        }
                        MarkdownSpan::InlineCode(text) => {
                            paragraph = paragraph.child(
                                div()
                                    .bg(theme.surface_code)
                                    .rounded(px(3.0))
                                    .px(px(4.0))
                                    .py(px(1.0))
                                    .text_size(px(13.0))
                                    .text_color(theme.code_string)
                                    .child(text),
                            );
                        }
                    }
                }
                elements.push(paragraph);
            }
            MarkdownBlock::Header(level, text) => {
                let font_size = match level {
                    1 => px(22.0),
                    2 => px(18.0),
                    _ => px(16.0),
                };
                elements.push(
                    div()
                        .text_size(font_size)
                        .font_weight(FontWeight::BOLD)
                        .pt(px(8.0))
                        .pb(px(4.0))
                        .child(text),
                );
            }
            MarkdownBlock::CodeBlock { language, code } => {
                elements.push(render_code_block(&language, &code, theme));
            }
        }
    }

    elements
}

fn render_code_block(language: &str, code: &str, theme: &Theme) -> Div {
    let mut block = div()
        .w_full()
        .bg(theme.surface_code)
        .rounded(px(6.0))
        .border_l_2()
        .border_color(theme.accent)
        .my(px(4.0))
        .overflow_hidden();

    if !language.is_empty() {
        block = block.child(
            div()
                .w_full()
                .px(px(12.0))
                .py(px(4.0))
                .flex()
                .flex_row()
                .justify_end()
                .child(
                    div()
                        .text_color(theme.text_muted)
                        .text_size(px(10.0))
                        .child(language.to_string()),
                ),
        );
    }

    let mut code_container = div().w_full().px(px(12.0)).py(px(8.0)).text_size(px(13.0));

    for line in code.lines() {
        code_container = code_container.child(render_syntax_line(line, language, theme));
    }

    block.child(code_container)
}

fn render_syntax_line(line: &str, language: &str, theme: &Theme) -> Div {
    if language != "rust" && language != "rs" {
        return div().text_color(theme.text_primary).child(line.to_string());
    }

    let rust_keywords = [
        "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for", "while",
        "loop", "if", "else", "match", "return", "self", "Self", "super", "crate", "async",
        "await", "move", "ref", "const", "static", "type", "where", "as", "in", "unsafe", "extern",
        "dyn", "true", "false",
    ];

    let trimmed = line.trim_start();

    if trimmed.starts_with("//") {
        return div().text_color(theme.code_comment).child(line.to_string());
    }

    let mut result = div().flex().flex_row();
    let leading_whitespace = &line[..line.len() - trimmed.len()];
    if !leading_whitespace.is_empty() {
        result = result.child(div().child(leading_whitespace.to_string()));
    }

    let mut remaining = trimmed;
    while !remaining.is_empty() {
        if remaining.starts_with('"') {
            if let Some(end) = remaining[1..].find('"') {
                let string_literal = &remaining[..end + 2];
                result = result.child(
                    div()
                        .text_color(theme.code_string)
                        .child(string_literal.to_string()),
                );
                remaining = &remaining[end + 2..];
                continue;
            }
        }

        let mut found_keyword = false;
        for keyword in &rust_keywords {
            if remaining.starts_with(keyword) {
                let after = remaining
                    .get(keyword.len()..keyword.len() + 1)
                    .unwrap_or(" ");
                let is_boundary = !after
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_alphanumeric() || c == '_');
                if is_boundary {
                    result = result.child(
                        div()
                            .text_color(theme.code_keyword)
                            .child(keyword.to_string()),
                    );
                    remaining = &remaining[keyword.len()..];
                    found_keyword = true;
                    break;
                }
            }
        }
        if found_keyword {
            continue;
        }

        if remaining.starts_with(|c: char| c.is_alphabetic() || c == '_') {
            let end = remaining
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(remaining.len());
            let word = &remaining[..end];
            let after_word = remaining.get(end..end + 1).unwrap_or("");

            let color = if after_word == "(" || after_word == "!" {
                theme.code_function
            } else {
                theme.text_primary
            };

            result = result.child(div().text_color(color).child(word.to_string()));
            remaining = &remaining[end..];
            continue;
        }

        let next_interesting = remaining[1..]
            .find(|c: char| c.is_alphabetic() || c == '_' || c == '"')
            .map(|i| i + 1)
            .unwrap_or(remaining.len());
        result = result.child(
            div()
                .text_color(theme.text_primary)
                .child(remaining[..next_interesting].to_string()),
        );
        remaining = &remaining[next_interesting..];
    }

    result
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    "info,jfc=debug,jfc::ui=debug,jfc::text_input=debug,gpui_linux=debug,gpui_wgpu=debug,gpui=debug,gpui::scene=debug,gpui::window=debug,wgpu::renderer=debug"
                        .into()
                }),
        )
        .with_target(true)
        .with_line_number(true)
        .init();

    tracing::info!("jfc starting");

    application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("backspace", text_input::Backspace, Some("TextInput")),
            KeyBinding::new("delete", text_input::Delete, Some("TextInput")),
            KeyBinding::new("left", text_input::Left, Some("TextInput")),
            KeyBinding::new("right", text_input::Right, Some("TextInput")),
            KeyBinding::new("shift-left", text_input::SelectLeft, Some("TextInput")),
            KeyBinding::new("shift-right", text_input::SelectRight, Some("TextInput")),
            KeyBinding::new("ctrl-a", text_input::SelectAll, Some("TextInput")),
            KeyBinding::new("ctrl-v", text_input::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-c", text_input::Copy, Some("TextInput")),
            KeyBinding::new("ctrl-x", text_input::Cut, Some("TextInput")),
            KeyBinding::new("home", text_input::Home, Some("TextInput")),
            KeyBinding::new("end", text_input::End, Some("TextInput")),
            KeyBinding::new("enter", text_input::Submit, Some("TextInput")),
            KeyBinding::new("ctrl-r", text_input::Submit, Some("TextInput")),
            KeyBinding::new("ctrl-r", SubmitPrompt, Some("RootView")),
            KeyBinding::new("ctrl-r", SubmitPrompt, None),
            KeyBinding::new("ctrl-p", ToggleCommandPalette, Some("RootView")),
            KeyBinding::new("escape", DismissCommandPalette, Some("RootView")),
        ]);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);

        let window_options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("jfc".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1200.0), px(800.0)),
                cx,
            ))),
            ..Default::default()
        };

        let window = cx
            .open_window(window_options, |_window, cx| cx.new(RootView::new))
            .expect("failed to open window");

        window
            .update(cx, |view, window, cx| {
                window.focus(&view.text_input.focus_handle(cx), cx);
                cx.activate(true);
            })
            .expect("failed to update window");
    });
}
