use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use tracing::{debug, warn};

use crate::error::{AuditError, Result};
use crate::types::{Finding, SuppressReason};

/// Filter criteria for querying findings.
#[derive(Debug, Default, Clone)]
pub struct FindingFilter {
    pub kind: Option<crate::types::FindingKind>,
    pub severity_floor: Option<crate::types::Severity>,
    pub suppressed: Option<bool>,
    pub file_prefix: Option<String>,
}

/// Append-only persistent store for audit findings.
///
/// Backed by a JSONL file at `.jfc/audit/findings.jsonl`.
/// Uses flock on `findings.lock` for cross-process safety.
pub struct FindingStore {
    root: PathBuf,
    findings_path: PathBuf,
    lock_path: PathBuf,
    /// In-memory index keyed by finding id.
    index: HashMap<String, Finding>,
}

impl FindingStore {
    /// Open (or create) the finding store for a project root.
    pub fn open_project(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let audit_dir = root.join(".jfc").join("audit");
        fs::create_dir_all(&audit_dir).map_err(|e| AuditError::Io {
            source: e,
            context: format!("creating audit dir at {}", audit_dir.display()),
        })?;

        let findings_path = audit_dir.join("findings.jsonl");
        let lock_path = audit_dir.join("findings.lock");

        // Ensure files exist
        if !findings_path.exists() {
            File::create(&findings_path).map_err(|e| AuditError::Io {
                source: e,
                context: "creating findings.jsonl".to_string(),
            })?;
        }

        let mut store = Self {
            root,
            findings_path,
            lock_path,
            index: HashMap::new(),
        };

        store.reload()?;
        Ok(store)
    }

    /// Reload the in-memory index from the JSONL file.
    fn reload(&mut self) -> Result<()> {
        self.index.clear();
        let file = File::open(&self.findings_path).map_err(|e| AuditError::Io {
            source: e,
            context: "opening findings.jsonl for reload".to_string(),
        })?;

        let reader = BufReader::new(file);
        for (line_num, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| AuditError::Io {
                source: e,
                context: format!("reading line {line_num}"),
            })?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Finding>(trimmed) {
                Ok(finding) => {
                    self.index.insert(finding.id.clone(), finding);
                }
                Err(e) => {
                    warn!(line_num, error = %e, "skipping corrupt finding line");
                }
            }
        }
        debug!(count = self.index.len(), "loaded findings from store");
        Ok(())
    }

    /// Append a finding. Dedup by id — collisions update last_seen_revision.
    pub fn append(&mut self, mut finding: Finding) -> Result<()> {
        // Dedup: if we already have this id, update last_seen and merge verdicts
        if let Some(existing) = self.index.get_mut(&finding.id) {
            existing.last_seen_revision = finding.last_seen_revision;
            // Merge new validator verdicts
            for v in finding.validator_verdicts.drain(..) {
                if !existing
                    .validator_verdicts
                    .iter()
                    .any(|ev| ev.validator_id == v.validator_id && ev.timestamp == v.timestamp)
                {
                    existing.validator_verdicts.push(v);
                }
            }
            // Update poc_status if it advanced
            existing.poc_status = finding.poc_status;
            // Rewrite the full file (simple for v1)
            self.flush()?;
            return Ok(());
        }

        // New finding — append to file with lock
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| AuditError::Io {
                source: e,
                context: "opening lock file".to_string(),
            })?;

        lock_file.lock_exclusive().map_err(|e| AuditError::Io {
            source: e,
            context: "acquiring exclusive lock".to_string(),
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.findings_path)
            .map_err(|e| AuditError::Io {
                source: e,
                context: "opening findings.jsonl for append".to_string(),
            })?;

        let json = serde_json::to_string(&finding)?;
        writeln!(file, "{json}").map_err(|e| AuditError::Io {
            source: e,
            context: "writing finding line".to_string(),
        })?;

        lock_file.unlock().map_err(|e| AuditError::Io {
            source: e,
            context: "releasing lock".to_string(),
        })?;

        self.index.insert(finding.id.clone(), finding);
        Ok(())
    }

    /// Query findings matching the filter.
    pub fn query(&self, filter: &FindingFilter) -> Vec<&Finding> {
        self.index
            .values()
            .filter(|f| {
                if let Some(kind) = &filter.kind
                    && f.kind != *kind
                {
                    return false;
                }
                if let Some(floor) = &filter.severity_floor
                    && f.severity < *floor
                {
                    return false;
                }
                if let Some(suppressed) = filter.suppressed
                    && suppressed != f.suppressed.is_some()
                {
                    return false;
                }
                if let Some(prefix) = &filter.file_prefix
                    && !f.location.file.starts_with(prefix.as_str())
                {
                    return false;
                }
                true
            })
            .collect()
    }

    /// Mark a finding as suppressed.
    pub fn mark_suppressed(&mut self, id: &str, reason: SuppressReason) -> Result<()> {
        if let Some(finding) = self.index.get_mut(id) {
            finding.suppressed = Some(reason);
            self.flush()?;
            Ok(())
        } else {
            Err(AuditError::Internal {
                message: format!("finding {id} not found in store"),
            })
        }
    }

    /// Get a finding by id.
    pub fn get(&self, id: &str) -> Option<&Finding> {
        self.index.get(id)
    }

    /// Total count of findings in the store.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Flush the entire index to the JSONL file (rewrite).
    fn flush(&self) -> Result<()> {
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| AuditError::Io {
                source: e,
                context: "opening lock file for flush".to_string(),
            })?;

        lock_file.lock_exclusive().map_err(|e| AuditError::Io {
            source: e,
            context: "acquiring lock for flush".to_string(),
        })?;

        // Atomic write: serialize to a sibling tmp file, then rename over the
        // target. A crash mid-flush leaves the previous findings.jsonl intact
        // instead of a truncated/half-written file.
        let tmp_path = self.findings_path.with_extension("jsonl.tmp");
        let mut file = File::create(&tmp_path).map_err(|e| AuditError::Io {
            source: e,
            context: "creating findings.jsonl.tmp for flush".to_string(),
        })?;

        for finding in self.index.values() {
            let json = serde_json::to_string(finding)?;
            writeln!(file, "{json}").map_err(|e| AuditError::Io {
                source: e,
                context: "writing finding during flush".to_string(),
            })?;
        }

        file.flush().map_err(|e| AuditError::Io {
            source: e,
            context: "flushing findings.jsonl.tmp".to_string(),
        })?;
        drop(file);
        std::fs::rename(&tmp_path, &self.findings_path).map_err(|e| AuditError::Io {
            source: e,
            context: "atomically replacing findings.jsonl".to_string(),
        })?;

        lock_file.unlock().map_err(|e| AuditError::Io {
            source: e,
            context: "releasing lock after flush".to_string(),
        })?;

        Ok(())
    }

    /// Project root path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use tempfile::TempDir;

    fn sample_finding(id_suffix: &str) -> Finding {
        let location = SourceSpan {
            file: "src/main.rs".to_string(),
            start_line: 10,
            end_line: 15,
        };
        let id = Finding::compute_id(FindingKind::TaintedSink, &location, id_suffix);
        Finding {
            id,
            severity: Severity::High,
            kind: FindingKind::TaintedSink,
            location,
            granularity: Granularity::Function,
            reachability_path: vec!["fn:main".to_string()],
            taint_chain: None,
            preconditions: vec![],
            validator_verdicts: vec![],
            poc_status: PocStatus::NotAttempted,
            first_seen_revision: 1,
            last_seen_revision: 1,
            suppressed: None,
        }
    }

    #[test]
    fn store_write_and_reload_normal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Write two findings
        {
            let mut store = FindingStore::open_project(root).unwrap();
            store.append(sample_finding("entry1")).unwrap();
            store.append(sample_finding("entry2")).unwrap();
            assert_eq!(store.len(), 2);
        }

        // Reload from disk
        {
            let store = FindingStore::open_project(root).unwrap();
            assert_eq!(store.len(), 2);
        }
    }

    #[test]
    fn store_concurrent_append_robust() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Simulate concurrent appends by opening two stores
        let mut store1 = FindingStore::open_project(root).unwrap();
        let mut store2 = FindingStore::open_project(root).unwrap();

        store1.append(sample_finding("a")).unwrap();
        store2.append(sample_finding("b")).unwrap();

        // Reload — both should be present (last writer wins for overlap, but these are distinct)
        let store3 = FindingStore::open_project(root).unwrap();
        assert_eq!(store3.len(), 2);
    }

    #[test]
    fn store_dedup_updates_last_seen_normal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let mut store = FindingStore::open_project(root).unwrap();
        let mut f = sample_finding("x");
        f.last_seen_revision = 1;
        store.append(f.clone()).unwrap();

        // Append same id with updated revision
        let mut f2 = f.clone();
        f2.last_seen_revision = 5;
        store.append(f2).unwrap();

        assert_eq!(store.len(), 1);
        let stored = store.get(&f.id).unwrap();
        assert_eq!(stored.last_seen_revision, 5);
    }
}
