mod text_input;
mod theme;

use gpui::*;
use gpui_platform::application;
use text_input::{SubmitEvent, TextInput};
use theme::Theme;

actions!(
    jfc,
    [ToggleCommandPalette, DismissCommandPalette, ToggleSidebar, Quit]
);

#[derive(Clone, Copy, PartialEq)]
enum Role {
    User,
    Assistant,
}

struct ChatMessage {
    role: Role,
    content: String,
    agent_name: Option<String>,
    model_name: Option<String>,
    cost_tier: Option<String>,
    elapsed: Option<String>,
    tool_name: Option<String>,
    tool_content: Option<String>,
    is_tool_collapsed: bool,
}

impl ChatMessage {
    fn user(content: String) -> Self {
        Self {
            role: Role::User,
            content,
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            tool_name: None,
            tool_content: None,
            is_tool_collapsed: true,
        }
    }

    fn assistant(content: String) -> Self {
        Self {
            role: Role::Assistant,
            content,
            agent_name: Some("Sisyphus - Ultraworker".into()),
            model_name: Some("Anthropic - Claude Opus 4.6".into()),
            cost_tier: Some("$$$$".into()),
            elapsed: Some("3.9s".into()),
            tool_name: None,
            tool_content: None,
            is_tool_collapsed: true,
        }
    }

    fn tool_result(tool_name: String, tool_content: String) -> Self {
        Self {
            role: Role::Assistant,
            content: String::new(),
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            tool_name: Some(tool_name),
            tool_content: Some(tool_content),
            is_tool_collapsed: true,
        }
    }
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
        self.messages
            .push(ChatMessage::tool_result("Read".into(), "/home/cole/RustProjects/active/jfc/crates/jfc-ui/src/main.rs (296 lines)".into()));
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

    fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_sidebar_visible = !self.is_sidebar_visible;
        cx.notify();
    }

    fn toggle_tool_collapsed(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(message) = self.messages.get_mut(index) {
            message.is_tool_collapsed = !message.is_tool_collapsed;
            cx.notify();
        }
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
            if message.tool_name.is_some() {
                message_list =
                    message_list.child(self.render_tool_result(message, index, cx));
            } else {
                message_list = message_list.child(self.render_message(message));
            }
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

    fn render_message(&self, message: &ChatMessage) -> Div {
        let theme = &self.theme;
        let (role_label, role_color, bubble_bg) = match message.role {
            Role::User => ("you", theme.accent, theme.user_bubble),
            Role::Assistant => ("assistant", theme.text_secondary, theme.assistant_bubble),
        };

        let content_elements = render_markdown(&message.content, theme);

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
            if let (Some(agent), Some(model)) =
                (&message.agent_name, &message.model_name)
            {
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

    fn render_tool_result(
        &self,
        message: &ChatMessage,
        index: usize,
        cx: &Context<Self>,
    ) -> Div {
        let theme = &self.theme;
        let tool_name = message
            .tool_name
            .as_deref()
            .unwrap_or("tool");
        let is_collapsed = message.is_tool_collapsed;

        let arrow = if is_collapsed { "▶" } else { "▼" };
        let header_text = format!("{} {}", arrow, tool_name);

        let mut container = div().w_full().flex().flex_col().child(
            div()
                .id(ElementId::Name(format!("tool-{}", index).into()))
                .cursor(CursorStyle::PointingHand)
                .text_color(theme.text_secondary)
                .text_size(px(12.0))
                .px(px(12.0))
                .py(px(4.0))
                .bg(theme.surface)
                .rounded(px(4.0))
                .hover(|style| style.bg(theme.surface_raised))
                .on_click(cx.listener(move |this, _event, _window, _cx| {
                    this.toggle_tool_collapsed(index, _cx);
                }))
                .child(header_text),
        );

        if !is_collapsed {
            if let Some(tool_content) = &message.tool_content {
                container = container.child(
                    div()
                        .w_full()
                        .bg(theme.surface_code)
                        .rounded_b(px(4.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_color(theme.text_secondary)
                        .text_size(px(12.0))
                        .child(tool_content.clone()),
                );
            }
        }

        container
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
                    .border_color(theme.border)
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
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child("session: default"),
                    ),
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
            .child(self.render_sidebar_section(
                "Quick note",
                vec![("Session", "ses_default")],
            ))
            .child(self.render_sidebar_context())
            .child(self.render_sidebar_mcp())
            .child(self.render_sidebar_lsp())
            .child(self.render_sidebar_footer())
    }

    fn render_sidebar_section(
        &self,
        title: &str,
        items: Vec<(&str, &str)>,
    ) -> impl IntoElement {
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
                    .child(div().text_color(theme.text_secondary).child(label.to_string()))
                    .child(div().text_color(theme.text_primary).child(value.to_string())),
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
            let status_label = if connected {
                "Connected"
            } else {
                "Disabled"
            };

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
            let header_text = line
                .trim_start_matches('#')
                .trim()
                .to_string();
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
                            paragraph = paragraph.child(
                                div().font_weight(FontWeight::BOLD).child(text),
                            );
                        }
                        MarkdownSpan::Italic(text) => {
                            paragraph = paragraph.child(
                                div().italic().child(text),
                            );
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

    let mut code_container = div()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .text_size(px(13.0));

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
        "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for",
        "while", "loop", "if", "else", "match", "return", "self", "Self", "super", "crate",
        "async", "await", "move", "ref", "const", "static", "type", "where", "as", "in",
        "unsafe", "extern", "dyn", "true", "false",
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
                let after = remaining.get(keyword.len()..keyword.len() + 1).unwrap_or(" ");
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
