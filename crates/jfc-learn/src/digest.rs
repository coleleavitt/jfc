//! Memory digest, dream-instructions, and knowledge wiki.
//!
//! Mirrors Perplexity Computer's memory surface found in the 2026-06-11
//! mindemon dump:
//! - `/rest/sse/computer/memory/dream-instructions` + `dream-settings` — a
//!   user-editable prompt + cadence controlling the overnight "dream"
//!   consolidation pass.
//! - `/rest/sse/computer/memory/digest-settings` — a scheduled brief ("Build an
//!   organised memory from your threads and connected sources once a day";
//!   "Digest runs twice a day").
//! - `/rest/sse/computer/memory/wiki-pages/*` — a topic-grouped knowledge base
//!   built from consolidated memories ("build your internal wiki page").
//!
//! This module is deliberately deterministic and LLM-free: it operates over the
//! same [`MemoryRecord`] set the [`crate::dreamer::Dreamer`] scans, so it is
//! unit-testable and cheap to run on a schedule. A caller that wants
//! LLM-quality prose can feed the [`Digest`]/`WikiPage` outputs to a model, but
//! the structural grouping, recency selection, and cadence logic live here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dreamer::MemoryRecord;

/// How often a scheduled memory job runs. Mirrors Perplexity's dream/digest
/// cadence options ("once a day" / "twice a day").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    /// Disabled — the job never runs automatically.
    Off,
    /// Once per day.
    #[default]
    Daily,
    /// Twice per day (Perplexity's "Digest runs twice a day").
    TwiceDaily,
}

impl Cadence {
    /// Minimum spacing between runs, in seconds. `None` when [`Cadence::Off`].
    pub fn min_interval_secs(self) -> Option<u64> {
        match self {
            Cadence::Off => None,
            Cadence::Daily => Some(24 * 3600),
            Cadence::TwiceDaily => Some(12 * 3600),
        }
    }

    /// Whether a run is due given the last run time and the current time
    /// (both unix seconds). A job that has never run (`last_run = None`) is due
    /// unless it's [`Cadence::Off`].
    pub fn is_due(self, last_run: Option<u64>, now: u64) -> bool {
        let Some(interval) = self.min_interval_secs() else {
            return false;
        };
        match last_run {
            None => true,
            Some(prev) => now.saturating_sub(prev) >= interval,
        }
    }
}

/// User-editable instructions + cadence for the overnight "dream" pass.
/// Mirrors `dream-instructions` + `dream-settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamSettings {
    /// Free-text guidance the user gives the consolidation pass, e.g.
    /// "focus on architecture decisions, drop one-off debugging notes".
    pub instructions: String,
    pub cadence: Cadence,
    /// Last run time (unix seconds), `None` if never run.
    pub last_run: Option<u64>,
}

impl Default for DreamSettings {
    fn default() -> Self {
        Self {
            instructions: String::new(),
            cadence: Cadence::Daily,
            last_run: None,
        }
    }
}

impl DreamSettings {
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    pub fn with_cadence(mut self, cadence: Cadence) -> Self {
        self.cadence = cadence;
        self
    }

    pub fn is_due(&self, now: u64) -> bool {
        self.cadence.is_due(self.last_run, now)
    }

    /// Render the user instructions as a prompt preamble for an LLM-backed
    /// consolidation pass. Empty instructions yield a sensible default.
    pub fn prompt_preamble(&self) -> String {
        let trimmed = self.instructions.trim();
        if trimmed.is_empty() {
            "Consolidate related memories, drop duplicates and stale one-off notes, \
             and keep durable facts."
                .to_owned()
        } else {
            trimmed.to_owned()
        }
    }
}

/// Settings for the scheduled Digest brief. Mirrors `digest-settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSettings {
    pub cadence: Cadence,
    /// Only memories seen within this many seconds are "new" for the brief.
    /// Defaults to one day.
    pub lookback_secs: u64,
    /// Max number of items in the brief.
    pub max_items: usize,
    pub last_run: Option<u64>,
}

impl Default for DigestSettings {
    fn default() -> Self {
        Self {
            cadence: Cadence::Daily,
            lookback_secs: 24 * 3600,
            max_items: 20,
            last_run: None,
        }
    }
}

impl DigestSettings {
    pub fn is_due(&self, now: u64) -> bool {
        self.cadence.is_due(self.last_run, now)
    }
}

/// One line in a digest brief.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestItem {
    pub category: String,
    pub summary: String,
    pub seen_at: Option<u64>,
}

/// A generated digest brief: the "what's new in your memory" summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub generated_at: u64,
    pub items: Vec<DigestItem>,
    /// Number of fresh memories considered (may exceed `items.len()` when
    /// capped by `max_items`).
    pub considered: usize,
}

impl Digest {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Render the digest as a Markdown brief grouped by category.
    pub fn to_markdown(&self) -> String {
        if self.items.is_empty() {
            return "## Memory Digest\n\n_No new memories since the last digest._\n".to_owned();
        }
        let mut by_cat: BTreeMap<&str, Vec<&DigestItem>> = BTreeMap::new();
        for item in &self.items {
            by_cat.entry(item.category.as_str()).or_default().push(item);
        }
        let mut out = String::from("## Memory Digest\n\n");
        out.push_str(&format!(
            "_{} new {} since the last digest._\n\n",
            self.considered,
            if self.considered == 1 {
                "memory"
            } else {
                "memories"
            }
        ));
        for (cat, items) in &by_cat {
            out.push_str(&format!("### {cat}\n"));
            for item in items {
                out.push_str(&format!("- {}\n", item.summary));
            }
            out.push('\n');
        }
        out
    }
}

/// Build a digest brief from the memory set: take memories seen within the
/// lookback window, newest first, capped at `max_items`.
pub fn build_digest(memories: &[MemoryRecord], settings: &DigestSettings, now: u64) -> Digest {
    let cutoff = now.saturating_sub(settings.lookback_secs);
    let mut fresh: Vec<&MemoryRecord> = memories
        .iter()
        .filter(|m| is_active(m))
        .filter(|m| m.last_seen_at.map(|t| t >= cutoff).unwrap_or(false))
        .collect();
    // Newest first; records without a timestamp already filtered out above.
    fresh.sort_by_key(|m| std::cmp::Reverse(m.last_seen_at));

    let considered = fresh.len();
    let items = fresh
        .into_iter()
        .take(settings.max_items)
        .map(|m| DigestItem {
            category: m
                .category
                .clone()
                .unwrap_or_else(|| "UNCATEGORIZED".to_owned()),
            summary: first_line(&m.content, 200),
            seen_at: m.last_seen_at,
        })
        .collect();

    Digest {
        generated_at: now,
        items,
        considered,
    }
}

/// A single knowledge-wiki page: one topic (category) with its memory entries.
/// Mirrors Perplexity's `wiki-pages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPage {
    pub topic: String,
    pub slug: String,
    pub entries: Vec<String>,
}

impl WikiPage {
    pub fn to_markdown(&self) -> String {
        let mut out = format!("# {}\n\n", self.topic);
        for entry in &self.entries {
            out.push_str(&format!("- {entry}\n"));
        }
        out
    }
}

/// A generated knowledge wiki: one page per memory category, sorted by topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wiki {
    pub pages: Vec<WikiPage>,
}

impl Wiki {
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn page(&self, slug: &str) -> Option<&WikiPage> {
        self.pages.iter().find(|p| p.slug == slug)
    }

    /// Render an index of all pages plus their contents.
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("# Knowledge Wiki\n\n");
        if self.pages.is_empty() {
            out.push_str("_No knowledge pages yet._\n");
            return out;
        }
        out.push_str("## Index\n\n");
        for page in &self.pages {
            out.push_str(&format!("- [{}](#{})\n", page.topic, page.slug));
        }
        out.push('\n');
        for page in &self.pages {
            out.push_str(&page.to_markdown());
            out.push('\n');
        }
        out
    }
}

/// Build a knowledge wiki from the active memory set, grouping by category into
/// one page per topic. Entries within a page are de-duplicated by their
/// normalized hash (falling back to the first content line).
pub fn build_wiki(memories: &[MemoryRecord]) -> Wiki {
    let mut by_topic: BTreeMap<String, Vec<&MemoryRecord>> = BTreeMap::new();
    for mem in memories.iter().filter(|m| is_active(m)) {
        let topic = mem
            .category
            .clone()
            .unwrap_or_else(|| "UNCATEGORIZED".to_owned());
        by_topic.entry(topic).or_default().push(mem);
    }

    let pages = by_topic
        .into_iter()
        .map(|(topic, mems)| {
            let mut seen_hashes = std::collections::HashSet::new();
            let mut entries = Vec::new();
            for mem in mems {
                let dedup_key = mem
                    .normalized_hash
                    .clone()
                    .unwrap_or_else(|| first_line(&mem.content, usize::MAX));
                if seen_hashes.insert(dedup_key) {
                    entries.push(first_line(&mem.content, 240));
                }
            }
            WikiPage {
                slug: slugify(&topic),
                topic,
                entries,
            }
        })
        .filter(|p| !p.entries.is_empty())
        .collect();

    Wiki { pages }
}

fn is_active(m: &MemoryRecord) -> bool {
    m.memory_status.as_deref().unwrap_or("active") == "active"
}

/// First non-empty line of `content`, trimmed and capped at `max` chars on a
/// char boundary.
fn first_line(content: &str, max: usize) -> String {
    let line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_owned();
    if max == usize::MAX || line.chars().count() <= max {
        return line;
    }
    let end = line.floor_char_boundary(max);
    format!("{}…", &line[..end])
}

/// Lowercase, hyphenated slug for a wiki page anchor.
fn slugify(topic: &str) -> String {
    let mut slug = String::with_capacity(topic.len());
    let mut prev_dash = false;
    for ch in topic.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(cat: &str, content: &str, seen_at: Option<u64>) -> MemoryRecord {
        MemoryRecord {
            path: format!("/mem/{cat}.md"),
            category: Some(cat.to_owned()),
            normalized_hash: Some(format!("{cat}-{}", content.len())),
            content: content.to_owned(),
            last_seen_at: seen_at,
            memory_status: Some("active".to_owned()),
        }
    }

    // ── Cadence ──────────────────────────────────────────────────────────────

    #[test]
    fn cadence_due_when_never_run_normal() {
        assert!(Cadence::Daily.is_due(None, 1000));
        assert!(Cadence::TwiceDaily.is_due(None, 1000));
        assert!(!Cadence::Off.is_due(None, 1000));
    }

    #[test]
    fn cadence_respects_interval_normal() {
        let day = 24 * 3600;
        // Just under a day → not due; at/over → due.
        assert!(!Cadence::Daily.is_due(Some(0), day - 1));
        assert!(Cadence::Daily.is_due(Some(0), day));
        // Twice daily spaces at 12h.
        assert!(!Cadence::TwiceDaily.is_due(Some(0), 12 * 3600 - 1));
        assert!(Cadence::TwiceDaily.is_due(Some(0), 12 * 3600));
    }

    #[test]
    fn dream_settings_default_prompt_when_empty_normal() {
        let s = DreamSettings::default();
        assert!(s.prompt_preamble().contains("Consolidate"));
        let s = s.with_instructions("  focus on rust patterns  ");
        assert_eq!(s.prompt_preamble(), "focus on rust patterns");
    }

    // ── Digest ───────────────────────────────────────────────────────────────

    #[test]
    fn build_digest_selects_fresh_newest_first_normal() {
        let now = 100_000u64;
        let settings = DigestSettings {
            cadence: Cadence::Daily,
            lookback_secs: 3600,
            max_items: 10,
            last_run: None,
        };
        let memories = vec![
            mem("ARCH", "recent decision", Some(now - 100)),
            mem("BUG", "older but fresh", Some(now - 3000)),
            mem("STALE", "too old", Some(now - 10_000)),
        ];
        let digest = build_digest(&memories, &settings, now);
        assert_eq!(digest.considered, 2);
        assert_eq!(digest.items.len(), 2);
        // Newest first.
        assert_eq!(digest.items[0].summary, "recent decision");
        assert_eq!(digest.items[1].summary, "older but fresh");
    }

    #[test]
    fn build_digest_caps_at_max_items_robust() {
        let now = 1000u64;
        let settings = DigestSettings {
            cadence: Cadence::Daily,
            lookback_secs: 10_000,
            max_items: 2,
            last_run: None,
        };
        let memories: Vec<MemoryRecord> = (0..5)
            .map(|i| mem("X", &format!("item {i}"), Some(now - i)))
            .collect();
        let digest = build_digest(&memories, &settings, now);
        assert_eq!(digest.considered, 5);
        assert_eq!(digest.items.len(), 2);
    }

    #[test]
    fn build_digest_empty_renders_placeholder_robust() {
        let settings = DigestSettings::default();
        let digest = build_digest(&[], &settings, 1000);
        assert!(digest.is_empty());
        assert!(digest.to_markdown().contains("No new memories"));
    }

    #[test]
    fn digest_markdown_groups_by_category_normal() {
        let now = 1000u64;
        let settings = DigestSettings {
            cadence: Cadence::Daily,
            lookback_secs: 10_000,
            max_items: 10,
            last_run: None,
        };
        let memories = vec![
            mem("ARCH", "use traits", Some(now - 1)),
            mem("ARCH", "prefer enums", Some(now - 2)),
            mem("BUG", "off-by-one", Some(now - 3)),
        ];
        let md = build_digest(&memories, &settings, now).to_markdown();
        assert!(md.contains("### ARCH"));
        assert!(md.contains("### BUG"));
        assert!(md.contains("- use traits"));
    }

    // ── Wiki ─────────────────────────────────────────────────────────────────

    #[test]
    fn build_wiki_one_page_per_topic_normal() {
        let memories = vec![
            mem("Architecture", "trait-based design", Some(1)),
            mem("Architecture", "no god objects", Some(2)),
            mem("Testing", "robust + normal naming", Some(3)),
        ];
        let wiki = build_wiki(&memories);
        assert_eq!(wiki.pages.len(), 2);
        let arch = wiki.page("architecture").expect("arch page");
        assert_eq!(arch.entries.len(), 2);
        assert!(wiki.page("testing").is_some());
    }

    #[test]
    fn build_wiki_dedups_by_hash_robust() {
        let mut dup = mem("Architecture", "trait-based design", Some(1));
        let mut dup2 = mem("Architecture", "trait-based design", Some(2));
        // Same normalized hash → one entry.
        dup.normalized_hash = Some("same".to_owned());
        dup2.normalized_hash = Some("same".to_owned());
        let wiki = build_wiki(&[dup, dup2]);
        assert_eq!(wiki.pages.len(), 1);
        assert_eq!(wiki.pages[0].entries.len(), 1);
    }

    #[test]
    fn build_wiki_skips_archived_robust() {
        let mut archived = mem("Architecture", "old note", Some(1));
        archived.memory_status = Some("archived".to_owned());
        let wiki = build_wiki(&[archived]);
        assert!(wiki.is_empty());
    }

    #[test]
    fn wiki_markdown_has_index_and_pages_normal() {
        let memories = vec![mem("Architecture", "trait-based design", Some(1))];
        let md = build_wiki(&memories).to_markdown();
        assert!(md.contains("# Knowledge Wiki"));
        assert!(md.contains("## Index"));
        assert!(md.contains("# Architecture"));
        assert!(md.contains("- trait-based design"));
    }

    #[test]
    fn slugify_handles_spaces_and_punctuation_robust() {
        assert_eq!(slugify("Architecture Decisions!"), "architecture-decisions");
        assert_eq!(slugify("  Multi   Word  "), "multi-word");
        assert_eq!(slugify("ALL_CAPS_CAT"), "all-caps-cat");
    }
}
