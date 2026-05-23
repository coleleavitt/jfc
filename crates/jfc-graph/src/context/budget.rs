//! Adaptive output budget for context/explore queries.
//!
//! Scales output caps by indexed-file count so a focused query on a tiny
//! project doesn't dump a whole file's worth of source into the agent's
//! context, while a large project still gets generous output where the
//! agent's native discovery cost (grep + find + many Reads) would
//! genuinely dwarf the fat explore payload.
//!
//! Tier breakpoints intentionally mirror codegraph's
//! `getExploreOutputBudget` (< 500, < 5000, < 15 000, < 25 000+) so a
//! project sits in the same tier across both knobs and the agent's
//! mental model carries over.

/// Adaptive output budget — every knob a context formatter needs.
#[derive(Debug, Clone, Copy)]
pub struct ExploreBudget {
    /// Hard cap on the total assembled output, in characters.
    pub max_output_chars: usize,
    /// Default file count when the caller didn't specify `max_files`.
    pub default_max_files: usize,
    /// Cap on contiguous source returned per file (sum of all clusters).
    pub max_chars_per_file: usize,
    /// Cluster-merge gap threshold, in source lines.
    pub gap_threshold: u32,
    /// Cap on symbols listed in the per-file header
    /// (`#### path — sym(kind), ...`).
    pub max_symbols_in_file_header: usize,
    /// Cap on edges shown per relationship kind in the
    /// "Relationships" section.
    pub max_edges_per_relationship_kind: usize,
    /// Include the "Relationships" section.
    pub include_relationships: bool,
    /// Include the trailing "Additional relevant files (not shown)" list.
    pub include_additional_files: bool,
    /// Include the "Complete source code is included above…" reminder.
    pub include_completeness_signal: bool,
    /// Include the explore-budget reminder at the end.
    pub include_budget_note: bool,
}

impl ExploreBudget {
    /// Resolve the budget for a given indexed-file count. Tiers mirror
    /// codegraph's `getExploreOutputBudget` so cross-tool advice
    /// transfers.
    pub fn for_file_count(file_count: usize) -> Self {
        if file_count < 500 {
            Self {
                max_output_chars: 18_000,
                default_max_files: 5,
                max_chars_per_file: 3_800,
                gap_threshold: 8,
                max_symbols_in_file_header: 6,
                max_edges_per_relationship_kind: 6,
                include_relationships: true,
                include_additional_files: false,
                include_completeness_signal: false,
                include_budget_note: false,
            }
        } else if file_count < 5_000 {
            Self {
                max_output_chars: 13_000,
                default_max_files: 6,
                max_chars_per_file: 2_500,
                gap_threshold: 10,
                max_symbols_in_file_header: 8,
                max_edges_per_relationship_kind: 8,
                include_relationships: true,
                include_additional_files: true,
                include_completeness_signal: true,
                include_budget_note: true,
            }
        } else if file_count < 15_000 {
            Self {
                max_output_chars: 35_000,
                default_max_files: 12,
                max_chars_per_file: 7_000,
                gap_threshold: 15,
                max_symbols_in_file_header: 15,
                max_edges_per_relationship_kind: 15,
                include_relationships: true,
                include_additional_files: true,
                include_completeness_signal: true,
                include_budget_note: true,
            }
        } else {
            Self {
                max_output_chars: 38_000,
                default_max_files: 14,
                max_chars_per_file: 7_000,
                gap_threshold: 15,
                max_symbols_in_file_header: 15,
                max_edges_per_relationship_kind: 15,
                include_relationships: true,
                include_additional_files: true,
                include_completeness_signal: true,
                include_budget_note: true,
            }
        }
    }

    /// Recommended `explore` call budget for the given project size.
    /// Surfaced in the tool description so the agent knows when to stop
    /// fanning out queries and answer.
    pub fn call_budget(file_count: usize) -> usize {
        if file_count < 500 {
            1
        } else if file_count < 5_000 {
            2
        } else if file_count < 15_000 {
            3
        } else if file_count < 25_000 {
            4
        } else {
            5
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_project_gets_tight_budget() {
        let b = ExploreBudget::for_file_count(100);
        assert_eq!(b.default_max_files, 5);
        assert_eq!(b.max_chars_per_file, 3_800);
        assert!(!b.include_additional_files);
        assert!(!b.include_completeness_signal);
        assert!(!b.include_budget_note);
    }

    #[test]
    fn small_project_includes_signals() {
        let b = ExploreBudget::for_file_count(2_000);
        assert_eq!(b.default_max_files, 6);
        assert!(b.include_additional_files);
        assert!(b.include_completeness_signal);
        assert!(b.include_budget_note);
    }

    #[test]
    fn medium_project_gets_generous_caps() {
        let b = ExploreBudget::for_file_count(10_000);
        assert_eq!(b.default_max_files, 12);
        assert_eq!(b.max_chars_per_file, 7_000);
        assert_eq!(b.max_output_chars, 35_000);
    }

    #[test]
    fn huge_project_hits_top_tier() {
        let b = ExploreBudget::for_file_count(50_000);
        assert_eq!(b.default_max_files, 14);
        assert_eq!(b.max_output_chars, 38_000);
    }

    #[test]
    fn call_budget_scales_by_tier() {
        assert_eq!(ExploreBudget::call_budget(100), 1);
        assert_eq!(ExploreBudget::call_budget(2_000), 2);
        assert_eq!(ExploreBudget::call_budget(10_000), 3);
        assert_eq!(ExploreBudget::call_budget(20_000), 4);
        assert_eq!(ExploreBudget::call_budget(50_000), 5);
    }

    #[test]
    fn boundary_500_promotes_to_tier_2() {
        let b = ExploreBudget::for_file_count(500);
        assert_eq!(b.default_max_files, 6);
        assert!(b.include_additional_files);
    }
}
