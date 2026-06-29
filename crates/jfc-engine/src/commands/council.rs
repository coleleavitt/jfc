//! `/council` **session** subcommands — the RoundTable operator surface.
//!
//! The bare `/council <question>` form stays a one-shot fan-out
//! (`commands::context::cmd_council`). These subcommands drive a persistent,
//! resumable [`CouncilSession`] held on [`EngineState::council_session`]:
//! `start`, `continue`, `everyone`, `skip`, `@Name`, `consensus`, `synthesize`,
//! `verdict`, `tie`, `positions`, `flag`, `kick`, `dm`, `unmute`, `share`,
//! `end`, `status`.
//!
//! Each handler mutates the session and renders a fresh transcript/advisor
//! reply into the chat. Provider resolution reuses
//! [`crate::runtime::bootstrap::resolve_provider_model`] so the roster matches
//! AskModel/Council membership semantics.

use crate::commands::prelude::*;
use crate::council_session::{CouncilSeat, CouncilSession, CouncilSessionMode, Persona, Role};

/// Subcommands that route to a persistent session rather than the one-shot fan-out.
const SESSION_SUBCOMMANDS: &[&str] = &[
    "start",
    "continue",
    "next",
    "everyone",
    "skip",
    "consensus",
    "synthesize",
    "synthesis",
    "verdict",
    "tie",
    "positions",
    "flag",
    "kick",
    "dm",
    "approve",
    "deny",
    "leave",
    "unmute",
    "share",
    "export",
    "end",
    "status",
    "help",
];

pub fn is_session_subcommand(head: &str) -> bool {
    SESSION_SUBCOMMANDS.contains(&head.to_ascii_lowercase().as_str())
}

pub fn usage_text() -> String {
    "Council — one-shot or a turn-based session.\n\
     • `/council <question>` — fan out to N models and synthesise once.\n\
     • `/council start [debate|collaborate|blind-reveal|blindmap] [model-a,model-b] <topic>` — open a session.\n\
     • `/council continue` · `/council @Name <steer>` · `/council skip` — drive turns.\n\
     • `/council consensus` · `/council synthesize` · `/council verdict` · `/council tie <Name>`.\n\
     • `/council flag <claim>` · `/council kick <Name>` · `/council dm @A @B <topic>`.\n\
     • `/council approve` · `/council deny` — resolve model-requested challenges, asides, and votes.\n\
     • `/council unmute` · `/council positions` · `/council status` · `/council share` · `/council end`."
        .to_owned()
}

/// Route a session subcommand. `args` is everything after `/council`.
pub async fn dispatch(state: &mut EngineState, args: &str) {
    let (head, rest) = split_head(args);
    let head_lc = head.to_ascii_lowercase();

    if head.starts_with('@') {
        return steer_at(state, args).await;
    }

    match head_lc.as_str() {
        "start" => start_session(state, rest).await,
        "continue" | "next" => continue_turn(state).await,
        "everyone" => everyone(state).await,
        "skip" => skip(state),
        "consensus" => consensus(state).await,
        "synthesize" | "synthesis" => synthesize(state).await,
        "verdict" => verdict(state).await,
        "tie" => break_tie(state, rest),
        "positions" | "status" => status(state),
        "flag" => flag(state, rest),
        "kick" => kick(state, rest).await,
        "dm" => dm(state, rest).await,
        "approve" => approve(state).await,
        "deny" => deny(state),
        "leave" => leave(state, rest),
        "unmute" => unmute(state),
        "share" | "export" => share(state),
        "end" => end(state),
        "help" => reply(state, usage_text()),
        _ => reply(state, usage_text()),
    }
}

fn split_head(args: &str) -> (&str, &str) {
    match args.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (args, ""),
    }
}

fn reply(state: &mut EngineState, body: impl Into<String>) {
    state
        .messages
        .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
            body.into(),
        )]));
}

/// Post the standard "no active session" hint. Called from the `else` arm of a
/// `let Some(session) = state.council_session.as_mut()` bind (the failed borrow
/// has ended, so re-borrowing `state` here is sound).
fn require_no_session(state: &mut EngineState) {
    reply(
        state,
        "No active council session. Start one with `/council start <topic>`.",
    );
}

/// `/council start [mode] [model-a,model-b] <topic>`
async fn start_session(state: &mut EngineState, rest: &str) {
    if state
        .council_session
        .as_ref()
        .is_some_and(|s| !s.is_concluded())
    {
        reply(
            state,
            "A council session is already active. `/council end` it first.",
        );
        return;
    }
    let cfg = crate::config::load_arc();
    let session_cfg = cfg.council.as_ref().map(|c| &c.session);
    let default_mode = session_cfg
        .and_then(|c| CouncilSessionMode::parse(&c.mode))
        .unwrap_or(CouncilSessionMode::Debate);
    let (mode, after_mode) = match split_head(rest) {
        (head, tail) if CouncilSessionMode::parse(head).is_some() => {
            (CouncilSessionMode::parse(head).unwrap(), tail)
        }
        _ => (default_mode, rest),
    };
    let max_rounds = session_cfg.map(|c| c.max_rounds).unwrap_or(4).max(1);
    let aside_allowance = session_cfg.map(|c| c.aside_allowance).unwrap_or(1);
    let (model_ids, topic) = parse_models_and_topic(state, after_mode);
    if topic.trim().is_empty() {
        reply(state, "Provide a topic: `/council start <topic>`.");
        return;
    }

    let mut seats = Vec::new();
    let mut unresolved = Vec::new();
    for id in &model_ids {
        match crate::runtime::bootstrap::resolve_provider_model(&state.providers, id) {
            Some(res) => {
                let seat_id = format!("seat{}", seats.len() + 1);
                let mut seat = CouncilSeat::new(seat_id, id.clone(), res.provider, res.model);
                seat.asides_remaining = aside_allowance;
                seats.push(seat);
            }
            None => unresolved.push(id.clone()),
        }
    }
    if seats.is_empty() {
        reply(
            state,
            format!(
                "Could not resolve any council models from: {}.",
                model_ids.join(", ")
            ),
        );
        return;
    }
    apply_default_dispositions(&mut seats, mode);

    let mut session = CouncilSession::new(topic.clone(), mode, seats).with_max_rounds(max_rounds);
    session.start();
    if matches!(mode, CouncilSessionMode::BlindMapReduce) {
        if let Err(e) = session.run_blind_map_reduce(true).await {
            let round = session.round;
            session
                .transcript
                .push(crate::council_session::TranscriptEntry::system(
                    round,
                    format!("Blind Map-Reduce failed: {e}"),
                ));
        }
    } else if session.current_speaker.is_some() {
        if let Err(e) = session.run_current_turn().await {
            let round = session.round;
            session
                .transcript
                .push(crate::council_session::TranscriptEntry::system(
                    round,
                    format!("Opening turn failed: {e}"),
                ));
        }
    }
    let mut body = session.to_markdown();
    if !unresolved.is_empty() {
        body.push_str(&format!(
            "\n_(skipped unresolved models: {})_",
            unresolved.join(", ")
        ));
    }
    if session.is_concluded() {
        body.push_str(
            "\n\n_Use `/council share` to export this run, or `/council end` to clear it._",
        );
    } else {
        body.push_str(
            "\n\n_Use `/council continue` to advance, or `/council verdict` to finalize._",
        );
    }
    state.council_session = Some(session);
    reply(state, body);
}

/// `/council continue` — run the current speaker's turn, then advance.
async fn continue_turn(state: &mut EngineState) {
    // Validate without holding the borrow across the `run_current_turn` await.
    let refusal: Option<&str> = match state.council_session.as_ref() {
        None => return require_no_session(state),
        Some(s) if s.is_concluded() => {
            Some("This session has concluded. `/council end` or start a new one.")
        }
        Some(s) if s.active_count() == 0 => Some("No active participants remain."),
        Some(s) if s.current_speaker.is_none() => Some("No next speaker."),
        Some(_) => None,
    };
    if let Some(msg) = refusal {
        return reply(state, msg);
    }
    let session = state.council_session.as_mut().expect("session present");
    let notes = match session.run_current_turn().await {
        Ok((_, notes)) => notes,
        Err(e) => return reply(state, format!("Turn failed: {e}")),
    };
    render_recent_with_notes(state, 1, &notes);
}

/// `/council everyone` — every active seat answers the current prompt at once.
async fn everyone(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let ids: Vec<String> = session.active_seats().map(|s| s.id.clone()).collect();
    let mut answered = 0;
    for seat_id in &ids {
        if session.run_seat_turn(seat_id).await.is_ok() {
            answered += 1;
        }
    }
    session.round = session.round.saturating_add(1);
    render_recent(state, answered);
}

fn skip(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    session.skip_turn();
    let next = session
        .current_speaker
        .as_ref()
        .and_then(|id| session.seat(id))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "—".into());
    reply(state, format!("Skipped. Up next: {next}."));
}

/// `/council @Name <steer>` — post an operator steer and call that seat next.
async fn steer_at(state: &mut EngineState, args: &str) {
    let (mention, steer) = split_head(args);
    let name = mention.trim_start_matches('@');
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    if session.operator_muted {
        reply(
            state,
            "The council voted to mute operator input. `/council unmute` to lift it.",
        );
        return;
    }
    let seat_id = session.resolve_seat(name, None).map(|s| s.id.clone());
    let Some(seat_id) = seat_id else {
        reply(state, format!("No active participant named '{name}'."));
        return;
    };
    {
        let session = state.council_session.as_mut().expect("session present");
        if !steer.trim().is_empty() {
            let round = session.round;
            session
                .transcript
                .push(crate::council_session::TranscriptEntry::operator(
                    round,
                    steer.to_owned(),
                ));
        }
        session.call_next(&seat_id);
        match session.run_current_turn().await {
            Ok((_, notes)) => {
                render_recent_with_notes(state, 1, &notes);
                return;
            }
            Err(e) => return reply(state, format!("Turn failed: {e}")),
        }
    }
}

async fn consensus(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    match session.check_consensus().await {
        Ok(_) => render_recent(state, 1),
        Err(e) => reply(state, format!("Consensus failed: {e}")),
    }
}

async fn synthesize(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    match session.synthesize(None).await {
        Ok(_) => render_recent(state, 1),
        Err(e) => reply(state, format!("Synthesis failed: {e}")),
    }
}

async fn verdict(state: &mut EngineState) {
    let blocked = match state.council_session.as_ref() {
        Some(session) => session
            .verdict_blocked_by_flags()
            .then(|| session.open_flag_count()),
        None => return require_no_session(state),
    };
    if let Some(n) = blocked {
        reply(
            state,
            format!(
                "{n} flagged claim(s) still open — resolve or override before a verdict. (Proceeding anyway.)"
            ),
        );
    }
    let session = state.council_session.as_mut().expect("session present");
    match session.trigger_verdict().await {
        Ok(outcome) if outcome.tie => {
            let names: Vec<String> = outcome
                .positions
                .iter()
                .map(|p| {
                    session
                        .seat(&p.seat_id)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| p.seat_id.clone())
                })
                .collect();
            reply(
                state,
                format!(
                    "Tie — operator decides. Break it with `/council tie <Name>`. Positions: {}.",
                    names.join(", ")
                ),
            );
        }
        Ok(_) => render_recent(state, 2),
        Err(e) => reply(state, format!("Verdict failed: {e}")),
    }
}

fn break_tie(state: &mut EngineState, rest: &str) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let seat_id = session
        .resolve_seat(rest.trim(), None)
        .map(|s| s.id.clone());
    let Some(seat_id) = seat_id else {
        reply(
            state,
            "Name the winning participant: `/council tie <Name>`.",
        );
        return;
    };
    let outcome = session.break_tie(&seat_id);
    match outcome {
        Some(outcome) => reply(state, outcome.to_markdown()),
        None => reply(state, "No tie is pending."),
    }
}

fn status(state: &mut EngineState) {
    let out = match state.council_session.as_ref() {
        Some(session) => render_status(session),
        None => return require_no_session(state),
    };
    reply(state, out);
}

fn render_status(session: &CouncilSession) -> String {
    let mut out = format!(
        "Council `{}` · round {} · {} active · {} tokens.\n",
        session.mode.as_str(),
        session.round,
        session.active_count(),
        session.total_tokens(),
    );
    for seat in &session.seats {
        let stance = seat
            .last_stance
            .map(|(p, c)| {
                format!(
                    " [{}{}]",
                    p.as_str(),
                    c.map(|c| format!(" {c}%")).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let flags = if seat.kicked {
            " (kicked)"
        } else if seat.has_left {
            " (left)"
        } else {
            ""
        };
        out.push_str(&format!("• {}{flags}{stance}\n", seat.name));
    }
    if session.open_flag_count() > 0 {
        out.push_str(&format!("\n{} open flag(s).", session.open_flag_count()));
    }
    out
}

fn flag(state: &mut EngineState, rest: &str) {
    let claim = rest.trim();
    if claim.is_empty() {
        reply(
            state,
            "Usage: `/council flag <claim>` or `/council flag <Name>: <claim>`.",
        );
        return;
    }
    let open = match state.council_session.as_mut() {
        Some(session) => {
            let (target, claim_text) = match claim.split_once(':') {
                Some((name, c)) if session.resolve_seat(name.trim(), None).is_some() => (
                    session
                        .resolve_seat(name.trim(), None)
                        .map(|s| s.id.clone()),
                    c.trim().to_owned(),
                ),
                _ => (None, claim.to_owned()),
            };
            session.add_flag(claim_text, "operator", target);
            session.open_flag_count()
        }
        None => return require_no_session(state),
    };
    reply(state, format!("Flagged. {open} open flag(s)."));
}

async fn kick(state: &mut EngineState, rest: &str) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let seat_id = session
        .resolve_seat(rest.trim(), None)
        .map(|s| s.id.clone());
    let Some(seat_id) = seat_id else {
        reply(state, "Name a participant: `/council kick <Name>`.");
        return;
    };
    match session.run_kick_vote(&seat_id).await {
        Ok(_) => render_recent(state, 1),
        Err(e) => reply(state, format!("Kick vote failed: {e}")),
    }
}

/// `/council dm @A @B <topic>` — operator-directed sealed aside between two seats.
async fn dm(state: &mut EngineState, rest: &str) {
    let mentions: Vec<&str> = rest
        .split_whitespace()
        .filter(|t| t.starts_with('@'))
        .collect();
    if mentions.len() < 2 {
        reply(state, "Usage: `/council dm @A @B <topic>`.");
        return;
    }
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let a = session
        .resolve_seat(mentions[0].trim_start_matches('@'), None)
        .map(|s| s.id.clone());
    let b = session
        .resolve_seat(mentions[1].trim_start_matches('@'), None)
        .map(|s| s.id.clone());
    let (Some(a), Some(b)) = (a, b) else {
        reply(state, "Could not resolve both participants.");
        return;
    };
    let brief = rest
        .split_whitespace()
        .filter(|t| !t.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ");
    match session
        .run_side_conversation(&a, &b, None, Some(brief))
        .await
    {
        Ok(_) => reply(
            state,
            "Sealed aside complete (visible to you; sealed from the table).",
        ),
        Err(e) => reply(state, format!("Aside failed: {e}")),
    }
}

async fn approve(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    match session.approve_pending_action().await {
        Ok(notes) => render_recent_with_notes(state, 2, &notes),
        Err(e) => reply(state, format!("Approval failed: {e}")),
    }
}

fn deny(state: &mut EngineState) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let note = session
        .deny_pending_action()
        .unwrap_or_else(|| "No pending council action to deny.".to_owned());
    reply(state, note);
}

fn leave(state: &mut EngineState, rest: &str) {
    let Some(session) = state.council_session.as_mut() else {
        return require_no_session(state);
    };
    let seat_id = session
        .resolve_seat(rest.trim(), None)
        .map(|s| s.id.clone());
    let Some(seat_id) = seat_id else {
        reply(state, "Name a participant: `/council leave <Name>`.");
        return;
    };
    session.leave_table(&seat_id, "operator removed");
    reply(state, "Removed from the table.");
}

fn unmute(state: &mut EngineState) {
    match state.council_session.as_mut() {
        Some(session) => session.unmute_operator(),
        None => return require_no_session(state),
    }
    reply(
        state,
        "Operator mute lifted — your input reaches the council again.",
    );
}

fn share(state: &mut EngineState) {
    let md = match state.council_session.as_ref() {
        Some(session) => session.to_markdown(),
        None => return require_no_session(state),
    };
    reply(state, md);
}

fn end(state: &mut EngineState) {
    if let Some(session) = state.council_session.as_mut() {
        session.conclude();
        let md = session.to_markdown();
        state.council_session = None;
        reply(state, format!("Session concluded.\n\n{md}"));
    } else {
        reply(state, "No active council session.");
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Render the last `n` transcript entries plus any directive-orchestration
/// notes (challenges, asides, votes, operator DMs) as an advisor reply.
fn render_recent_with_notes(state: &mut EngineState, n: usize, notes: &[String]) {
    // Some directives append system entries to the transcript (votes,
    // challenges); widen the tail to include them. Notes without a transcript
    // entry (e.g. a private operator DM) are appended explicitly.
    render_recent(state, n + notes.len());
    if !notes.is_empty() {
        let appendix = notes
            .iter()
            .map(|note| format!("_· {note}_"))
            .collect::<Vec<_>>()
            .join("\n");
        reply(state, appendix);
    }
}

/// Render the last `n` transcript entries as an advisor reply.
fn render_recent(state: &mut EngineState, n: usize) {
    let Some(session) = state.council_session.as_ref() else {
        return;
    };
    let total = session.transcript.len();
    let start = total.saturating_sub(n.max(1));
    let mut out = String::new();
    for entry in &session.transcript[start..] {
        let who = match &entry.speaker {
            crate::council_session::Speaker::Operator => "Operator".to_owned(),
            crate::council_session::Speaker::System => "Council".to_owned(),
            crate::council_session::Speaker::Seat(id) => session
                .seat(id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| id.clone()),
        };
        let blind = if entry.blind { " · blind" } else { "" };
        out.push_str(&format!("**{who}**{blind}: {}\n\n", entry.content));
    }
    if let Some(next) = session
        .current_speaker
        .as_ref()
        .and_then(|id| session.seat(id))
        .map(|s| s.name.clone())
    {
        if !session.is_concluded() {
            out.push_str(&format!("_Up next: {next} — `/council continue`._"));
        }
    }
    reply(state, out);
}

/// Pull a leading comma-separated model list off `rest`, returning the ids and
/// the remaining topic. `None` when there's no comma-list prefix (so the caller
/// falls back to config/active models). Pure — unit-testable without state.
fn split_explicit_models(rest: &str) -> Option<(Vec<String>, String)> {
    let (head, tail) = rest.split_once(char::is_whitespace)?;
    if !head.contains(',') {
        return None;
    }
    let ids: Vec<String> = head
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    (!ids.is_empty()).then(|| (ids, tail.trim().to_owned()))
}

/// Parse an optional leading comma-separated model list and the remaining topic.
/// Falls back to config members or active+advisor models.
fn parse_models_and_topic(state: &EngineState, rest: &str) -> (Vec<String>, String) {
    if let Some(explicit) = split_explicit_models(rest) {
        return explicit;
    }
    let cfg = crate::config::load_arc();
    let council_cfg = cfg.council.as_ref();
    if let Some(members) = council_cfg
        .map(|c| c.members.as_slice())
        .filter(|m| !m.is_empty())
    {
        let ids: Vec<String> = members
            .iter()
            .map(|m| m.model.trim())
            .filter(|m| !m.is_empty())
            .map(str::to_owned)
            .collect();
        if !ids.is_empty() {
            return (ids, rest.to_owned());
        }
    }
    let mut ids = vec![state.model.to_string()];
    if let Some(advisor) = state.local_advisor_model.as_ref() {
        let advisor = advisor.to_string();
        if advisor != ids[0] {
            ids.push(advisor);
        }
    }
    (ids, rest.to_owned())
}

/// Seed default personas/professions/roles so a fresh roster has variety:
/// in debate mode the second seat plays skeptic; in collaborate mode seats get
/// distinct roles. Operators can re-`ASSIGN` mid-session.
fn apply_default_dispositions(seats: &mut [CouncilSeat], mode: CouncilSessionMode) {
    match mode {
        CouncilSessionMode::Collaborate => {
            const ROLES: &[Role] = &[
                Role::Boss,
                Role::Researcher,
                Role::Coder,
                Role::Qa,
                Role::Writer,
            ];
            for (i, seat) in seats.iter_mut().enumerate() {
                seat.role = ROLES[i % ROLES.len()];
            }
        }
        _ => {
            // Give the second seat a skeptic disposition so a fresh 2-seat
            // debate has tension out of the box; professions stay None until
            // the operator (or an ASSIGN directive) sets them.
            if let Some(second) = seats.get_mut(1) {
                second.persona = Persona::Skeptic;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council_session::test_support::seat;

    #[test]
    fn is_session_subcommand_matches_normal() {
        assert!(is_session_subcommand("start"));
        assert!(is_session_subcommand("VERDICT"));
        assert!(is_session_subcommand("consensus"));
        assert!(is_session_subcommand("approve"));
        assert!(is_session_subcommand("deny"));
        assert!(!is_session_subcommand("why"));
        assert!(!is_session_subcommand("anthropic/claude"));
    }

    #[test]
    fn split_explicit_models_parses_comma_list_robust() {
        let (ids, topic) = split_explicit_models("a,b,c Is X true?").unwrap();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(topic, "Is X true?");
        // No comma → fall through (None) so the caller uses config/active models.
        assert!(split_explicit_models("just a plain topic").is_none());
        // Comma-list with a trailing topic that itself has commas keeps only the
        // leading head as the model list.
        let (ids2, topic2) = split_explicit_models("x,y do A, then B").unwrap();
        assert_eq!(ids2, vec!["x", "y"]);
        assert_eq!(topic2, "do A, then B");
    }

    #[test]
    fn default_dispositions_debate_and_collaborate_normal() {
        let mut debate = vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")];
        apply_default_dispositions(&mut debate, CouncilSessionMode::Debate);
        assert_eq!(debate[0].persona, Persona::Default);
        assert_eq!(debate[1].persona, Persona::Skeptic);

        let mut collab = vec![
            seat("a", "Alpha", "x"),
            seat("b", "Beta", "y"),
            seat("c", "Gamma", "z"),
        ];
        apply_default_dispositions(&mut collab, CouncilSessionMode::Collaborate);
        assert_eq!(collab[0].role, Role::Boss);
        assert_eq!(collab[1].role, Role::Researcher);
        assert_eq!(collab[2].role, Role::Coder);
    }
}
