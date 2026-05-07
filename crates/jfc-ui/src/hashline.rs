//! Content-hash-anchored line addressing for reliable edits.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use sha2::{Digest, Sha256};

/// First 8 hex chars of the SHA-256 digest of trimmed line content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineId([u8; 4]);

impl LineId {
    pub fn compute(line: &str) -> Self {
        let digest = Sha256::digest(line.trim().as_bytes());
        Self([digest[0], digest[1], digest[2], digest[3]])
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

pub struct FileIndex {
    /// Maps LineId → Vec of 0-based line indices with that hash
    index: HashMap<LineId, Vec<usize>>,
    /// Modification time when index was built
    mtime: Option<SystemTime>,
}

impl FileIndex {
    pub fn build(content: &str) -> Self {
        let mut index: HashMap<LineId, Vec<usize>> = HashMap::new();

        for (line_number, line) in content.lines().enumerate() {
            index
                .entry(LineId::compute(line))
                .or_default()
                .push(line_number);
        }

        Self { index, mtime: None }
    }

    pub fn resolve(&self, line_id: &LineId, hint_line: usize) -> Option<usize> {
        let lines = self.index.get(line_id)?;

        if lines.contains(&hint_line) {
            return Some(hint_line);
        }

        lines
            .iter()
            .copied()
            .min_by_key(|line| line.abs_diff(hint_line))
    }

    pub fn mtime(&self) -> Option<SystemTime> {
        self.mtime
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub line: usize,
    pub confidence: f32,
    pub method: ResolutionMethod,
}

#[derive(Debug, Clone)]
pub struct EditResolution {
    /// 0-based line where old_string starts
    pub start_line: usize,
    /// 0-based line where old_string ends (inclusive)
    pub end_line: usize,
    /// Confidence of the resolution
    pub confidence: f32,
    /// Method used
    pub method: ResolutionMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMethod {
    ExactHash,
    Fuzzy,
    Failed,
}

/// High-level edit resolution: given file content and the old_string the model wants to replace,
/// attempt to locate it using Hashline when exact string match is ambiguous or fails.
///
/// Returns `Some((start_line, end_line))` (0-based, inclusive) if resolved, None if failed.
pub fn try_resolve_edit_target(
    content: &str,
    old_string: &str,
    hint_line: Option<usize>,
) -> Option<EditResolution> {
    let first_line = old_string.lines().next()?;
    if old_string.is_empty() || first_line.is_empty() {
        return None;
    }

    let occurrences: Vec<usize> = content
        .match_indices(old_string)
        .map(|(offset, _)| offset)
        .collect();
    let line_span = old_string.lines().count().saturating_sub(1);

    match occurrences.as_slice() {
        [offset] => {
            let start_line = line_number_at_offset(content, *offset);
            Some(EditResolution {
                start_line,
                end_line: start_line + line_span,
                confidence: 1.0,
                method: ResolutionMethod::ExactHash,
            })
        }
        [] => {
            let hint_line = hint_line.unwrap_or(0);
            let resolution = resolve_fuzzy(content, first_line, hint_line);

            (resolution.method != ResolutionMethod::Failed && resolution.confidence >= 0.9)
                .then_some(EditResolution {
                    start_line: resolution.line,
                    end_line: resolution.line + line_span,
                    confidence: resolution.confidence,
                    method: resolution.method,
                })
        }
        _ => {
            let hint_line = hint_line?;
            let line_id = LineId::compute(first_line);
            let index = FileIndex::build(content);
            let resolved_line = index.resolve(&line_id, hint_line)?;
            let occurrence_start_lines: Vec<usize> = occurrences
                .iter()
                .map(|offset| line_number_at_offset(content, *offset))
                .collect();
            let start_line = occurrence_start_lines
                .iter()
                .copied()
                .find(|line| *line == resolved_line)
                .or_else(|| {
                    occurrence_start_lines
                        .iter()
                        .copied()
                        .min_by_key(|line| line.abs_diff(resolved_line))
                })?;

            Some(EditResolution {
                start_line,
                end_line: start_line + line_span,
                confidence: 1.0,
                method: ResolutionMethod::ExactHash,
            })
        }
    }
}

fn line_number_at_offset(content: &str, offset: usize) -> usize {
    content[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
}

pub fn resolve_fuzzy(content: &str, original_line: &str, hint_line: usize) -> Resolution {
    let expected_hash = LineId::compute(original_line);
    let index = FileIndex::build(content);

    if let Some(line) = index.resolve(&expected_hash, hint_line) {
        return Resolution {
            line,
            confidence: 1.0,
            method: ResolutionMethod::ExactHash,
        };
    }

    content
        .lines()
        .enumerate()
        .map(|(line, candidate)| (line, levenshtein_similarity(original_line, candidate)))
        .filter(|(_, confidence)| *confidence >= 0.8)
        .max_by(
            |(left_line, left_confidence), (right_line, right_confidence)| {
                left_confidence.total_cmp(right_confidence).then_with(|| {
                    right_line
                        .abs_diff(hint_line)
                        .cmp(&left_line.abs_diff(hint_line))
                })
            },
        )
        .map_or(
            Resolution {
                line: hint_line,
                confidence: 0.0,
                method: ResolutionMethod::Failed,
            },
            |(line, confidence)| Resolution {
                line,
                confidence,
                method: ResolutionMethod::Fuzzy,
            },
        )
}

fn levenshtein_similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let max_len = a.len().max(b.len());

    if max_len == 0 {
        return 1.0;
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];

    for (i, a_char) in a.iter().enumerate() {
        current[0] = i + 1;

        for (j, b_char) in b.iter().enumerate() {
            let substitution_cost = usize::from(a_char != b_char);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution_cost);
        }

        std::mem::swap(&mut previous, &mut current);
    }

    1.0 - (previous[b.len()] as f32 / max_len as f32)
}

pub fn verify_before_apply(content: &str, resolved_line: usize, expected_hash: &LineId) -> bool {
    content
        .lines()
        .nth(resolved_line)
        .is_some_and(|line| LineId::compute(line) == *expected_hash)
}

#[derive(Default)]
pub struct HashlineCache {
    cache: HashMap<PathBuf, (SystemTime, FileIndex)>,
}

impl HashlineCache {
    pub fn get_or_build(&mut self, path: &Path) -> &FileIndex {
        let mtime = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let should_rebuild = self
            .cache
            .get(path)
            .is_none_or(|(cached_mtime, _)| *cached_mtime != mtime);

        if should_rebuild {
            let content = fs::read_to_string(path).unwrap_or_default();
            let mut index = FileIndex::build(&content);
            index.mtime = Some(mtime);
            self.cache.insert(path.to_path_buf(), (mtime, index));
        }

        &self
            .cache
            .get(path)
            .expect("cache entry was just inserted")
            .1
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn test_line_id_deterministic() {
        let first = LineId::compute("  target line  ");
        let second = LineId::compute("target line");

        assert_eq!(first, second);
        assert_eq!(first.to_hex().len(), 8);
    }

    #[test]
    fn test_resolve_exact_hint() {
        let index = FileIndex::build("alpha\nbeta\ngamma");
        let line_id = LineId::compute("beta");

        assert_eq!(index.resolve(&line_id, 1), Some(1));
    }

    #[test]
    fn test_resolve_after_insertion() {
        let original_line_id = LineId::compute("target");
        let mutated = FileIndex::build("inserted 1\ninserted 2\ninserted 3\nalpha\ntarget\nomega");

        assert_eq!(mutated.resolve(&original_line_id, 1), Some(4));
    }

    #[test]
    fn test_resolve_duplicate_lines() {
        let index = FileIndex::build("same\nother\nsame\nother\nsame");
        let line_id = LineId::compute("same");

        assert_eq!(index.resolve(&line_id, 3), Some(2));
    }

    #[test]
    fn test_cache_invalidation() {
        let file = NamedTempFile::new().expect("create temp file");
        fs::write(file.path(), "alpha\nbeta\n").expect("write initial content");

        let mut cache = HashlineCache::default();
        let first_mtime = cache
            .get_or_build(file.path())
            .mtime()
            .expect("initial mtime");
        assert_eq!(
            cache
                .get_or_build(file.path())
                .resolve(&LineId::compute("beta"), 1),
            Some(1)
        );

        thread::sleep(Duration::from_millis(10));
        fs::write(file.path(), "inserted\nalpha\nbeta\n").expect("write changed content");

        let second = cache.get_or_build(file.path());

        assert_ne!(second.mtime().expect("updated mtime"), first_mtime);
        assert_eq!(second.resolve(&LineId::compute("beta"), 1), Some(2));
    }

    #[test]
    fn test_fuzzy_finds_minor_edit() {
        let resolution = resolve_fuzzy("fn foo(x: u128)\nfn bar()", "fn foo(x: u64)", 0);

        assert_eq!(resolution.line, 0);
        assert_eq!(resolution.method, ResolutionMethod::Fuzzy);
        assert!(resolution.confidence >= 0.8);
    }

    #[test]
    fn test_fuzzy_rejects_dissimilar() {
        let resolution = resolve_fuzzy("struct Widget;\nimpl Widget {}", "fn foo(x: u64)", 0);

        assert_eq!(resolution.method, ResolutionMethod::Failed);
        assert_eq!(resolution.confidence, 0.0);
    }

    #[test]
    fn test_verify_before_apply_confirms() {
        let content = "alpha\nbeta\ngamma";
        let expected_hash = LineId::compute("beta");

        assert!(verify_before_apply(content, 1, &expected_hash));
        assert!(!verify_before_apply(content, 0, &expected_hash));
    }

    #[test]
    fn test_edit_resolution_unique_match() {
        let resolution = try_resolve_edit_target("alpha\ntarget\ngamma", "target", None)
            .expect("unique match resolves");

        assert_eq!(resolution.start_line, 1);
        assert_eq!(resolution.end_line, 1);
        assert_eq!(resolution.confidence, 1.0);
        assert_eq!(resolution.method, ResolutionMethod::ExactHash);
    }

    #[test]
    fn test_edit_resolution_disambiguates_duplicates() {
        let content = "same\nother\nsame\nother\nsame";
        let resolution = try_resolve_edit_target(content, "same", Some(3))
            .expect("duplicate match resolves with hint");

        assert_eq!(resolution.start_line, 2);
        assert_eq!(resolution.end_line, 2);
        assert_eq!(resolution.confidence, 1.0);
        assert_eq!(resolution.method, ResolutionMethod::ExactHash);
    }

    #[test]
    fn test_edit_resolution_fuzzy_after_drift() {
        let content = "fn foo(x: u64)\nfn bar()";
        let resolution = try_resolve_edit_target(content, "fn foo(x: u65)", Some(0))
            .expect("fuzzy match resolves after drift");

        assert_eq!(resolution.start_line, 0);
        assert_eq!(resolution.end_line, 0);
        assert!(resolution.confidence >= 0.9);
        assert_eq!(resolution.method, ResolutionMethod::Fuzzy);
    }

    #[test]
    fn test_edit_resolution_fails_gracefully() {
        let resolution =
            try_resolve_edit_target("struct Widget;\nimpl Widget {}", "fn foo(x: u64)", Some(0));

        assert!(resolution.is_none());
    }
}
