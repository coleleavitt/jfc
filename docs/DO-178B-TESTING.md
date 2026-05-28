# DO-178B Test Convention

This codebase adopts the RTCA DO-178B §6.4.2 testing methodology for safety-critical software. This is not because jfc is avionics software, but because the testing discipline scales well to complex systems with many edge cases.

## Test Classification

All tests are organized into two categories:

### `_normal` Tests
Tests that verify the **happy path** and standard behavior. These exercises the primary requirements under expected conditions.

- No error injection
- Valid inputs
- System in expected state
- Verifies correctness of the main flow

**Example:**
```rust
#[test]
fn parse_schedule_daily_normal() {
    let s = parse_schedule("@daily").unwrap();
    assert!(matches!(s, CronSchedule::Crontab { hour: CronField::Any, .. }));
}
```

### `_robust` Tests
Tests that verify **boundary conditions, error cases, and defensive behavior**. These ensure the system doesn't crash or behave unpredictably when:

- Given invalid inputs
- In error states
- With missing resources
- At boundary values

**Example:**
```rust
#[test]
fn parse_schedule_empty_robust() {
    assert!(parse_schedule("").is_err());
    assert!(parse_schedule("   ").is_err());
}
```

## Coverage Goal

Every public function or critical behavior should have **both** a `_normal` and `_robust` variant:

- `_normal` proves it works correctly on valid input
- `_robust` proves it fails safely on invalid input

This ensures no silent failures or panics on edge cases.

## Files Using This Convention

The following crates use the DO-178B test split systematically:

- `crates/jfc-daemon/src/tests.rs` — schedule parsing, daemon state, background agents
- `crates/jfc-providers/src/anthropic*.rs` — OAuth, model config, HTTP retries
- `crates/jfc-memory/src/recall.rs` — memory storage and recall
- `crates/jfc-markdown/src/lib.rs` — markdown rendering
- `crates/jfc-ui/src/inline_tools.rs` — inline tool execution
- `crates/jfc-ui/src/tools/tests.rs` — tool dispatch and monitoring

## Example Test Block Structure

```rust
// ─── function_name (DO-178B _normal) ─────────────────────────────────────
// Tests that verify correct behavior on valid input

#[test]
fn function_name_happy_path_normal() {
    // Standard case
}

// ─── function_name (DO-178B _robust) ─────────────────────────────────────
// Tests that verify safe failure on invalid input, boundary conditions, etc.

#[test]
fn function_name_missing_file_robust() {
    // Error handling
}

#[test]
fn function_name_zero_value_robust() {
    // Boundary condition
}
```

## Why This Matters

1. **Exhaustiveness**: Reviewers can quickly scan test names and verify all paths are covered
2. **Maintenance**: Adding a new edge case? Add a `_robust` test, not a random test
3. **Audit trail**: The naming convention serves as documentation
4. **Safety**: Distinguishes "does it work?" from "does it not crash?"

## References

- [`docs/RTCA-DO-178B.pdf`](RTCA-DO-178B.pdf) — Full RTCA DO-178B standard (local copy)
- [`docs/RTCA-DO-178B.txt`](RTCA-DO-178B.txt) — Plain-text version for grepping
- RTCA DO-178B §6.4.2: Software Unit Design and Verification
- [DO-178B Summary](https://en.wikipedia.org/wiki/DO-178B)
- Adopted methodology: Not for compliance, but for discipline and scalability
