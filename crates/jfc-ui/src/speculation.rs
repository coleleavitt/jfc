//! Speculation engine — speculative execution while the user types.
//!
//! Mirrors Claude Code v2.1.142+'s `tengu_chomp_inflection` feature:
//! - While the user is composing their next message, predict likely tool calls
//! - Execute them speculatively in an overlay filesystem
//! - On submit: if speculation matches, accept instantly; otherwise discard
//!
//! The overlay ensures no side-effects leak to the real filesystem until accepted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Instant;

/// Active overlay session — when set, write-tools route their writes through
/// the overlay directory. Cleared on accept (writes flushed) or discard (rm -rf).
static ACTIVE_OVERLAY: RwLock<Option<OverlaySession>> = RwLock::new(None);

/// Overlay session state stored globally so tool dispatch can consult it
/// without threading state through every call site.
#[derive(Debug, Clone)]
pub struct OverlaySession {
    pub id: String,
    pub overlay_root: PathBuf,
    pub real_cwd: PathBuf,
}

/// Install an overlay session — subsequent Write/Edit calls route through
/// the overlay until `clear_overlay` is called.
pub fn install_overlay(session: OverlaySession) {
    if let Ok(mut g) = ACTIVE_OVERLAY.write() {
        *g = Some(session);
    }
}

/// Snapshot of the currently-active overlay session, if any.
pub fn active_overlay() -> Option<OverlaySession> {
    ACTIVE_OVERLAY.read().ok().and_then(|g| g.clone())
}

/// Clear the active overlay session. Caller owns deciding whether to
/// commit (flush overlay to real cwd) or discard (rm -rf the overlay).
pub fn clear_overlay() -> Option<OverlaySession> {
    ACTIVE_OVERLAY.write().ok().and_then(|mut g| g.take())
}

/// Translate a real-cwd-relative path to its overlay counterpart.
/// Returns `None` if no overlay is active or the path escapes the overlay.
pub fn overlay_path_for(real_path: &Path) -> Option<PathBuf> {
    let session = active_overlay()?;
    let abs = if real_path.is_absolute() {
        real_path.to_path_buf()
    } else {
        session.real_cwd.join(real_path)
    };
    let rel = abs.strip_prefix(&session.real_cwd).ok()?;
    Some(session.overlay_root.join(rel))
}

/// Accept a speculation: copy every file written under the overlay back
/// to the real cwd, then rm -rf the overlay.
pub fn accept_overlay() -> std::io::Result<usize> {
    let Some(session) = clear_overlay() else {
        return Ok(0);
    };
    let mut copied = 0;
    fn walk(src: &Path, dst: &Path, copied: &mut usize) -> std::io::Result<()> {
        if !src.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let target = dst.join(entry.file_name());
            if path.is_dir() {
                std::fs::create_dir_all(&target)?;
                walk(&path, &target, copied)?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&path, &target)?;
                *copied += 1;
            }
        }
        Ok(())
    }
    walk(&session.overlay_root, &session.real_cwd, &mut copied)?;
    let _ = std::fs::remove_dir_all(&session.overlay_root);
    Ok(copied)
}

/// Discard a speculation: rm -rf the overlay without copying anything.
pub fn discard_overlay() -> std::io::Result<()> {
    let Some(session) = clear_overlay() else {
        return Ok(());
    };
    std::fs::remove_dir_all(&session.overlay_root)
}

/// Tools allowed during speculation (read-only + scoped writes).
const ALLOWED_SPECULATION_TOOLS: &[&str] = &["Read", "Glob", "Grep", "ToolSearch", "WebFetch"];

/// Tools that can write during speculation (scoped to cwd).
const WRITE_SPECULATION_TOOLS: &[&str] = &["Write", "Edit"];

/// Status of a running speculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeculationStatus {
    /// Actively running speculative tool calls.
    Active,
    /// Paused due to a boundary condition (e.g. backgrounded shell).
    Paused { reason: String },
    /// Completed — waiting for user to submit.
    Idle,
}

/// A single speculation session.
#[derive(Debug)]
pub struct SpeculationSession {
    pub id: String,
    pub status: SpeculationStatus,
    pub started_at: Instant,
    pub suggestion_length: usize,
    pub tool_use_count: u32,
    pub overlay_dir: PathBuf,
    pub written_paths: HashSet<PathBuf>,
}

impl SpeculationSession {
    pub fn new(id: String, overlay_dir: PathBuf, suggestion_length: usize) -> Self {
        Self {
            id,
            status: SpeculationStatus::Active,
            started_at: Instant::now(),
            suggestion_length,
            tool_use_count: 0,
            overlay_dir,
            written_paths: HashSet::new(),
        }
    }

    /// Check if a tool is allowed during speculation.
    pub fn is_tool_allowed(tool_name: &str) -> bool {
        ALLOWED_SPECULATION_TOOLS.contains(&tool_name)
            || WRITE_SPECULATION_TOOLS.contains(&tool_name)
    }

    /// Check if a tool can write during speculation.
    pub fn is_write_tool(tool_name: &str) -> bool {
        WRITE_SPECULATION_TOOLS.contains(&tool_name)
    }

    /// Record a speculative file write.
    pub fn record_write(&mut self, path: PathBuf) {
        self.written_paths.insert(path);
        self.tool_use_count += 1;
    }

    /// Accept the speculation — move overlay writes to the real filesystem.
    pub fn accept(self) -> SpeculationResult {
        SpeculationResult {
            accepted: true,
            duration_ms: self.started_at.elapsed().as_millis() as u64,
            tool_calls: self.tool_use_count,
            files_written: self.written_paths.len() as u32,
        }
    }

    /// Discard the speculation — clean up overlay.
    pub fn discard(self) -> SpeculationResult {
        // In a real implementation, we'd clean up overlay_dir here
        SpeculationResult {
            accepted: false,
            duration_ms: self.started_at.elapsed().as_millis() as u64,
            tool_calls: self.tool_use_count,
            files_written: 0,
        }
    }

    /// Pause speculation due to a boundary condition.
    pub fn pause(&mut self, reason: String) {
        self.status = SpeculationStatus::Paused { reason };
    }
}

/// Result of a completed speculation (accepted or discarded).
#[derive(Debug, Clone)]
pub struct SpeculationResult {
    pub accepted: bool,
    pub duration_ms: u64,
    pub tool_calls: u32,
    pub files_written: u32,
}

/// Session-level accumulated speculation statistics.
#[derive(Debug, Clone, Default)]
pub struct SpeculationStats {
    pub total_speculations: u32,
    pub accepted_count: u32,
    pub discarded_count: u32,
    pub time_saved_ms: u64,
}

impl SpeculationStats {
    pub fn record(&mut self, result: &SpeculationResult) {
        self.total_speculations += 1;
        if result.accepted {
            self.accepted_count += 1;
            self.time_saved_ms += result.duration_ms;
        } else {
            self.discarded_count += 1;
        }
    }
}

// ─── Prompt Prediction ─────────────────────────────────────────────────────

use std::time::Duration;

/// Minimum idle time after an assistant response before prediction fires.
const PREDICTION_IDLE_THRESHOLD: Duration = Duration::from_secs(2);

/// Maximum number of recent messages to consider for prediction context.
const PREDICTION_CONTEXT_WINDOW: usize = 6;

/// A predicted next-prompt that can be used to speculatively pre-run tools.
#[derive(Debug, Clone)]
pub struct PromptPrediction {
    pub predicted_prompt: String,
    pub confidence: f32,
    pub generated_at: Instant,
}

/// Holds prediction state across ticks. Tracks whether a prediction has
/// already been generated for the current idle period (to avoid re-firing).
#[derive(Debug)]
pub struct PromptPredictor {
    /// The current prediction, if one has been generated this idle period.
    pub current: Option<PromptPrediction>,
    /// Instant the last assistant response completed (idle clock starts here).
    last_response_at: Option<Instant>,
    /// Whether we've already attempted prediction for this idle period.
    prediction_attempted: bool,
}

impl Default for PromptPredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptPredictor {
    pub fn new() -> Self {
        Self {
            current: None,
            last_response_at: None,
            prediction_attempted: false,
        }
    }

    /// Notify the predictor that an assistant response just completed.
    pub fn on_assistant_response(&mut self) {
        self.last_response_at = Some(Instant::now());
        self.prediction_attempted = false;
        self.current = None;
    }

    /// Notify the predictor that the user submitted a prompt (resets state).
    pub fn on_user_prompt(&mut self) {
        self.last_response_at = None;
        self.prediction_attempted = false;
        self.current = None;
    }

    /// Check whether we should speculate. Returns `Some(predicted_prompt)` when:
    /// - The user has been idle for >2 seconds after the last assistant response
    /// - We haven't already predicted for this idle period
    /// - The last messages provide enough context for a reasonable prediction
    ///
    /// `last_messages` should be the tail of the conversation (up to 6 messages).
    pub fn should_speculate(
        &mut self,
        idle_duration: Duration,
        last_messages: &[MessageSummary],
    ) -> Option<String> {
        // Don't re-predict if we already have one for this idle period
        if self.prediction_attempted {
            return None;
        }

        // Need at least the idle threshold
        if idle_duration < PREDICTION_IDLE_THRESHOLD {
            return None;
        }

        // Need at least one assistant message to predict from
        if last_messages.is_empty() {
            return None;
        }

        // Mark as attempted so we don't fire again this period
        self.prediction_attempted = true;

        // Generate prediction from context
        let prediction = predict_next_prompt(last_messages)?;

        let result = PromptPrediction {
            predicted_prompt: prediction.clone(),
            confidence: estimate_confidence(last_messages),
            generated_at: Instant::now(),
        };
        self.current = Some(result);
        Some(prediction)
    }

    /// Consume the current prediction (e.g. when starting speculative execution).
    pub fn take_prediction(&mut self) -> Option<PromptPrediction> {
        self.current.take()
    }
}

/// Minimal summary of a message for prediction purposes.
#[derive(Debug, Clone)]
pub struct MessageSummary {
    pub role: MessageRole,
    pub text: String,
    /// Tool names used in this message (if assistant).
    pub tools_used: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// Predict the user's next prompt based on recent conversation context.
///
/// Uses simple heuristics:
/// - If the last assistant message ended with a question → predict an answer
/// - If tools were recently used → predict a follow-up about results
/// - If there's a pattern of iterative refinement → predict continuation
///
/// Returns `None` if no confident prediction can be made.
pub fn predict_next_prompt(last_messages: &[MessageSummary]) -> Option<String> {
    let window = if last_messages.len() > PREDICTION_CONTEXT_WINDOW {
        &last_messages[last_messages.len() - PREDICTION_CONTEXT_WINDOW..]
    } else {
        last_messages
    };

    // Find the last assistant message
    let last_assistant = window
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant)?;

    let text = last_assistant.text.trim();

    // Heuristic 1: Assistant asked a question → predict a short affirmative
    if text.ends_with('?') {
        // If it's a yes/no question, predict "yes"
        let lower = text.to_lowercase();
        if lower.contains("should i")
            || lower.contains("shall i")
            || lower.contains("do you want")
            || lower.contains("would you like")
        {
            return Some("yes, go ahead".to_string());
        }
        // Otherwise can't confidently predict the answer
        return None;
    }

    // Heuristic 2: Assistant just completed file edits → predict "run the tests"
    if !last_assistant.tools_used.is_empty() {
        let has_writes = last_assistant
            .tools_used
            .iter()
            .any(|t| t == "Write" || t == "Edit" || t == "MultiEdit");
        if has_writes {
            return Some("run the tests to verify the changes work".to_string());
        }
    }

    // Heuristic 3: Assistant showed an error → predict "fix it"
    if text.contains("error") || text.contains("failed") || text.contains("Error") {
        return Some("fix that error".to_string());
    }

    // Heuristic 4: Assistant completed a task → predict "continue"
    if text.contains("Done") || text.contains("complete") || text.contains("finished") {
        // Check if there's an ongoing multi-step pattern
        let user_msgs: Vec<_> = window
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .collect();
        if user_msgs.len() >= 2 {
            // If the user has been giving sequential instructions, predict "continue"
            return Some("continue with the next step".to_string());
        }
    }

    None
}

/// Estimate confidence (0.0–1.0) of a prediction based on context signals.
fn estimate_confidence(messages: &[MessageSummary]) -> f32 {
    let mut confidence: f32 = 0.3; // base

    // More context → higher confidence
    if messages.len() >= 4 {
        confidence += 0.1;
    }

    // Recent tool usage → more predictable follow-ups
    if let Some(last) = messages.last()
        && !last.tools_used.is_empty()
    {
        confidence += 0.2;
    }

    // Pattern of short user messages → likely another short one
    let short_user_count = messages
        .iter()
        .filter(|m| m.role == MessageRole::User && m.text.len() < 30)
        .count();
    if short_user_count >= 2 {
        confidence += 0.1;
    }

    confidence.min(0.9)
}
