---
name: submodule-update-wgpui
description: Workflow command scaffold for submodule-update-wgpui in jfc.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /submodule-update-wgpui

Use this workflow when working on **submodule-update-wgpui** in `jfc`.

## Goal

Keeps the wgpui submodule up to date and integrates upstream fixes or changes.

## Common Files

- `references/wgpui`
- `crates/jfc-ui/src/main.rs`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Update the references/wgpui submodule.
- Optionally update related integration code in crates/jfc-ui/src/main.rs or other UI files.
- Commit with a message referencing the wgpui update or fix.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.