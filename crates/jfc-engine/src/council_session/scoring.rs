//! Fuzzy choice grouping for the verdict flow.
//!
//! Ports the RoundTable web client's `normalizeChoice` / `levenshtein` /
//! `choicesMatch` / `groupChoices`: final verdict CHOICEs are clustered so
//! "Forty-two", "42", and "the answer is 42" count as one position, letting the
//! session detect unanimity (skip the ballot) or run a vote over genuinely
//! distinct positions.

/// Normalize a choice for comparison: lowercase, drop punctuation and a small
/// stoplist, collapse whitespace.
pub fn normalize_choice(s: &str) -> String {
    const STOP: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "of", "in", "on", "at", "to",
        "for", "and", "or", "but", "with", "that", "this", "it", "i", "we", "they",
    ];
    let lowered: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    lowered
        .split_whitespace()
        .filter(|w| !STOP.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Classic Levenshtein edit distance (iterative, two-row).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Whether two choices should cluster: exact, token-set, substring, or a
/// length-scaled Levenshtein tolerance (typos / minor word variants).
pub fn choices_match(a: &str, b: &str) -> bool {
    let na = normalize_choice(a);
    let nb = normalize_choice(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    if na == nb {
        return true;
    }
    let ta: std::collections::BTreeSet<&str> = na.split(' ').collect();
    let tb: std::collections::BTreeSet<&str> = nb.split(' ').collect();
    if ta == tb {
        return true;
    }
    if na.len() >= 3 && nb.len() >= 3 && (na.contains(&nb) || nb.contains(&na)) {
        return true;
    }
    let threshold = 2.max((na.len().min(nb.len()) as f64 * 0.18) as usize);
    levenshtein(&na, &nb) <= threshold
}

/// A cluster of seats that converged on one (representative) choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceGroup {
    /// The first choice text seen for this cluster (its representative label).
    pub choice: String,
    /// Seat ids whose choices clustered here.
    pub seat_ids: Vec<String>,
}

/// Cluster `(seat_id, choice)` pairs by [`choices_match`]. Order-stable: the
/// first choice of each cluster is its representative.
pub fn group_choices<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<ChoiceGroup> {
    let mut groups: Vec<ChoiceGroup> = Vec::new();
    for (seat_id, choice) in entries {
        if let Some(g) = groups.iter_mut().find(|g| choices_match(&g.choice, choice)) {
            g.seat_ids.push(seat_id.to_owned());
        } else {
            groups.push(ChoiceGroup {
                choice: choice.to_owned(),
                seat_ids: vec![seat_id.to_owned()],
            });
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_drops_stopwords_and_punct_normal() {
        assert_eq!(normalize_choice("The answer is 42!"), "answer 42");
    }

    #[test]
    fn choices_match_variants_robust() {
        assert!(choices_match("Forty-two", "forty two")); // punctuation-insensitive
        assert!(choices_match("Use Redis", "redis")); // substring after normalize (>=3 chars)
        assert!(choices_match("go with Redis", "Redis go")); // token-set, order-insensitive
        assert!(choices_match("Postgres", "Postgress")); // levenshtein typo tolerance
        assert!(!choices_match("yes", "no"));
    }

    #[test]
    fn group_choices_clusters_normal() {
        let groups = group_choices([("a", "Redis"), ("b", "redis"), ("c", "Postgres")]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].seat_ids, vec!["a", "b"]);
        assert_eq!(groups[1].seat_ids, vec!["c"]);
    }

    #[test]
    fn levenshtein_basic_normal() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }
}
