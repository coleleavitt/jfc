//! Render a useful subset of Mermaid and Graphviz/DOT diagrams to themed
//! box-drawing ASCII for the terminal message view.
//!
//! Scope (deliberately small — see `research/codegraph_live_viz/SPEC.md`):
//!   * Mermaid `flowchart`/`graph` (`TD`/`TB`/`BT`/`LR`/`RL`) — nodes + edges.
//!   * Graphviz `digraph`/`graph` — `a -> b` / `a -- b` edges with labels.
//!   * Mermaid `sequenceDiagram` — participants + ordered messages.
//!
//! Anything we don't understand (parse failure, unsupported diagram type)
//! falls back to a clean labelled outline of the raw source rather than
//! erroring — the caller always gets renderable [`Line`]s.
//!
//! The layout is a simple layered (Sugiyama-lite) top-to-bottom DAG draw:
//! nodes are assigned to ranks by longest-path from a root, drawn as boxes,
//! and edges are listed beneath as `from → to` connectors. This is honest
//! about the terminal's limits — it favours legibility over pixel-perfect
//! routing — while still giving the at-a-glance structure the user wants.

use jfc_theme::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use std::collections::{BTreeMap, BTreeSet};
use unicode_width::UnicodeWidthStr;

/// Terminal display width of a string (CJK/emoji aware).
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// True if `lang` (the fenced code-block info string, already trimmed) selects
/// the diagram renderer.
pub fn is_diagram_lang(lang: &str) -> bool {
    matches!(
        lang.trim().to_ascii_lowercase().as_str(),
        "mermaid" | "mmd" | "dot" | "graphviz" | "gv"
    )
}

/// A parsed node: a stable id plus the human label to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    id: String,
    label: String,
}

/// A directed (or undirected) edge with an optional label.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edge {
    from: String,
    to: String,
    label: Option<String>,
    directed: bool,
}

/// The diagram kinds we can lay out structurally.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Graph {
    /// Node/edge graph (flowchart / DOT).
    Flow {
        title: Option<String>,
        nodes: Vec<Node>,
        edges: Vec<Edge>,
    },
    /// Ordered participant messages (sequence diagram).
    Sequence {
        participants: Vec<Node>,
        messages: Vec<Edge>,
    },
}

/// Render a diagram code block to themed lines. `lang` is the fence info string;
/// `src` is the raw block body. Never panics — unsupported input degrades to a
/// labelled outline.
pub fn render(lang: &str, src: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let _linkscope_render = linkscope::phase("markdown.diagram.render");
    linkscope::record_bytes("markdown.diagram.input", usize_to_u64_saturating(src.len()));
    let inner_width = width.saturating_sub(2).max(20);
    let parsed = match lang.trim().to_ascii_lowercase().as_str() {
        "dot" | "graphviz" | "gv" => parse_dot(src),
        _ => parse_mermaid(src),
    };
    match parsed {
        Some(graph) => {
            linkscope::record_items("markdown.diagram.parsed", 1);
            let body = match graph {
                Graph::Flow {
                    title,
                    nodes,
                    edges,
                } => draw_flow(title.as_deref(), &nodes, &edges, theme, inner_width),
                Graph::Sequence {
                    participants,
                    messages,
                } => draw_sequence(&participants, &messages, theme, inner_width),
            };
            frame(body, theme, "diagram")
        }
        None => frame(
            {
                linkscope::record_items("markdown.diagram.fallback", 1);
                fallback_outline(src, theme, inner_width)
            },
            theme,
            "diagram (raw)",
        ),
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Strip an optional `"…"` / `'…'` / `[…]` wrapper from a node label.
fn clean_label(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s);
    s.trim().to_string()
}

/// Pull a node id + display label out of a mermaid node token such as
/// `A`, `A[Label]`, `A(Label)`, `A{Label}`, `A([Label])`, `A[[Label]]`.
fn parse_mermaid_node(token: &str) -> Option<Node> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    // Find the first shape opener; everything before is the id.
    let openers = ['[', '(', '{'];
    if let Some(pos) = token.find(openers) {
        let id = token[..pos].trim().to_string();
        if id.is_empty() {
            return None;
        }
        // Strip the matching shape delimiters greedily from both ends.
        let inner = token[pos..].trim_matches(|c| matches!(c, '[' | ']' | '(' | ')' | '{' | '}'));
        let label = clean_label(inner);
        let label = if label.is_empty() { id.clone() } else { label };
        Some(Node { id, label })
    } else {
        let id = token.to_string();
        Some(Node {
            label: id.clone(),
            id,
        })
    }
}

/// Parse a Mermaid flowchart/graph or sequenceDiagram. Returns `None` for input
/// we don't recognise so the caller can fall back.
fn parse_mermaid(src: &str) -> Option<Graph> {
    let mut lines = src
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.clone().next()?.to_ascii_lowercase();

    if header.starts_with("sequencediagram") {
        return parse_mermaid_sequence(src);
    }
    if !(header.starts_with("flowchart") || header.starts_with("graph")) {
        return None;
    }
    // Consume the header line.
    let _ = lines.next();

    let edge_re = mermaid_edge_re();
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let upsert = |nodes: &mut BTreeMap<String, Node>, order: &mut Vec<String>, n: Node| {
        if !nodes.contains_key(&n.id) {
            order.push(n.id.clone());
        }
        // Prefer a richer (shaped) label over a bare id seen earlier.
        nodes
            .entry(n.id.clone())
            .and_modify(|existing| {
                if existing.label == existing.id && n.label != n.id {
                    existing.label = n.label.clone();
                }
            })
            .or_insert(n);
    };

    for line in lines {
        let line = line.trim_end_matches(';').trim();
        if let Some(caps) = edge_re.captures(line) {
            let left = caps.name("from").map(|m| m.as_str()).unwrap_or("");
            let right = caps.name("to").map(|m| m.as_str()).unwrap_or("");
            let label = caps
                .name("lbl")
                .map(|m| clean_label(m.as_str()))
                .filter(|s| !s.is_empty());
            let arrow = caps.name("arrow").map(|m| m.as_str()).unwrap_or("-->");
            let directed = arrow.contains('>') || arrow.contains('<');
            let (Some(fnode), Some(tnode)) = (parse_mermaid_node(left), parse_mermaid_node(right))
            else {
                continue;
            };
            let (from, to) = (fnode.id.clone(), tnode.id.clone());
            upsert(&mut nodes, &mut order, fnode);
            upsert(&mut nodes, &mut order, tnode);
            edges.push(Edge {
                from,
                to,
                label,
                directed,
            });
        } else if let Some(node) = parse_mermaid_node(line) {
            // A standalone node declaration (no edge).
            if node
                .id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                upsert(&mut nodes, &mut order, node);
            }
        }
    }

    if nodes.is_empty() {
        return None;
    }
    let nodes = order
        .into_iter()
        .filter_map(|id| nodes.get(&id).cloned())
        .collect();
    Some(Graph::Flow {
        title: None,
        nodes,
        edges,
    })
}

/// Parse `sequenceDiagram` participants + ordered messages.
fn parse_mermaid_sequence(src: &str) -> Option<Graph> {
    let msg_re = sequence_msg_re();
    let mut participants: BTreeMap<String, Node> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut messages: Vec<Edge> = Vec::new();

    let add_participant = |id: &str,
                           label: Option<&str>,
                           order: &mut Vec<String>,
                           participants: &mut BTreeMap<String, Node>| {
        let id = id.trim().to_string();
        if id.is_empty() {
            return;
        }
        if !participants.contains_key(&id) {
            order.push(id.clone());
            participants.insert(
                id.clone(),
                Node {
                    label: label.map(clean_label).unwrap_or_else(|| id.clone()),
                    id,
                },
            );
        }
    };

    for line in src.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("sequencediagram") {
            continue;
        }
        if lower.starts_with("participant ") || lower.starts_with("actor ") {
            // Preserve original casing of the id from the raw line.
            let prefix_len = if lower.starts_with("participant ") {
                "participant ".len()
            } else {
                "actor ".len()
            };
            let raw_rest = &line[prefix_len..];
            if let Some((id, label)) = raw_rest.split_once(" as ") {
                add_participant(id, Some(label), &mut order, &mut participants);
            } else {
                add_participant(raw_rest, None, &mut order, &mut participants);
            }
            continue;
        }
        if let Some(caps) = msg_re.captures(line) {
            let from = caps.name("from").map(|m| m.as_str()).unwrap_or("").trim();
            let to = caps.name("to").map(|m| m.as_str()).unwrap_or("").trim();
            let label = caps
                .name("lbl")
                .map(|m| clean_label(m.as_str()))
                .filter(|s| !s.is_empty());
            add_participant(from, None, &mut order, &mut participants);
            add_participant(to, None, &mut order, &mut participants);
            messages.push(Edge {
                from: from.to_string(),
                to: to.to_string(),
                label,
                directed: true,
            });
        }
    }

    if participants.is_empty() {
        return None;
    }
    let participants = order
        .into_iter()
        .filter_map(|id| participants.get(&id).cloned())
        .collect();
    Some(Graph::Sequence {
        participants,
        messages,
    })
}

/// Parse a Graphviz `digraph`/`graph` body: `a -> b [label="x"]` edges and
/// `a [label="X"]` node declarations.
fn parse_dot(src: &str) -> Option<Graph> {
    let lower = src.to_ascii_lowercase();
    let directed_default = lower.contains("digraph");
    if !(lower.contains("digraph") || lower.contains("graph")) {
        return None;
    }
    // Title from `digraph Name {`.
    let title = dot_title_re()
        .captures(src)
        .and_then(|c| c.name("name").map(|m| m.as_str().to_string()))
        .filter(|s| !s.is_empty() && s != "{");

    // Flatten to statements split on `;` and newlines, dropping graph braces.
    let body = src
        .split_once('{')
        .map(|(_, b)| b)
        .unwrap_or(src)
        .rsplit_once('}')
        .map(|(b, _)| b)
        .unwrap_or(src);

    let edge_re = dot_edge_re();
    let node_re = dot_node_re();
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let upsert = |nodes: &mut BTreeMap<String, Node>,
                  order: &mut Vec<String>,
                  id: String,
                  label: Option<String>| {
        if !nodes.contains_key(&id) {
            order.push(id.clone());
            nodes.insert(
                id.clone(),
                Node {
                    label: label.unwrap_or_else(|| id.clone()),
                    id,
                },
            );
        } else if let Some(label) = label {
            if let Some(n) = nodes.get_mut(&id) {
                n.label = label;
            }
        }
    };

    for stmt in body.split([';', '\n']) {
        let stmt = stmt.trim();
        if stmt.is_empty()
            || stmt.starts_with("//")
            || stmt.starts_with('#')
            || stmt.starts_with("rankdir")
            || stmt.starts_with("node ")
            || stmt.starts_with("edge ")
            || stmt.starts_with("graph ")
        {
            continue;
        }
        if let Some(caps) = edge_re.captures(stmt) {
            let from = dot_unquote(caps.name("from").map(|m| m.as_str()).unwrap_or(""));
            let to = dot_unquote(caps.name("to").map(|m| m.as_str()).unwrap_or(""));
            let arrow = caps.name("arrow").map(|m| m.as_str()).unwrap_or("->");
            let label = caps
                .name("lbl")
                .map(|m| clean_label(m.as_str()))
                .filter(|s| !s.is_empty());
            if from.is_empty() || to.is_empty() {
                continue;
            }
            upsert(&mut nodes, &mut order, from.clone(), None);
            upsert(&mut nodes, &mut order, to.clone(), None);
            edges.push(Edge {
                from,
                to,
                label,
                directed: arrow.contains("->"),
            });
        } else if let Some(caps) = node_re.captures(stmt) {
            let id = dot_unquote(caps.name("id").map(|m| m.as_str()).unwrap_or(""));
            if id.is_empty() {
                continue;
            }
            let label = caps.name("lbl").map(|m| clean_label(m.as_str()));
            upsert(&mut nodes, &mut order, id, label);
        }
    }

    if nodes.is_empty() {
        return None;
    }
    // Re-tag default directedness onto edges that used the graph's default.
    let edges = edges
        .into_iter()
        .map(|mut e| {
            e.directed = e.directed || directed_default;
            e
        })
        .collect();
    let nodes = order
        .into_iter()
        .filter_map(|id| nodes.get(&id).cloned())
        .collect();
    Some(Graph::Flow {
        title,
        nodes,
        edges,
    })
}

fn dot_unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim().to_string()
}

// ── Layout + drawing ─────────────────────────────────────────────────────────

/// Assign each node a rank by longest-path from any root (a node with no
/// incoming edge). Cycles are handled by a visited cap. Returns id → rank.
fn rank_nodes(nodes: &[Node], edges: &[Edge]) -> BTreeMap<String, usize> {
    let ids: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut indeg: BTreeMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges {
        if ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()) && e.from != e.to {
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
            *indeg.entry(e.to.as_str()).or_insert(0) += 1;
        }
    }
    let mut rank: BTreeMap<String, usize> = nodes.iter().map(|n| (n.id.clone(), 0)).collect();
    // Roots = indegree 0; if all nodes are in a cycle, seed with the first node.
    let mut frontier: Vec<&str> = indeg
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();
    if frontier.is_empty() {
        if let Some(first) = nodes.first() {
            frontier.push(first.id.as_str());
        }
    }
    let max_iter = nodes.len().saturating_mul(nodes.len()).max(16);
    let mut iters = 0;
    while let Some(cur) = frontier.pop() {
        iters += 1;
        if iters > max_iter {
            break;
        }
        let cur_rank = *rank.get(cur).unwrap_or(&0);
        if let Some(children) = adj.get(cur) {
            for &child in children {
                let cr = rank.get(child).copied().unwrap_or(0);
                if cur_rank + 1 > cr {
                    rank.insert(child.to_string(), cur_rank + 1);
                    frontier.push(child);
                }
            }
        }
    }
    rank
}

/// Draw a node label as a single-line box: `╭─ label ─╮` style, returned as the
/// three rows (top, middle, bottom).
fn node_box(label: &str, style: Style, border: Style, max_w: usize) -> [Line<'static>; 3] {
    let label = truncate(label, max_w.saturating_sub(4).max(3));
    let inner = format!(" {label} ");
    let w = display_width(&inner);
    let top = format!("╭{}╮", "─".repeat(w));
    let bot = format!("╰{}╯", "─".repeat(w));
    [
        Line::from(Span::styled(top, border)),
        Line::from(vec![
            Span::styled("│", border),
            Span::styled(inner, style),
            Span::styled("│", border),
        ]),
        Line::from(Span::styled(bot, border)),
    ]
}

/// Draw a flowchart: ranked node boxes grouped by rank, then an edge list.
fn draw_flow(
    title: Option<&str>,
    nodes: &[Node],
    edges: &[Edge],
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let node_style = theme.style_text_primary;
    let border = theme.style_border;
    let accent = theme.style_accent;
    let muted = theme.style_text_muted;

    if let Some(t) = title {
        out.push(Line::from(Span::styled(
            t.to_string(),
            accent.add_modifier(Modifier::BOLD),
        )));
        out.push(Line::default());
    }

    let label_of: BTreeMap<&str, &str> = nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let ranks = rank_nodes(nodes, edges);
    let max_rank = ranks.values().copied().max().unwrap_or(0);
    let box_w = (width / 3).clamp(10, 40);

    // Group nodes by rank, preserving declaration order within a rank.
    let mut by_rank: BTreeMap<usize, Vec<&Node>> = BTreeMap::new();
    for n in nodes {
        let r = ranks.get(&n.id).copied().unwrap_or(0);
        by_rank.entry(r).or_default().push(n);
    }

    for r in 0..=max_rank {
        let Some(row) = by_rank.get(&r) else { continue };
        // Each rank: render its node boxes stacked (terminal width is tight, so
        // one box per line keeps labels readable), with a rank marker.
        for (i, node) in row.iter().enumerate() {
            let [top, mid, bot] = node_box(&node.label, node_style, border, box_w);
            let connector = if r == 0 && i == 0 { "  " } else { "  " };
            out.push(prefix(connector, top, muted));
            out.push(prefix(connector, mid, muted));
            out.push(prefix(connector, bot, muted));
        }
        if r < max_rank {
            out.push(Line::from(Span::styled("   │".to_string(), border)));
            out.push(Line::from(Span::styled("   ▼".to_string(), accent)));
        }
    }

    // Edge list — the authoritative wiring, since the stacked layout above
    // can't draw every crossing edge faithfully in a terminal.
    let drawable: Vec<&Edge> = edges.iter().collect();
    if !drawable.is_empty() {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            "edges".to_string(),
            muted.add_modifier(Modifier::BOLD),
        )));
        for e in drawable {
            let from = label_of
                .get(e.from.as_str())
                .copied()
                .unwrap_or(e.from.as_str());
            let to = label_of
                .get(e.to.as_str())
                .copied()
                .unwrap_or(e.to.as_str());
            let arrow = if e.directed { "→" } else { "—" };
            let mut spans = vec![
                Span::styled("  ", muted),
                Span::styled(truncate(from, width / 3), node_style),
                Span::styled(format!(" {arrow} "), accent),
                Span::styled(truncate(to, width / 3), node_style),
            ];
            if let Some(lbl) = &e.label {
                spans.push(Span::styled(
                    format!("  ({})", truncate(lbl, width / 4)),
                    muted,
                ));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

/// Draw a sequence diagram: participants on a header row, then ordered messages.
fn draw_sequence(
    participants: &[Node],
    messages: &[Edge],
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let node_style = theme.style_text_primary;
    let accent = theme.style_accent;
    let muted = theme.style_text_muted;
    let border = theme.style_border;

    out.push(Line::from(Span::styled(
        "participants".to_string(),
        muted.add_modifier(Modifier::BOLD),
    )));
    let mut header = vec![Span::styled("  ", muted)];
    for (i, p) in participants.iter().enumerate() {
        if i > 0 {
            header.push(Span::styled("  │  ", border));
        }
        header.push(Span::styled(
            truncate(&p.label, 18),
            node_style.add_modifier(Modifier::BOLD),
        ));
    }
    out.push(Line::from(header));
    out.push(Line::default());

    let label_of: BTreeMap<&str, &str> = participants
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    for (i, m) in messages.iter().enumerate() {
        let from = label_of
            .get(m.from.as_str())
            .copied()
            .unwrap_or(m.from.as_str());
        let to = label_of
            .get(m.to.as_str())
            .copied()
            .unwrap_or(m.to.as_str());
        let mut spans = vec![
            Span::styled(format!("  {:>2}. ", i + 1), muted),
            Span::styled(truncate(from, width / 4), node_style),
            Span::styled(" → ", accent),
            Span::styled(truncate(to, width / 4), node_style),
        ];
        if let Some(lbl) = &m.label {
            spans.push(Span::styled(
                format!(": {}", truncate(lbl, width / 2)),
                muted,
            ));
        }
        out.push(Line::from(spans));
    }
    out
}

/// Fallback: show the raw diagram source as a muted, line-numbered outline so the
/// user still sees the intended content when we can't lay it out.
fn fallback_outline(src: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let muted = theme.style_text_muted;
    let mut out = vec![Line::from(Span::styled(
        "unrecognised diagram — showing source".to_string(),
        muted.add_modifier(Modifier::ITALIC),
    ))];
    for line in src.lines().take(40) {
        out.push(Line::from(Span::styled(
            truncate(line, width),
            theme.style_text_secondary,
        )));
    }
    out
}

// ── Shared rendering helpers ─────────────────────────────────────────────────

/// Wrap a rendered body in the same `┌─ … └─` chrome the code-block renderer
/// uses so diagrams read as distinct units.
fn frame(body: Vec<Line<'static>>, theme: &Theme, label: &str) -> Vec<Line<'static>> {
    let border = theme.style_border;
    let header_style = theme.style_accent.add_modifier(Modifier::BOLD);
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(Line::from(vec![
        Span::styled("┌─ ", border),
        Span::styled(label.to_string(), header_style),
    ]));
    for line in body {
        let mut spans = vec![Span::styled("│ ", border)];
        spans.extend(line.spans);
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled("└─".to_string(), border)));
    out
}

/// Prepend a styled gutter string to a line.
fn prefix(gutter: &str, line: Line<'static>, style: Style) -> Line<'static> {
    let mut spans = vec![Span::styled(gutter.to_string(), style)];
    spans.extend(line.spans);
    Line::from(spans)
}

/// Truncate to a display width, appending `…` when cut.
fn truncate(s: &str, max: usize) -> String {
    let max = max.max(1);
    if display_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = display_width(&ch.to_string());
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

// ── Regexes (compiled once) ──────────────────────────────────────────────────

fn mermaid_edge_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        // `A[x] -->|label| B(y)` / `A --- B` / `A -- text --> B`
        regex::Regex::new(
            r#"(?x)
            ^\s*
            (?P<from>[^\s\-=.][^\-=>]*?)      # source token (incl. shape)
            \s*
            (?P<arrow>--+>?|-\.->|==+>?|--+|<-->|<==>)  # edge arrow
            \s*
            (?:\|(?P<lbl>[^|]*)\|)?           # optional |label|
            \s*
            (?P<to>[^\s].*?)                  # target token
            \s*$
            "#,
        )
        .expect("valid mermaid edge regex")
    })
}

fn sequence_msg_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        // `A->>B: text` / `A-->>B: text` / `A->B`
        regex::Regex::new(
            r#"^\s*(?P<from>\w[\w\-]*?)\s*(?:-->>|->>|-->|->|--)\s*(?P<to>\w[\w\-]*)\s*(?::\s*(?P<lbl>.*))?$"#,
        )
        .expect("valid sequence regex")
    })
}

fn dot_title_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"(?i)(?:di)?graph\s+(?P<name>"[^"]*"|\w+)?\s*\{"#)
            .expect("valid dot title regex")
    })
}

fn dot_edge_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?x)
            ^\s*
            (?P<from>"[^"]+"|[\w.]+)
            \s*(?P<arrow>->|--)\s*
            (?P<to>"[^"]+"|[\w.]+)
            \s*
            (?:\[[^\]]*\blabel\s*=\s*"(?P<lbl>[^"]*)"[^\]]*\])?
            "#,
        )
        .expect("valid dot edge regex")
    })
}

fn dot_node_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r#"^\s*(?P<id>"[^"]+"|[\w.]+)\s*\[[^\]]*\blabel\s*=\s*"(?P<lbl>[^"]*)"[^\]]*\]\s*$"#,
        )
        .expect("valid dot node regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::dark()
    }

    fn graph_of(src: &str) -> Graph {
        parse_mermaid(src).expect("parse")
    }

    #[test]
    fn is_diagram_lang_matches_known_normal() {
        for l in ["mermaid", "MERMAID", "mmd", "dot", "graphviz", "gv"] {
            assert!(is_diagram_lang(l), "{l} should be a diagram lang");
        }
        for l in ["rust", "bash", "json", ""] {
            assert!(!is_diagram_lang(l), "{l} should NOT be a diagram lang");
        }
    }

    #[test]
    fn parse_mermaid_flowchart_nodes_and_edges_normal() {
        let src = "flowchart TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[Done]\n  B -->|no| A";
        let Graph::Flow { nodes, edges, .. } = graph_of(src) else {
            panic!("expected flow");
        };
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"A") && ids.contains(&"B") && ids.contains(&"C"));
        // Labels stripped of shape delimiters.
        let a = nodes.iter().find(|n| n.id == "A").unwrap();
        assert_eq!(a.label, "Start");
        let b = nodes.iter().find(|n| n.id == "B").unwrap();
        assert_eq!(b.label, "Choice");
        assert_eq!(edges.len(), 3);
        let labelled = edges
            .iter()
            .find(|e| e.label.as_deref() == Some("yes"))
            .unwrap();
        assert_eq!(labelled.from, "B");
        assert_eq!(labelled.to, "C");
        assert!(labelled.directed);
    }

    #[test]
    fn parse_mermaid_graph_lr_undirected_robust() {
        let src = "graph LR\n  x --- y";
        let Graph::Flow { edges, .. } = graph_of(src) else {
            panic!("expected flow");
        };
        assert_eq!(edges.len(), 1);
        assert!(!edges[0].directed, "--- is undirected");
    }

    #[test]
    fn parse_sequence_participants_and_messages_normal() {
        let src = "sequenceDiagram\n  participant U as User\n  participant S as Server\n  U->>S: hello\n  S-->>U: hi";
        let g = parse_mermaid(src).expect("parse seq");
        let Graph::Sequence {
            participants,
            messages,
        } = g
        else {
            panic!("expected sequence");
        };
        assert_eq!(participants.len(), 2);
        assert_eq!(participants[0].label, "User");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].from, "U");
        assert_eq!(messages[0].to, "S");
        assert_eq!(messages[0].label.as_deref(), Some("hello"));
    }

    #[test]
    fn parse_dot_digraph_edges_and_labels_normal() {
        let src = "digraph G {\n  rankdir=LR;\n  a -> b [label=\"calls\"];\n  b -> c;\n  c [label=\"Sink\"];\n}";
        let g = parse_dot(src).expect("parse dot");
        let Graph::Flow {
            title,
            nodes,
            edges,
        } = g
        else {
            panic!("expected flow");
        };
        assert_eq!(title.as_deref(), Some("G"));
        assert!(edges.iter().all(|e| e.directed));
        let ab = edges.iter().find(|e| e.from == "a" && e.to == "b").unwrap();
        assert_eq!(ab.label.as_deref(), Some("calls"));
        let c = nodes.iter().find(|n| n.id == "c").unwrap();
        assert_eq!(c.label, "Sink");
    }

    #[test]
    fn parse_dot_undirected_graph_robust() {
        let src = "graph G {\n  a -- b;\n}";
        let g = parse_dot(src).expect("parse");
        let Graph::Flow { edges, .. } = g else {
            panic!("flow");
        };
        assert!(!edges[0].directed);
    }

    #[test]
    fn unrecognised_input_returns_none_robust() {
        assert!(parse_mermaid("this is just prose\nnot a diagram").is_none());
        assert!(parse_dot("not a graph at all").is_none());
    }

    #[test]
    fn render_never_panics_and_frames_output_robust() {
        let th = theme();
        for (lang, src) in [
            ("mermaid", "flowchart TD\n A-->B"),
            ("mermaid", "sequenceDiagram\n A->>B: x"),
            ("dot", "digraph{a->b}"),
            ("mermaid", "garbage input"),
            ("dot", ""),
        ] {
            let lines = render(lang, src, &th, 60);
            assert!(!lines.is_empty(), "{lang}/{src} produced no lines");
            // Framed: first line starts with the top chrome.
            let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(first.starts_with("┌─"), "missing frame header: {first:?}");
        }
    }

    #[test]
    fn rank_nodes_orders_chain_normal() {
        let nodes = vec![
            Node {
                id: "a".into(),
                label: "a".into(),
            },
            Node {
                id: "b".into(),
                label: "b".into(),
            },
            Node {
                id: "c".into(),
                label: "c".into(),
            },
        ];
        let edges = vec![
            Edge {
                from: "a".into(),
                to: "b".into(),
                label: None,
                directed: true,
            },
            Edge {
                from: "b".into(),
                to: "c".into(),
                label: None,
                directed: true,
            },
        ];
        let ranks = rank_nodes(&nodes, &edges);
        assert_eq!(ranks["a"], 0);
        assert_eq!(ranks["b"], 1);
        assert_eq!(ranks["c"], 2);
    }

    #[test]
    fn rank_nodes_terminates_on_cycle_robust() {
        let nodes = vec![
            Node {
                id: "a".into(),
                label: "a".into(),
            },
            Node {
                id: "b".into(),
                label: "b".into(),
            },
        ];
        let edges = vec![
            Edge {
                from: "a".into(),
                to: "b".into(),
                label: None,
                directed: true,
            },
            Edge {
                from: "b".into(),
                to: "a".into(),
                label: None,
                directed: true,
            },
        ];
        // Must not loop forever.
        let ranks = rank_nodes(&nodes, &edges);
        assert!(ranks.contains_key("a") && ranks.contains_key("b"));
    }
}
