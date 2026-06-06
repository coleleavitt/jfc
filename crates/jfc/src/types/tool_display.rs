/// Tri-state display mode for a tool block. Replaces three independent
/// bools (`is_collapsed`, `expanded`, `pinned`) so mutually-exclusive
/// states like "collapsed teaser" + "expanded with raised cap" are
/// unrepresentable-by-construction instead of relying on unchecked
/// invariants every renderer + toggle had to obey by hand. `pinned`
/// is associated only with the variants where it makes sense
/// (Default, Expanded) — the Collapsed teaser is never pinned because
/// pinning would make it expand on the next render anyway, so a
/// `Collapsed { pinned: true }` would be incoherent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolDisplayState {
    /// Default rendering: full content, capped at 80 lines (the
    /// preview cap). The user has not asked for either a one-line
    /// teaser or a raised cap. `pinned=true` resists auto-collapse
    /// (e.g. on huge LargeText results) and surfaces the ◆ glyph.
    Default { pinned: bool },
    /// One-line teaser only ("▶ N reads · click to expand").
    /// Set on huge outputs (LargeText that exceed COLLAPSE_LINES /
    /// COLLAPSE_BYTES) that would otherwise dominate the chat, and
    /// on grouped tool runs the user has not opted into.
    Collapsed,
    /// Full content with the cap raised from 80 to 500. Entered via
    /// `Ctrl+O` / `o` / click on the title. `pinned=true` means the
    /// user double-clicked to lock it expanded — only another
    /// double-click can flip it off, so the long Read they wanted to
    /// keep visible while scrolling doesn't silently re-collapse.
    Expanded { pinned: bool },
}

impl ToolDisplayState {
    /// Default rendering, no pin. The construction default for new
    /// tool calls.
    pub const DEFAULT: Self = Self::Default { pinned: false };

    pub fn is_collapsed(&self) -> bool {
        matches!(self, Self::Collapsed)
    }

    pub fn is_expanded(&self) -> bool {
        matches!(self, Self::Expanded { .. })
    }

    pub fn is_pinned(&self) -> bool {
        matches!(
            self,
            Self::Default { pinned: true } | Self::Expanded { pinned: true }
        )
    }

    /// Single source of truth for the renderer's per-row line cap.
    /// Expanded variants raise the cap to 500; everything else uses
    /// the 80-line preview cap. Note: per-output-kind caps in
    /// message_view (e.g. grep at 200/1000) still scale around
    /// `is_expanded()` — the leaf producers keep their own kind-
    /// specific multipliers — but for the generic text/file paths
    /// this is the canonical decision.
    pub fn cap_lines(&self) -> usize {
        if self.is_expanded() { 500 } else { 80 }
    }

    /// Toggle expanded ↔ default behind `o` / `Ctrl+O` /
    /// click-on-title. A pinned-expanded tool collapses back to a
    /// pinned-default; a pinned-default expands to pinned-expanded.
    /// Collapsed (huge LargeText teaser) is left alone — the caller
    /// uses `toggle_collapsed` for that arm so the two-level expand
    /// (teaser ⇄ body, body ⇄ raised-cap) stays distinct.
    pub fn toggle_expanded(&mut self) {
        *self = match *self {
            Self::Default { pinned } => Self::Expanded { pinned },
            Self::Expanded { pinned } => Self::Default { pinned },
            Self::Collapsed => Self::Default { pinned: false },
        };
    }

    /// Toggle the pin glyph on Default + Expanded. Pinning forces
    /// the Expanded state (the renderer needs a body to put the pin
    /// next to); unpinning leaves the cap state alone. Collapsed
    /// can't be pinned by construction, so a pin on a Collapsed
    /// teaser promotes it to a pinned-Expanded body.
    pub fn toggle_pinned(&mut self) {
        *self = match *self {
            Self::Default { pinned } => {
                if pinned {
                    Self::Default { pinned: false }
                } else {
                    Self::Expanded { pinned: true }
                }
            }
            Self::Expanded { pinned } => Self::Expanded { pinned: !pinned },
            Self::Collapsed => Self::Expanded { pinned: true },
        };
    }

    /// Force the teaser state (used when a huge LargeText result
    /// arrives — the dispatcher collapses by default so the chat
    /// isn't drowned).
    pub fn collapse(&mut self) {
        *self = Self::Collapsed;
    }

    /// Toggle between teaser (Collapsed) and body
    /// (Default { pinned: false }). Used by `o` on huge LargeText
    /// outputs where the two-level expand model pivots around
    /// teaser ⇄ body rather than body ⇄ raised-cap.
    pub fn toggle_collapsed(&mut self) {
        *self = match *self {
            Self::Collapsed => Self::Default { pinned: false },
            // From a body state, the user wanted to fold it back to
            // a teaser. Pin status is dropped intentionally — a
            // teaser is never pinned (see enum doc comment).
            Self::Default { .. } | Self::Expanded { .. } => Self::Collapsed,
        };
    }
}

impl Default for ToolDisplayState {
    fn default() -> Self {
        Self::DEFAULT
    }
}
