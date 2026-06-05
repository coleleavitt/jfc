# Thinking spinner: frozen token count, vanishing output count, doubled "thinking" labels

**Summary** — During summarized/extended thinking, the spinner shows a frozen `↻ 150 thinking`, the `↓ N tokens` count disappears, and two separate "thinking" indicators render at once.

**User goal** — Watch live progress (tokens/sec, count) while the model thinks on a long agentic turn, the same way Claude Code surfaces thinking progress.

**Symptom** — Spinner reads `Synthesizing (48s · ↻ 150 thinking)` and a minute later still `Surfacing (1m 49s · ↻ 150 thinking)` — the `150` never moves, no `↓ tokens` count is visible during the whole thinking phase, and the word "thinking" appears both as the token chip and (when the phase verb fires) as `thinking ✶ drafting`.

**Repro** — Run a thinking-enabled turn (claude-opus-4-8, effort xhigh) that thinks for >30s before emitting tools/text. Watch the spinner during the pre-output thinking window.

**Observed** (from `~/.config/jfc/logs/ses_20260601_005604.log`):
- `first stream byte … first_kind="reasoning"` fires **63 times over ~94s** (07:58:09→07:59:43). That trace is gated on `streaming_response_bytes == 0`, so every reasoning delta in this phase carried **empty text** — summarized-thinking keepalive pings.
- `message_delta … output_tokens=0` for the entire thinking phase; jumps to `output_tokens=8223` only when the first tool block starts.
- The thinking-token estimate reached ~150 early, then the server emitted bare pings with no further `estimated_tokens`, so the chip froze at 150.
- No `ThinkingTokens dropped (buffer full)` lines — the events were delivered, the number simply stopped climbing.

**Hypothesis** — Three compounding issues in `crates/jfc-ui/src/spinner.rs::status_segments`:
1. `↓ N tokens` is gated on `output_tokens > 0`, which is 0 during thinking → the count vanishes for the whole phase.
2. The thinking-token chip prints a raw `⟳ {n} thinking` with no live element; when the server stops incrementing `estimated_tokens` the number freezes and reads as a hang (the elapsed clock is the only moving thing, and it's in a different segment).
3. The thinking-token chip AND the `thinking {glyph} {phase}` verb both render the word "thinking" → doubled. The `planning/considering/drafting` phases are derived from `elapsed.as_secs()`, not anything real.

Fix direction: collapse to ONE thinking segment carrying a **live thinking duration** (never frozen) with the token estimate folded in (`thinking 1m 30s · ~150 tok`); keep a single cumulative token count visible across the thinking→output transition; drop the fake phases.

**Context** — model claude-opus-4-8, provider anthropic-oauth, effort xhigh, betas include thinking-token-count-2026-05-13; jfc HEAD ea1e879 (binary built 00:33, has c4f45a3 thinking-phase fix); terminal TUI spinner.
