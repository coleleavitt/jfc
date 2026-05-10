//! Speculation engine — pre-runs tool calls in an isolated filesystem
//! overlay before the user approves them.
//!
//! Mirrors v132's `tengu_speculation` flow: while a Write/Edit/MultiEdit
//! tool is awaiting approval we redirect its writes into a per-tool
//! overlay at `/tmp/jfc-speculation/$pid/$tool_id/`. On approve we copy
//! the overlay back to the real filesystem (`commit`); on reject we
//! `rm -rf` the overlay (`abort`); on interrupt we hold the overlay in
//! `Paused` until the user makes a final decision.
//!
//! The module is intentionally synchronous — local filesystem ops are
//! cheap and putting a tokio task between the tool runner and the
//! overlay just adds latency.
//!
//! Default-off: gate all real-tool integration on
//! `Config.experimental.speculation_enabled`.

pub mod overlay;
pub mod safety;

use std::io;
use std::path::{Path, PathBuf};

pub use overlay::{Overlay, default_base};
pub use safety::SafetyError;

/// State machine for a single tool's speculative execution.
///
/// ```text
///                    queue
///   Idle  ────────────────────────►  Active
///                                      │  tool finished
///                                      ▼
///   Active ─── interrupt ──► Paused ◄──┤
///                              │       │
///                              └─►  Pending
///                                      │  approve / reject
///                                      ▼
///                                  Committed / Aborted
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeculationState {
    /// No speculation in flight.
    Idle,
    /// Overlay created; tool body is currently writing into it.
    Active,
    /// Tool finished writing into the overlay; awaiting user decision.
    Pending,
    /// Tool was interrupted while running. Overlay is preserved until
    /// the user approves/rejects.
    Paused,
    /// Overlay copied back to the real filesystem.
    Committed,
    /// Overlay discarded.
    Aborted,
}

/// One speculative execution context — owns the overlay for the
/// lifetime of a single tool call.
#[derive(Debug)]
pub struct SpeculationSession {
    tool_id: String,
    state: SpeculationState,
    overlay: Option<Overlay>,
}

impl SpeculationSession {
    /// Begin a new speculation: allocate the overlay under
    /// [`default_base()`] and transition `Idle → Active`.
    pub fn start(tool_id: impl Into<String>, cwd: &Path) -> io::Result<Self> {
        Self::start_in(tool_id, cwd, &default_base())
    }

    /// Like [`start`](Self::start) but with a caller-supplied overlay
    /// base — used by tests for hermetic isolation.
    pub fn start_in(tool_id: impl Into<String>, cwd: &Path, base: &Path) -> io::Result<Self> {
        let tool_id = tool_id.into();
        let overlay = Overlay::create(base, cwd, &tool_id)?;
        Ok(Self {
            tool_id,
            state: SpeculationState::Active,
            overlay: Some(overlay),
        })
    }

    /// Tool id this session is bound to.
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// Current state of the speculation session.
    pub fn state(&self) -> SpeculationState {
        self.state
    }

    /// Borrow the overlay (e.g. to redirect a Write).
    pub fn overlay_mut(&mut self) -> Option<&mut Overlay> {
        self.overlay.as_mut()
    }

    /// Borrow the overlay immutably.
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Mark the speculative tool body as finished: `Active → Pending`.
    /// No-op if already pending. Errors if called from a terminal state.
    pub fn mark_finished(&mut self) -> io::Result<()> {
        match self.state {
            SpeculationState::Active | SpeculationState::Paused => {
                self.state = SpeculationState::Pending;
                Ok(())
            }
            SpeculationState::Pending => Ok(()),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot mark_finished from {other:?}"),
            )),
        }
    }

    /// Pause the speculation (interrupt while running). Overlay is kept.
    pub fn pause(&mut self) -> io::Result<()> {
        match self.state {
            SpeculationState::Active => {
                self.state = SpeculationState::Paused;
                Ok(())
            }
            SpeculationState::Paused => Ok(()),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot pause from {other:?}"),
            )),
        }
    }

    /// User approved the tool: copy the overlay back to the real fs.
    /// Valid from `Active` (zero-latency single-call), `Pending`, or
    /// `Paused`.
    pub fn commit(&mut self) -> io::Result<Vec<PathBuf>> {
        match self.state {
            SpeculationState::Active | SpeculationState::Pending | SpeculationState::Paused => {
                let mut overlay = self
                    .overlay
                    .take()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "overlay missing"))?;
                let updated = overlay.commit_changes()?;
                self.state = SpeculationState::Committed;
                Ok(updated)
            }
            SpeculationState::Committed => Ok(Vec::new()),
            SpeculationState::Aborted | SpeculationState::Idle => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot commit from {:?}", self.state),
            )),
        }
    }

    /// User rejected the tool: discard the overlay.
    pub fn abort(&mut self) -> io::Result<()> {
        match self.state {
            SpeculationState::Active | SpeculationState::Pending | SpeculationState::Paused => {
                if let Some(mut ov) = self.overlay.take() {
                    ov.abort()?;
                }
                self.state = SpeculationState::Aborted;
                Ok(())
            }
            SpeculationState::Aborted => Ok(()),
            SpeculationState::Committed | SpeculationState::Idle => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot abort from {:?}", self.state),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn session(base: &TempDir, cwd: &TempDir, id: &str) -> SpeculationSession {
        SpeculationSession::start_in(id, cwd.path(), base.path()).expect("session start")
    }

    #[test]
    fn start_transitions_idle_to_active_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let s = session(&base, &cwd, "tool-1");
        assert_eq!(s.state(), SpeculationState::Active);
        assert_eq!(s.tool_id(), "tool-1");
        assert!(s.overlay().is_some());
    }

    #[test]
    fn mark_finished_moves_to_pending_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        s.mark_finished().unwrap();
        assert_eq!(s.state(), SpeculationState::Pending);
    }

    #[test]
    fn pause_then_finish_then_commit_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        let real = cwd.path().join("a.txt");
        s.overlay_mut().unwrap().write_file(&real, b"x").unwrap();
        s.pause().unwrap();
        assert_eq!(s.state(), SpeculationState::Paused);
        s.mark_finished().unwrap();
        assert_eq!(s.state(), SpeculationState::Pending);
        let updated = s.commit().unwrap();
        assert_eq!(updated, vec![real.clone()]);
        assert_eq!(std::fs::read(&real).unwrap(), b"x");
        assert_eq!(s.state(), SpeculationState::Committed);
    }

    #[test]
    fn commit_from_active_copies_overlay_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        let real = cwd.path().join("greet.txt");
        s.overlay_mut()
            .unwrap()
            .write_file(&real, b"hello")
            .unwrap();
        let updated = s.commit().unwrap();
        assert_eq!(updated, vec![real.clone()]);
        assert_eq!(std::fs::read(&real).unwrap(), b"hello");
        assert_eq!(s.state(), SpeculationState::Committed);
    }

    #[test]
    fn abort_does_not_touch_real_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        let real = cwd.path().join("ghost.txt");
        s.overlay_mut()
            .unwrap()
            .write_file(&real, b"discard")
            .unwrap();
        s.abort().unwrap();
        assert!(!real.exists());
        assert_eq!(s.state(), SpeculationState::Aborted);
    }

    #[test]
    fn commit_after_abort_errors_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        s.abort().unwrap();
        let err = s.commit().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn double_abort_is_idempotent_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        s.abort().unwrap();
        s.abort().expect("second abort is no-op");
    }

    #[test]
    fn pause_from_pending_errors_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        s.mark_finished().unwrap();
        let err = s.pause().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn redirect_maps_under_cwd_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = session(&base, &cwd, "t");
        let mapped = s
            .overlay_mut()
            .unwrap()
            .redirect(&cwd.path().join("foo.txt"))
            .unwrap();
        assert!(mapped.starts_with(s.overlay().unwrap().overlay_root()));
        assert!(mapped.ends_with("foo.txt"));
    }
}
