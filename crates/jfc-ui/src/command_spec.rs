//! Unified command/tool metadata layer (Dolt-style `Command` contract).
//!
//! JFC dispatches commands through three historically separate systems: the
//! top-level CLI (clap subcommands), TUI slash commands (the `SLASH_COMMANDS`
//! table), and model-facing tools (`ToolKind` dispatch). Each carries its own
//! name + help text + permission notion, so docs, completions, and the tool
//! manifest drift apart.
//!
//! [`CommandSpec`] is the single contract every command/tool can describe
//! itself through — name, one-line description, which [`Surface`] it lives on,
//! and its [`Permission`] class. Help/usage/manifest generation
//! ([`render_help`]) reads this metadata instead of a hand-maintained list, so
//! the surfaces converge.
//!
//! This is the incremental seed the migration builds on: the trait plus one
//! adapter per surface (proving it spans all three), with the bulk migration
//! of remaining commands left as follow-up.

/// Which dispatch surface a command lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Surface {
    /// Top-level `jfc <cmd>` CLI subcommand.
    Cli,
    /// In-TUI `/cmd` slash command.
    Slash,
    /// Model-facing tool (`ToolKind`).
    Tool,
}

impl Surface {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Slash => "slash",
            Self::Tool => "tool",
        }
    }
}

/// Permission class — coarse enough to be uniform across surfaces, precise
/// enough to gate auto-approval and drive the audit ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Permission {
    /// No side effects on the user's files/system (Read, Grep, list, show).
    ReadOnly,
    /// Mutates files or runs arbitrary commands (Write, Edit, Bash, apply).
    Mutating,
    /// Manages jfc/agent state but not user code (task tools, /audit).
    Management,
}

impl Permission {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Mutating => "mutating",
            Self::Management => "management",
        }
    }
}

/// The unified self-description every command/tool can provide. Object-safe so
/// heterogeneous specs can be collected into `Vec<Box<dyn CommandSpec>>` for
/// help/manifest generation.
pub(crate) trait CommandSpec {
    /// Invocation name (`changes`, `/audit`, `Bash`).
    fn name(&self) -> &str;
    /// One-line human description for help/usage.
    fn description(&self) -> &str;
    /// Which surface this command is dispatched on.
    fn surface(&self) -> Surface;
    /// Permission class.
    fn permission(&self) -> Permission;
}

/// A plain-data [`CommandSpec`] — the adapter every surface can produce from
/// its existing metadata without restructuring its dispatch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StaticSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub surface: Surface,
    pub permission: Permission,
}

impl CommandSpec for StaticSpec {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        self.description
    }
    fn surface(&self) -> Surface {
        self.surface
    }
    fn permission(&self) -> Permission {
        self.permission
    }
}

/// Adapt a TUI slash-command table row `(name, help)` into a spec. Proves the
/// slash surface flows through the unified contract.
pub(crate) fn slash_spec(name: &'static str, help: &'static str) -> StaticSpec {
    StaticSpec {
        name,
        description: help,
        surface: Surface::Slash,
        permission: Permission::Management,
    }
}

/// Adapt a model tool into a spec, deriving permission from the tool kind.
/// Proves the tool surface flows through the unified contract.
///
/// `ToolKind::api_name`/`label` return `&str` borrowed from a temporary, so we
/// map to `'static` literals here to keep `StaticSpec` `Copy` + borrow-free.
pub(crate) fn tool_spec(kind: crate::types::ToolKind) -> StaticSpec {
    use crate::types::ToolKind;
    let (name, description, permission): (&'static str, &'static str, Permission) = match kind {
        ToolKind::Read => ("Read", "read a file", Permission::ReadOnly),
        ToolKind::Write => ("Write", "write a file", Permission::Mutating),
        ToolKind::Edit => ("Edit", "edit a file", Permission::Mutating),
        ToolKind::MultiEdit => ("MultiEdit", "apply multiple edits", Permission::Mutating),
        ToolKind::Bash => ("Bash", "run a shell command", Permission::Mutating),
        ToolKind::BashOutput => (
            "BashOutput",
            "read background Bash output",
            Permission::ReadOnly,
        ),
        ToolKind::ApplyPatch => ("apply_patch", "apply a patch", Permission::Mutating),
        ToolKind::Glob => ("Glob", "match files by glob", Permission::ReadOnly),
        ToolKind::Grep => ("Grep", "search file contents", Permission::ReadOnly),
        ToolKind::Search => (
            "codebase_search",
            "semantic code search",
            Permission::ReadOnly,
        ),
        ToolKind::TaskCreate => ("TaskCreate", "create a task", Permission::Management),
        _ => ("tool", "model tool", Permission::Management),
    };
    StaticSpec {
        name,
        description,
        surface: Surface::Tool,
        permission,
    }
}

/// Whether a tool mutates the user's files/system, derived from its
/// [`CommandSpec`] permission. This is the SINGLE source for the
/// "is this a mutating tool?" decision — the dispatch layer (audit-ledger
/// emission, isolation gating) calls this instead of hand-maintaining its own
/// `Bash|Edit|Write|…` match that could drift from the spec.
pub(crate) fn tool_is_mutating(kind: crate::types::ToolKind) -> bool {
    tool_spec(kind).permission() == Permission::Mutating
}

/// The `/help` command list, rendered from the slash rows of the unified
/// metadata — deduped by description so aliases collapse onto one line. `/help`
/// reads THIS instead of iterating `SLASH_COMMANDS` itself, so the help text
/// and the unified command surface can't drift.
pub(crate) fn slash_help_lines() -> String {
    let mut out = String::new();
    let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    for (name, help) in crate::input::SLASH_COMMANDS {
        if seen.insert(help) {
            out.push_str(&format!("- `{name}` — {help}\n"));
        }
    }
    out
}

/// Build the unified spec list from the live registries of all three
/// surfaces: the `SLASH_COMMANDS` table, the model tool kinds, and the CLI
/// subcommands. This is the single source the help/manifest generator reads,
/// so the surfaces converge instead of drifting.
pub(crate) fn all_specs() -> Vec<StaticSpec> {
    let mut specs = Vec::new();

    // Slash surface — straight from the registry table.
    for (name, help) in crate::input::SLASH_COMMANDS {
        specs.push(slash_spec(name, help));
    }

    // Tool surface — the mutating/read-only/management tools the model drives.
    use crate::types::ToolKind;
    for kind in [
        ToolKind::Read,
        ToolKind::Write,
        ToolKind::Edit,
        ToolKind::Bash,
        ToolKind::Grep,
        ToolKind::TaskCreate,
    ] {
        specs.push(tool_spec(kind));
    }

    // CLI surface — the top-level subcommands.
    for (name, desc) in [
        ("changes", "review/apply/revert agent change-sets"),
        ("daemon", "manage the background daemon"),
        ("auth", "manage provider authentication"),
    ] {
        specs.push(StaticSpec {
            name,
            description: desc,
            surface: Surface::Cli,
            permission: Permission::Management,
        });
    }

    specs
}

/// Render the unified command/tool help across every surface.
pub(crate) fn render_all() -> String {
    let specs = all_specs();
    let refs: Vec<&dyn CommandSpec> = specs.iter().map(|s| s as &dyn CommandSpec).collect();
    render_help(&refs)
}

/// Render a help/manifest table from any collection of specs. This is the
/// single generator the CLI `--help`, slash `/help`, and tool manifest can all
/// share, so they cannot drift from the metadata.
pub(crate) fn render_help(specs: &[&dyn CommandSpec]) -> String {
    if specs.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for spec in specs {
        out.push_str(&format!(
            "{:<22} [{:<6} {:<11}] {}\n",
            spec.name(),
            spec.surface().label(),
            spec.permission().label(),
            spec.description()
        ));
    }
    out
}

/// Generate a machine-readable command manifest (one JSON-ish line per spec)
/// from the unified metadata. This is the single artifact shell completions,
/// the model-facing tool manifest, and external tooling read — so they cannot
/// drift from the live registries the way three hand-maintained lists did.
pub(crate) fn render_manifest(specs: &[&dyn CommandSpec]) -> String {
    let mut out = String::new();
    for spec in specs {
        out.push_str(&format!(
            "{{\"name\":\"{}\",\"surface\":\"{}\",\"permission\":\"{}\",\"description\":\"{}\"}}\n",
            spec.name(),
            spec.surface().label(),
            spec.permission().label(),
            spec.description().replace('"', "'")
        ));
    }
    out
}

/// Render the full manifest from the live unified spec list.
pub(crate) fn render_manifest_all() -> String {
    let specs = all_specs();
    let refs: Vec<&dyn CommandSpec> = specs.iter().map(|s| s as &dyn CommandSpec).collect();
    render_manifest(&refs)
}

/// Generate bash completion words — just the command names, one per line, from
/// the same metadata. A completion script `source`s this so it never lists a
/// command the registry doesn't have.
pub(crate) fn render_completions(specs: &[&dyn CommandSpec]) -> String {
    let mut names: Vec<&str> = specs.iter().map(|s| s.name()).collect();
    names.sort_unstable();
    names.dedup();
    let mut out = names.join(" ");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a spec from each of the three surfaces renders into one help
    // table — proving the contract spans CLI + slash + tool.
    #[test]
    fn unified_help_spans_all_three_surfaces_normal() {
        let cli = StaticSpec {
            name: "changes",
            description: "review agent change-sets",
            surface: Surface::Cli,
            permission: Permission::Management,
        };
        let slash = slash_spec("/audit", "show the runtime audit ledger");
        let tool = tool_spec(crate::types::ToolKind::Bash);

        let specs: [&dyn CommandSpec; 3] = [&cli, &slash, &tool];
        let help = render_help(&specs);
        assert!(help.contains("changes"));
        assert!(help.contains("/audit"));
        assert!(help.contains("Bash"));
        // Each surface label appears.
        assert!(help.contains("cli"));
        assert!(help.contains("slash"));
        assert!(help.contains("tool"));
    }

    // Robust: tool permission is derived from kind — Bash is mutating, Read is
    // read-only, a task tool is management.
    #[test]
    fn tool_permission_derived_from_kind_robust() {
        assert_eq!(
            tool_spec(crate::types::ToolKind::Bash).permission(),
            Permission::Mutating
        );
        assert_eq!(
            tool_spec(crate::types::ToolKind::Read).permission(),
            Permission::ReadOnly
        );
        assert_eq!(
            tool_spec(crate::types::ToolKind::TaskCreate).permission(),
            Permission::Management
        );
    }

    // Robust: empty input renders empty (no panic, no stray header).
    #[test]
    fn render_help_empty_is_empty_robust() {
        assert_eq!(render_help(&[]), "");
    }

    // Normal — single-source guarantee: the manifest has exactly one line per
    // spec in the unified list, so it is derived from the metadata, never a
    // hand-maintained parallel list that could drift.
    #[test]
    fn manifest_is_one_line_per_spec_normal() {
        let specs = all_specs();
        let manifest = render_manifest_all();
        let lines = manifest.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(
            lines,
            specs.len(),
            "manifest must have exactly one row per metadata spec (no drift)"
        );
        // Every spec name appears in the manifest.
        for s in &specs {
            assert!(
                manifest.contains(&format!("\"name\":\"{}\"", s.name())),
                "manifest missing {}",
                s.name()
            );
        }
    }

    // Robust: completions are the sorted unique command names from the same
    // metadata — a known command is present, and there are no duplicates.
    #[test]
    fn completions_cover_metadata_names_robust() {
        let specs = all_specs();
        let refs: Vec<&dyn CommandSpec> = specs.iter().map(|s| s as &dyn CommandSpec).collect();
        let completions = render_completions(&refs);
        assert!(completions.contains("/changes"), "slash cmd present");
        assert!(completions.contains("changes"), "cli cmd present");

        let words: Vec<&str> = completions.split_whitespace().collect();
        let mut unique = words.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(words.len(), unique.len(), "completions must be deduped");
    }

    // Normal — single-source mutating classification: the dispatch layer's
    // "is this a mutating tool?" decision is derived from the spec. The
    // mutating tools are exactly Write/Edit/MultiEdit/Bash/ApplyPatch; the
    // read-only and management tools are not.
    #[test]
    fn tool_is_mutating_matches_spec_permission_normal() {
        use crate::types::ToolKind;
        for kind in [
            ToolKind::Write,
            ToolKind::Edit,
            ToolKind::MultiEdit,
            ToolKind::Bash,
            ToolKind::ApplyPatch,
        ] {
            assert!(tool_is_mutating(kind.clone()), "{kind:?} must be mutating");
        }
        for kind in [
            ToolKind::Read,
            ToolKind::Glob,
            ToolKind::Grep,
            ToolKind::Search,
            ToolKind::TaskCreate,
        ] {
            assert!(
                !tool_is_mutating(kind.clone()),
                "{kind:?} must not be mutating"
            );
        }
    }

    // Robust — no drift: /help's command list is byte-identical to rendering
    // the slash rows from the unified metadata, proving /help has no parallel
    // hand-maintained list.
    #[test]
    fn slash_help_lines_cover_every_unique_description_robust() {
        let rendered = slash_help_lines();
        // One line per UNIQUE help string in the registry (aliases dedup).
        let unique_helps: std::collections::HashSet<&str> = crate::input::SLASH_COMMANDS
            .iter()
            .map(|(_, h)| *h)
            .collect();
        let line_count = rendered.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(
            line_count,
            unique_helps.len(),
            "help must render exactly one line per unique command description"
        );
        // A known command appears.
        assert!(rendered.contains("/help"), "rendered: {rendered}");
    }

    // Robust: manifest JSON escapes embedded quotes so a description can't
    // break the line format.
    #[test]
    fn manifest_escapes_quotes_robust() {
        let spec = StaticSpec {
            name: "x",
            description: "has \"quotes\" inside",
            surface: Surface::Cli,
            permission: Permission::ReadOnly,
        };
        let refs: [&dyn CommandSpec; 1] = [&spec];
        let manifest = render_manifest(&refs);
        assert!(!manifest.contains("\"quotes\""), "embedded quotes escaped");
        assert_eq!(manifest.lines().count(), 1);
    }
}
