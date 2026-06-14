---
type: context
scope: team
created: 2026-05-23T00:36:26.356476786+00:00
---
magic-context (cortexkit) at ~/WebstormProjects/forks/magic-context is the gold-standard reference for cross-session memory, historian/dreamer/sidekick architecture, user-profile pipeline, key-file pinning, auto-search hints, caveman compression, synthetic-todowrite injection, and shared SQLite across harnesses (OpenCode + Pi).

Why: When designing jfc's self-learning agent (Initiative 3, t214–t223), mirror their architecture wholesale — battle-tested across 2000+ hours of real usage.

How to apply: For any "agent learning from sessions" / "cross-session memory" / "automatic context recall" / "agent self-improvement" question, read `~/WebstormProjects/forks/magic-context/ARCHITECTURE.md` first, then the specific subsystem under `packages/plugin/src/features/magic-context/` (memory/, dreamer/, user-memory/, key-files/, git-commits/, message-index.ts, search.ts). Their `dreamer/task-prompts.ts` and `agents/historian.ts` are the canonical prompt designs. Their `storage-memory.ts` COLUMN_MAP is the canonical schema for memories with seen_count/normalized_hash/verification_status fields.
