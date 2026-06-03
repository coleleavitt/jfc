//! The Claude Design → JFC parity matrix, as data.
//!
//! Each [`Capability`] records a Claude Design feature, where it lives in JFC, and
//! its [`Status`]. `jfc-design capabilities` prints this; the parity roadmap doc is
//! generated from the same source of truth.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Fully implemented natively in JFC.
    Done,
    /// Implemented via an existing JFC subsystem (skills, MCP, tools).
    ViaExisting,
    /// Partially implemented; a piece remains.
    Partial,
    /// Designed but not yet built — needs the browser-host / server phase.
    Planned,
}

impl Status {
    pub fn glyph(self) -> &'static str {
        match self {
            Status::Done => "✅ done",
            Status::ViaExisting => "🔗 via existing",
            Status::Partial => "🟡 partial",
            Status::Planned => "🗺  planned",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Claude Design feature / tool name.
    pub feature: &'static str,
    /// Where it lives in JFC (skill, crate module, tool, MCP, roadmap).
    pub surface: &'static str,
    pub status: Status,
}

/// The full parity matrix.
pub fn matrix() -> Vec<Capability> {
    use Status::*;
    vec![
        cap(
            "base design-agent prompt",
            ".claude/agents/designer.md",
            Done,
        ),
        cap(
            "skill registry (deck, hi-fi, wireframe, prototype, animation, mobile, …)",
            ".claude/skills/jfc-design/*",
            Done,
        ),
        cap(
            "starter components (deck_stage, tweaks_panel, animations, frames, canvas, image_slot)",
            ".claude/skills/jfc-design/starters/*",
            Done,
        ),
        cap(
            "design projects + sandboxed file store + asset registry",
            "jfc-design::project + DesignProject*/Design*File/Design*Asset tools + API",
            Done,
        ),
        cap(
            "super_inline_html (standalone offline HTML)",
            "jfc-design::inline + `jfc-design bundle` + DesignBundleHtml + API",
            Done,
        ),
        cap(
            "handoff package (Handoff to Claude Code)",
            "jfc-design::handoff + `jfc-design handoff` + DesignHandoff + API",
            Done,
        ),
        cap(
            "design-system indexer / check_design_system (discovery + validation)",
            "jfc-design::design_system + `jfc-design ds` + DesignCheckSystem + API",
            Done,
        ),
        cap(
            "save_as_pdf (print-ready HTML)",
            "design server /tools/save-pdf + Playwright PDF + browser print shell",
            Done,
        ),
        cap(
            "read_file / write_file / list_files / copy_files / view_image",
            "DesignReadFile/DesignWriteFile/DesignListFiles/DesignCopyFile + existing JFC tools",
            Done,
        ),
        cap("web_fetch / web_search", "JFC web tools", ViaExisting),
        cap(
            "questions_v2 (ask the user)",
            "JFC AskUserQuestion tool",
            ViaExisting,
        ),
        cap(
            "invoke_skill / tool_search",
            "JFC Skill tool + progressive tool disclosure",
            ViaExisting,
        ),
        cap(
            "GitHub import (github_get_tree / import_files)",
            "JFC GitHub CLI + MCP",
            ViaExisting,
        ),
        cap(
            "Figma MCP (connect_figma, get_design_context, generate_figma_design)",
            "Figma MCP server + send-to-figma skill",
            ViaExisting,
        ),
        cap(
            "Canva import (create-design-import-job)",
            "Canva MCP server + send-to-canva skill",
            ViaExisting,
        ),
        cap(
            "local preview / serve project files",
            "jfc-design::server + `jfc-design serve` + DesignServe + API static routes",
            Done,
        ),
        cap(
            "show_to_user / show_html (live preview pane)",
            "design server iframe host + frontend iframe + injected __om/__DM preview bridge",
            Done,
        ),
        cap(
            "get_webview_logs / eval_js_user_view (browser console + JS eval)",
            "design server /tools/eval-js + Playwright host",
            Done,
        ),
        cap(
            "save_screenshot / multi_screenshot",
            "design server /tools/screenshot + /tools/multi-screenshot + Playwright host",
            Done,
        ),
        cap(
            "done / fork_verifier_agent",
            "design server /tools/verify + /tools/verify-orchestrate + /tools/done multi-viewport reports",
            Done,
        ),
        cap(
            "gen_pptx (editable + screenshot PPTX export)",
            "design server /tools/gen-pptx + Playwright DOM reconstruction + screenshot fallback",
            Done,
        ),
        cap(
            "get_public_file_url",
            "signed expiring share tokens + scoped /design/public/:token host + share manifest/revoke + optional API token auth",
            Done,
        ),
        cap(
            "present_fs_item_for_download / open_for_print",
            "design server download endpoint + browser print path + save-pdf",
            Done,
        ),
        cap(
            "Tweaks transport (postMessage + on-disk persistence)",
            "preview runtime postMessage bridge + design server tweaks persistence",
            Done,
        ),
        cap(
            "direct edit / __om-edit-overrides (DOM→source mapping)",
            "design server inspect/apply endpoints + static HTML, Source Map v3, unique project-text rewrites + overlay fallback",
            Done,
        ),
        cap(
            "Design Components (.dc.html streaming runtime + dc_write)",
            "preview runtime __dcUpdate/dc_html_str_replace bridge + dc_write + dc-stream SSE",
            Done,
        ),
        cap(
            "generate_image (Gemini) / generate_sound (ElevenLabs)",
            "jfc-design::media local provider + external provider command hooks + API/frontend controls",
            Done,
        ),
        cap(
            "React/Vite project workspace frontend (projects, file tree, editor, preview, exports, chat)",
            "apps/design-web",
            Done,
        ),
    ]
}

fn cap(feature: &'static str, surface: &'static str, status: Status) -> Capability {
    Capability {
        feature,
        surface,
        status,
    }
}

/// Render the matrix as a Markdown table.
pub fn markdown_table() -> String {
    let mut s = String::from("| Feature | JFC surface | Status |\n|---|---|---|\n");
    for c in matrix() {
        s.push_str(&format!(
            "| {} | {} | {} |\n",
            c.feature,
            c.surface,
            c.status.glyph()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_is_nonempty_and_renders_normal() {
        let m = matrix();
        assert!(m.len() > 20);
        let table = markdown_table();
        assert!(table.contains("super_inline_html"));
        assert!(table.contains("gen_pptx"));
    }
}
