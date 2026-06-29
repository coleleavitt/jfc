//! Recurring user-prompt → skill miner.
//!
//! Users reword the same request many ways ("git diff stage commit and push for
//! f" vs "…for p"). This clusters user prompts by a normalized keyword
//! SIGNATURE — a cheap, deterministic stand-in for semantic similarity (true
//! embeddings can refine it later) — and surfaces the intents that recur often
//! enough to deserve their own skill/command. The signature drops harness
//! system-reminder noise and chat filler so only the substantive intent remains.

use std::collections::HashMap;

/// Words too generic to characterize an intent (english + JFC chat filler).
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "you", "your", "can", "could", "would", "please", "this", "that", "with",
    "from", "what", "when", "where", "which", "into", "right", "like", "just", "really",
    "actually", "stuff", "thing", "things", "want", "need", "make", "made", "does", "done", "here",
    "there", "then", "than", "they", "them", "have", "has", "had", "not", "but", "all", "any",
    "our", "out", "get", "got", "see", "let", "its", "are", "was", "were", "will", "should",
    "also", "about", "over", "more", "some", "very", "much", "etc", "idk", "able", "even", "give",
    "find",
];

/// A cluster of user prompts that share an intent signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCluster {
    /// Shared significant keywords (sorted) — the intent fingerprint.
    pub signature: Vec<String>,
    /// How many prompts fell in this cluster.
    pub count: usize,
    /// A few representative prompts (deduped, truncated).
    pub examples: Vec<String>,
}

/// Normalize a prompt to its intent signature: strip system-reminder / continue
/// noise, lowercase, keep significant words (len >= 4, not stopwords), dedup +
/// sort, cap to the most distinctive few. Empty/short when the prompt is filler.
pub fn prompt_signature(prompt: &str) -> Vec<String> {
    let cleaned: String = prompt
        .lines()
        .filter(|line| {
            let t = line.trim_start().to_ascii_lowercase();
            !t.starts_with("<system-reminder")
                && !t.contains("</system-reminder")
                && !t.starts_with("continue")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut words: Vec<String> = cleaned
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(w))
        .map(str::to_owned)
        .collect();
    words.sort();
    words.dedup();
    words.truncate(8); // bound the signature so one long prompt can't dominate
    words
}

/// Cluster prompts by signature; return clusters recurring at least `min_count`
/// times, most-frequent first. Prompts whose signature is too thin (< 2 words)
/// are skipped — not enough signal to be a reusable intent.
pub fn mine_user_prompt_skills(prompts: &[String], min_count: usize) -> Vec<PromptCluster> {
    let mut groups: HashMap<Vec<String>, Vec<String>> = HashMap::new();
    for p in prompts {
        let sig = prompt_signature(p);
        if sig.len() < 2 {
            continue;
        }
        groups
            .entry(sig)
            .or_default()
            .push(p.trim().chars().take(120).collect());
    }
    let mut clusters: Vec<PromptCluster> = groups
        .into_iter()
        .filter(|(_, ps)| ps.len() >= min_count)
        .map(|(signature, mut ps)| {
            let count = ps.len();
            ps.sort();
            ps.dedup();
            ps.truncate(3);
            PromptCluster {
                signature,
                count,
                examples: ps,
            }
        })
        .collect();
    clusters.sort_by(|a, b| b.count.cmp(&a.count).then(a.signature.cmp(&b.signature)));
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_reworded_same_intent_regression() {
        // The real example from the data: "git diff stage commit push" reworded.
        let prompts = vec![
            "can you git diff stage commit and push for f".to_owned(),
            "can you git diff stage commit and push for p".to_owned(),
            "git diff stage commit push please".to_owned(),
        ];
        let clusters = mine_user_prompt_skills(&prompts, 2);
        assert_eq!(clusters.len(), 1, "all three are the same intent");
        assert_eq!(clusters[0].count, 3);
        assert!(clusters[0].signature.contains(&"commit".to_owned()));
        assert!(clusters[0].signature.contains(&"push".to_owned()));
    }

    #[test]
    fn ignores_system_reminders_and_filler_normal() {
        let prompts = vec![
            "<system-reminder>\ncontinue the remaining".to_owned(),
            "idk right just do it please".to_owned(),
            "continue".to_owned(),
        ];
        assert!(mine_user_prompt_skills(&prompts, 1).is_empty());
    }

    #[test]
    fn below_threshold_not_returned_normal() {
        let prompts = vec!["implement the voice recorder module".to_owned()];
        assert!(mine_user_prompt_skills(&prompts, 2).is_empty());
    }
}
