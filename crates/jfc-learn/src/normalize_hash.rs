//! Normalize text and produce a stable SHA256 hash for deduplication.
//!
//! Normalization: strip whitespace runs → single space, lowercase, trim punctuation.

use sha2::{Digest, Sha256};

/// Normalize text (strip whitespace runs→single space, lowercase, trim punctuation)
/// then SHA256. Returns hex string.
pub fn normalize_and_hash(text: &str) -> String {
    let normalized = normalize(text);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let result = hasher.finalize();
    hex_encode(&result)
}

/// Normalize text for comparison: collapse whitespace, lowercase, strip leading/trailing punctuation.
pub fn normalize(text: &str) -> String {
    // Lowercase
    let lower = text.to_lowercase();

    // Collapse whitespace runs to single space
    let mut result = String::with_capacity(lower.len());
    let mut prev_ws = false;
    for ch in lower.chars() {
        if ch.is_whitespace() {
            if !prev_ws && !result.is_empty() {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            prev_ws = false;
            result.push(ch);
        }
    }

    // Trim trailing space
    let trimmed = result.trim().to_string();

    // Strip leading and trailing punctuation
    trimmed
        .trim_start_matches(|c: char| c.is_ascii_punctuation())
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hash_stable_normal() {
        let h1 = normalize_and_hash("The project uses serde for serialization.");
        let h2 = normalize_and_hash("The project uses serde for serialization.");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA256 hex = 64 chars
    }

    #[test]
    fn normalize_hash_strips_whitespace_normal() {
        let h1 = normalize_and_hash("The project uses   serde\n\tfor   serialization.");
        let h2 = normalize_and_hash("the project uses serde for serialization");
        assert_eq!(h1, h2);
    }
}
