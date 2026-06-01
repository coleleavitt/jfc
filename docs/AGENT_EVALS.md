# Agent Eval Harness

JFC has deterministic, API-free prompt evals in `jfc-agents`.

Run:

```sh
cargo test -p jfc-agents evals
```

The initial suite checks:

- required built-in agents exist
- verification stays adversarial and read-only
- verification FAIL/PARTIAL reports remain durable through `verification-findings`
- delegation guidance preserves parallel fan-out and synthesis rules
- skill listings expose name/description without dumping full skill bodies

Add new cases in `crates/jfc-agents/src/evals.rs` when changing built-in prompts, skill rendering, dispatch guidance, or verification policy.
