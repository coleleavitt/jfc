use std::path::{Path, PathBuf};

use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 500;
const MARKERS: &[&str] = &["TODO", "FIXME", "BUG", "HACK", "XXX"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceTodo {
    file: PathBuf,
    line: usize,
    marker: &'static str,
    text: String,
}

pub(super) async fn cmd_todo_tree(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));

    let limit = match parse_limit(parts.get(1).copied()) {
        Ok(limit) => limit,
        Err(message) => {
            state.messages.push(ChatMessage::assistant(message));
            return;
        }
    };
    let cwd = PathBuf::from(&state.cwd);
    state
        .messages
        .push(ChatMessage::assistant(render_todo_tree(&cwd, limit)));
}

fn parse_limit(arg: Option<&str>) -> Result<usize, String> {
    let Some(raw) = arg.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(DEFAULT_LIMIT);
    };
    let limit = raw
        .parse::<usize>()
        .map_err(|_| "Usage: `/todo-tree [limit]` where limit is 1..500.".to_owned())?;
    if limit == 0 || limit > MAX_LIMIT {
        return Err("Usage: `/todo-tree [limit]` where limit is 1..500.".to_owned());
    }
    Ok(limit)
}

fn render_todo_tree(root: &Path, limit: usize) -> String {
    let todos = collect_todos(root, limit);
    if todos.is_empty() {
        return "No source TODO markers found.".to_owned();
    }

    let capped = todos.len() == limit;
    let mut body = format!("**Source TODO tree** ({} marker(s)", todos.len());
    if capped {
        body.push_str(&format!(", capped at {limit}"));
    }
    body.push_str(")\n\n");

    let mut current_file: Option<&Path> = None;
    for todo in &todos {
        if current_file != Some(todo.file.as_path()) {
            current_file = Some(todo.file.as_path());
            body.push_str(&format!("- `{}`\n", todo.file.display()));
        }
        body.push_str(&format!(
            "  - L{} {}: {}\n",
            todo.line, todo.marker, todo.text
        ));
    }

    body
}

fn collect_todos(root: &Path, limit: usize) -> Vec<SourceTodo> {
    let mut todos = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .filter_entry(|entry| !is_skipped_dir(entry.path()))
        .build();

    for entry in walker.flatten() {
        if todos.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|ty| ty.is_file()) || !is_source_file(entry.path()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        collect_file_todos(relative, &content, limit, &mut todos);
    }

    todos.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    todos
}

fn collect_file_todos(relative: &Path, content: &str, limit: usize, todos: &mut Vec<SourceTodo>) {
    for (line_idx, line) in content.lines().enumerate() {
        if todos.len() >= limit {
            break;
        }
        let Some((marker, marker_pos)) = find_marker(line) else {
            continue;
        };
        let rest_start = marker_pos + marker.len();
        let text = line[rest_start..]
            .trim_start_matches(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
            .trim();
        todos.push(SourceTodo {
            file: relative.to_path_buf(),
            line: line_idx + 1,
            marker,
            text: if text.is_empty() {
                marker.to_owned()
            } else {
                text.to_owned()
            },
        });
    }
}

fn find_marker(line: &str) -> Option<(&'static str, usize)> {
    let uppercase = line.to_ascii_uppercase();
    MARKERS.iter().find_map(|marker| {
        uppercase
            .match_indices(marker)
            .find(|(pos, _)| marker_has_word_boundaries(&uppercase, *pos, marker.len()))
            .map(|(pos, _)| (*marker, pos))
    })
}

fn marker_has_word_boundaries(line: &str, pos: usize, len: usize) -> bool {
    let before = line[..pos].chars().next_back();
    let after = line[pos + len..].chars().next();
    !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_skipped_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "target" | "node_modules" | "dist" | "build"))
}

fn is_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(name, "Dockerfile" | "Makefile" | "Justfile") {
        return true;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "c" | "cc"
                    | "cpp"
                    | "css"
                    | "go"
                    | "h"
                    | "hpp"
                    | "html"
                    | "java"
                    | "js"
                    | "jsx"
                    | "json"
                    | "kt"
                    | "md"
                    | "nix"
                    | "py"
                    | "rs"
                    | "scss"
                    | "sh"
                    | "swift"
                    | "toml"
                    | "ts"
                    | "tsx"
                    | "txt"
                    | "vue"
                    | "yaml"
                    | "yml"
                    | "zig"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_tree_lists_grouped_source_markers_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let knowledge = dir.path().join("crates/jfc-knowledge/src");
        let learn = dir.path().join("crates/jfc-learn/src");
        std::fs::create_dir_all(&knowledge).unwrap();
        std::fs::create_dir_all(&learn).unwrap();
        std::fs::write(
            knowledge.join("lib.rs"),
            "// TODO 7+8: verified salience outranks unverified\nfn ok() {}\n",
        )
        .unwrap();
        std::fs::write(
            learn.join("digest.rs"),
            "pub fn digest() {}\n// FIXME: fold mined lessons into candidates\n",
        )
        .unwrap();

        let body = render_todo_tree(dir.path(), DEFAULT_LIMIT);

        assert!(body.contains("**Source TODO tree** (2 marker(s))"));
        assert!(body.contains("- `crates/jfc-knowledge/src/lib.rs`"));
        assert!(body.contains("L1 TODO: 7+8: verified salience outranks unverified"));
        assert!(body.contains("- `crates/jfc-learn/src/digest.rs`"));
        assert!(body.contains("L2 FIXME: fold mined lessons into candidates"));
    }

    #[test]
    fn todo_tree_ignores_identifier_substrings_regression() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "let methodologist = true;\n// todo: real item\n",
        )
        .unwrap();

        let todos = collect_todos(dir.path(), DEFAULT_LIMIT);

        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "real item");
    }

    #[test]
    fn todo_tree_command_is_registered_normal() {
        assert!(
            crate::commands::ENGINE_SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/todo-tree")
        );
    }
}
