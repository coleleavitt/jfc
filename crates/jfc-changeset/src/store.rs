use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use tracing::{debug, warn};

use crate::error::{ChangeSetError, Result};
use crate::state::ChangeState;
use crate::types::AgentChangeSet;

/// Filter criteria for querying change-sets.
#[derive(Debug, Default, Clone)]
pub struct ChangeFilter {
    pub state: Option<ChangeState>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    /// Only change-sets whose `ledger_refs` contains this id.
    pub ledger_ref: Option<String>,
}

impl ChangeFilter {
    fn matches(&self, cs: &AgentChangeSet) -> bool {
        if let Some(state) = self.state
            && cs.state != state
        {
            return false;
        }
        if let Some(task) = &self.task_id
            && cs.task_id.as_deref() != Some(task.as_str())
        {
            return false;
        }
        if let Some(agent) = &self.agent_id
            && cs.agent_id.as_deref() != Some(agent.as_str())
        {
            return false;
        }
        if let Some(reff) = &self.ledger_ref
            && !cs.ledger_refs.iter().any(|r| r == reff)
        {
            return false;
        }
        true
    }
}

/// Append-only persistent store for agent change-sets.
///
/// Backed by a JSONL file at `.jfc/changes/changes.jsonl`, with an in-memory
/// index keyed by change-set id. Mutations rewrite the file under an exclusive
/// flock on `changes.lock` for cross-process safety — the same discipline as
/// `jfc-audit`'s `FindingStore`. JSONL keeps each record one self-describing
/// line so a partially-written file degrades to "skip the bad line", not "lose
/// the whole history".
pub struct ChangeStore {
    changes_path: PathBuf,
    lock_path: PathBuf,
    index: HashMap<String, AgentChangeSet>,
}

impl ChangeStore {
    /// Open (or create) the change store under a project root.
    pub fn open_project(root: impl AsRef<Path>) -> Result<Self> {
        let _linkscope_open = linkscope::phase("changeset.store.open_project");
        let dir = root.as_ref().join(".jfc").join("changes");
        fs::create_dir_all(&dir)
            .map_err(|e| ChangeSetError::io(e, format!("creating {}", dir.display())))?;

        let changes_path = dir.join("changes.jsonl");
        let lock_path = dir.join("changes.lock");
        if !changes_path.exists() {
            File::create(&changes_path)
                .map_err(|e| ChangeSetError::io(e, "creating changes.jsonl"))?;
        }

        let mut store = Self {
            changes_path,
            lock_path,
            index: HashMap::new(),
        };
        store.reload()?;
        linkscope::record_items(
            "changeset.store.opened",
            usize_to_u64_saturating(store.len()),
        );
        Ok(store)
    }

    /// Rebuild the in-memory index from the JSONL file. A record that fails to
    /// parse is skipped with a warning rather than aborting the load — last
    /// write wins on duplicate ids (the file is replayed in order).
    fn reload(&mut self) -> Result<()> {
        let _linkscope_reload = linkscope::phase("changeset.store.reload");
        self.index.clear();
        let file = File::open(&self.changes_path)
            .map_err(|e| ChangeSetError::io(e, "opening changes.jsonl for reload"))?;
        for (n, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|e| ChangeSetError::io(e, format!("reading line {n}")))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<AgentChangeSet>(trimmed) {
                Ok(cs) => {
                    self.index.insert(cs.id.clone(), cs);
                }
                Err(e) => {
                    linkscope::record_items("changeset.store.corrupt_line", 1);
                    warn!(line = n, error = %e, "skipping corrupt change-set line");
                }
            }
        }
        linkscope::record_items(
            "changeset.store.loaded",
            usize_to_u64_saturating(self.index.len()),
        );
        debug!(count = self.index.len(), "loaded change-sets from store");
        Ok(())
    }

    /// Insert a new change-set or replace an existing one by id, then persist.
    /// `AgentChangeSet` carries its own monotonic `updated_at_ms`, so an
    /// upsert is the natural write for every lifecycle transition.
    pub fn upsert(&mut self, cs: AgentChangeSet) -> Result<()> {
        let _linkscope_upsert = linkscope::phase("changeset.store.upsert");
        self.index.insert(cs.id.clone(), cs);
        self.flush()
    }

    /// Get a change-set by id.
    pub fn get(&self, id: &str) -> Option<&AgentChangeSet> {
        self.index.get(id)
    }

    /// All change-sets matching `filter`, newest-updated first.
    pub fn query(&self, filter: &ChangeFilter) -> Vec<&AgentChangeSet> {
        let _linkscope_query = linkscope::phase("changeset.store.query");
        let mut out: Vec<&AgentChangeSet> = self
            .index
            .values()
            .filter(|cs| filter.matches(cs))
            .collect();
        out.sort_by_key(|cs| std::cmp::Reverse(cs.updated_at_ms));
        linkscope::record_items(
            "changeset.store.query.rows",
            usize_to_u64_saturating(out.len()),
        );
        out
    }

    /// Number of change-sets in the store.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the store has no change-sets.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Rewrite the whole JSONL file from the in-memory index under an
    /// exclusive lock. Records are written in stable id order so the file is
    /// diff-friendly across runs.
    fn flush(&self) -> Result<()> {
        let _linkscope_flush = linkscope::phase("changeset.store.flush");
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| ChangeSetError::io(e, "opening change-store lock"))?;
        lock.lock_exclusive()
            .map_err(|e| ChangeSetError::io(e, "acquiring exclusive lock"))?;

        let result = self.write_all_locked();

        // Always attempt to unlock, even if the write failed. A failed unlock
        // is not fatal (the lock releases when `lock` drops / the process
        // exits) but it signals filesystem trouble worth a warning.
        if let Err(e) = FileExt::unlock(&lock) {
            warn!(error = %e, "failed to release change-store lock (will release on drop)");
        }
        result
    }

    fn write_all_locked(&self) -> Result<()> {
        let _linkscope_write = linkscope::phase("changeset.store.write_all");
        let tmp_path = self.changes_path.with_extension("jsonl.tmp");
        let mut tmp = File::create(&tmp_path)
            .map_err(|e| ChangeSetError::io(e, "creating changes.jsonl.tmp"))?;

        let mut ids: Vec<&String> = self.index.keys().collect();
        ids.sort();
        for id in ids {
            let cs = &self.index[id];
            let json = serde_json::to_string(cs)
                .map_err(|e| ChangeSetError::serde(e, "encoding change-set"))?;
            writeln!(tmp, "{json}")
                .map_err(|e| ChangeSetError::io(e, "writing change-set line"))?;
        }
        tmp.flush()
            .map_err(|e| ChangeSetError::io(e, "flushing changes.jsonl.tmp"))?;
        // fsync before the rename: flush() only reaches OS buffers, so a
        // crash after rename could otherwise publish a truncated file.
        tmp.sync_all()
            .map_err(|e| ChangeSetError::io(e, "syncing changes.jsonl.tmp"))?;
        drop(tmp);
        // Atomic replace so a crash mid-write never truncates the real file.
        fs::rename(&tmp_path, &self.changes_path)
            .map_err(|e| ChangeSetError::io(e, "atomically replacing changes.jsonl"))?;
        linkscope::record_items(
            "changeset.store.persisted",
            usize_to_u64_saturating(self.index.len()),
        );
        Ok(())
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChangedFile, TestRun};
    use tempfile::TempDir;

    fn opened(now: u64) -> AgentChangeSet {
        AgentChangeSet::open("basehead", "jfc/wt", "/tmp/wt", now)
    }

    // Normal: upsert then reopen round-trips the change-set from disk.
    #[test]
    fn upsert_then_reopen_round_trips_normal() {
        let dir = TempDir::new().unwrap();
        let id = {
            let mut store = ChangeStore::open_project(dir.path()).unwrap();
            let cs = opened(100);
            let id = cs.id.clone();
            store.upsert(cs).unwrap();
            assert_eq!(store.len(), 1);
            id
        };
        // Reopen from disk — a fresh process would see the same.
        let store = ChangeStore::open_project(dir.path()).unwrap();
        let cs = store.get(&id).expect("change-set persisted");
        assert_eq!(cs.state, ChangeState::Draft);
        assert_eq!(cs.branch, "jfc/wt");
    }

    // Normal: a full lifecycle persists and the final state is Applied.
    #[test]
    fn lifecycle_persists_final_state_normal() {
        let dir = TempDir::new().unwrap();
        let mut store = ChangeStore::open_project(dir.path()).unwrap();
        let mut cs = opened(100);
        let id = cs.id.clone();
        cs.mark_ready(
            vec![ChangedFile {
                path: "a.rs".into(),
                insertions: 1,
                deletions: 0,
            }],
            "1 file changed",
            101,
        )
        .unwrap();
        cs.record_test_run(
            TestRun {
                command: "cargo test".into(),
                exit_code: 0,
                duration_ms: 10,
                finished_at_ms: 102,
            },
            102,
        )
        .unwrap();
        cs.approve(
            crate::types::Approval::Human {
                user: "cole".into(),
                at_ms: 103,
            },
            103,
        )
        .unwrap();
        cs.transition_to(ChangeState::Applied, 104).unwrap();
        store.upsert(cs).unwrap();

        let reread = ChangeStore::open_project(dir.path()).unwrap();
        assert_eq!(reread.get(&id).unwrap().state, ChangeState::Applied);
    }

    // Robust: query filters by state and orders newest-updated first.
    #[test]
    fn query_filters_and_orders_robust() {
        let dir = TempDir::new().unwrap();
        let mut store = ChangeStore::open_project(dir.path()).unwrap();

        let draft = opened(100);
        let mut ready = AgentChangeSet::open("basehead", "jfc/other", "/tmp/other", 200);
        ready.mark_ready(Vec::new(), "noop", 201).unwrap();
        store.upsert(draft).unwrap();
        store.upsert(ready).unwrap();

        let drafts = store.query(&ChangeFilter {
            state: Some(ChangeState::Draft),
            ..Default::default()
        });
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].state, ChangeState::Draft);

        // No filter → both, newest-updated (the Ready one, updated at 201) first.
        let all = store.query(&ChangeFilter::default());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].updated_at_ms, 201);
    }

    // Robust: a corrupt JSONL line is skipped, not fatal, and good records
    // still load.
    #[test]
    fn corrupt_line_is_skipped_robust() {
        let dir = TempDir::new().unwrap();
        let id = {
            let mut store = ChangeStore::open_project(dir.path()).unwrap();
            let cs = opened(100);
            let id = cs.id.clone();
            store.upsert(cs).unwrap();
            id
        };
        // Append garbage directly to the file.
        let path = dir
            .path()
            .join(".jfc")
            .join("changes")
            .join("changes.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{not valid json").unwrap();
        drop(f);

        let store = ChangeStore::open_project(dir.path()).unwrap();
        assert_eq!(
            store.len(),
            1,
            "good record survives a corrupt sibling line"
        );
        assert!(store.get(&id).is_some());
    }

    // Robust: upserting the same id replaces rather than duplicates.
    #[test]
    fn upsert_same_id_replaces_robust() {
        let dir = TempDir::new().unwrap();
        let mut store = ChangeStore::open_project(dir.path()).unwrap();
        let mut cs = opened(100);
        let id = cs.id.clone();
        store.upsert(cs.clone()).unwrap();
        cs.mark_ready(Vec::new(), "advanced", 101).unwrap();
        store.upsert(cs).unwrap();

        assert_eq!(store.len(), 1, "same id must not duplicate");
        assert_eq!(store.get(&id).unwrap().state, ChangeState::Ready);
    }
}
