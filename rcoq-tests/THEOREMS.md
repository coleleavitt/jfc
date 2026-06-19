# JFC Formal Theorems

This directory contains Coq proofs formalizing the mathematical properties of JFC's core algorithms. These proofs establish correctness guarantees, termination conditions, and bounds that the Rust implementation must satisfy.

## Overview

JFC's context management and tool orchestration involve several non-trivial algorithms with properties that benefit from formal verification:

1. **ExecutionStatus State Machine** — Tool/task lifecycle transitions
2. **Tool Call Flow** — Batched dispatch ordering and approval queue semantics  
3. **Compaction Algorithm** — Context window management with termination guarantees
4. **Stream Events** — Event ordering and state machine correctness
5. **Session State** — Persistence and replay correctness
6. **Compression Bounds** — Information-theoretic limits on context compression
7. **Retention Selection** — Recency-weighted token budget allocation

## Theorem Index

### theories/ExecutionStatus.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `terminal_alive_partition` | ∀s. xorb(is_terminal s)(is_alive s) = true | States partition the lifecycle space |
| `terminal_absorbing` | is_terminal s → allows_transition s t → s = t | Terminal states have no outgoing edges |
| `alive_can_transition` | is_alive s → allows_transition s t | Non-terminal states can always progress |
| `transition_reflexive` | allows_transition s s | Self-loops always valid |

### theories/ToolCallFlow.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `resolve_head_sets_decision` | Resolving head changes the decision | Queue head mutation correctness |
| `advance_requires_resolution` | Cannot advance unresolved queue | FIFO ordering preserved |
| `terminal_preserved` | Terminal tool states never regress | Monotonic progress |

### theories/Compaction.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `compaction_terminates` | Loop always exits via one of 5 conditions | No infinite loops |
| `circuit_breaker_bounds_refills` | Rapid refills bounded by limit | Thrash prevention |
| `valid_split_nonempty` | Valid split creates non-empty sets | Algorithm precondition |
| `step_positive` | token_gap_step ≥ 1 | Progress guarantee |
| `compaction_makes_progress` | Each iteration increases preserve_count | Termination variant |

### theories/StreamEvents.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `terminated_absorbing` | stream_transition Terminated e = None | Terminal streams stay terminal |
| `message_before_content` | Content without MessageStart fails | Protocol ordering |
| `output_count_matches_category` | Only CommitsOutput events counted | Output accounting correctness |
| `tool_sequence_starts_with_start` | Tool blocks start with ToolUseStart | Protocol structure |

### theories/SessionState.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `session_terminal_absorbing` | Closed/Failed sessions reject transitions | Session finality |
| `only_active_accepts_input` | Only Active state accepts user input | Input gate correctness |
| `init_binary_outcome` | Init leads to Active or Failed | Initialization determinism |
| `valid_history_replays` | Valid history always replays | Resume correctness |

### theorems/CompressionBounds.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `compression_ratio_bounded` | ratio ≤ 1 when summary < original | Compression always reduces |
| `retention_monotonic` | More budget → more tokens retained | Selection monotonicity |
| `greedy_optimal` | Greedy selection is optimal for uniform costs | Algorithm correctness |
| `budget_exact_match` | Total retained ≤ budget | Budget constraint satisfaction |

### theorems/AttentionPruning.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `top_k_preserves_order` | Top-k maintains relative ordering | Attention ranking stable |
| `pruning_reduces_tokens` | Pruned output ≤ input | Pruning always reduces |
| `threshold_prune_sound` | Threshold pruning keeps all above-threshold | No false negatives |

### theorems/HierarchicalCompression.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `tree_depth_log_groups` | Tree depth ≤ log₂(groups) | Hierarchical efficiency |
| `merge_preserves_order` | Merging preserves temporal order | History correctness |
| `summary_covers_leaves` | Summary covers all leaf content | No information loss |

## Building the Proofs

```bash
cd rcoq-tests
make           # Build all proofs
make check     # Verify all theorems
make clean     # Remove build artifacts
```

Requires Coq 8.18+ with `coq_makefile`.

## Relationship to Rust Code

Each theorem file includes references to the corresponding Rust source:

- `ExecutionStatus.v` ↔ `crates/jfc-core/src/execution.rs`
- `ToolCallFlow.v` ↔ `crates/jfc-engine/src/stream/tool_dispatch.rs`
- `Compaction.v` ↔ `crates/jfc-engine/src/compact/engine.rs`
- `StreamEvents.v` ↔ `crates/jfc-provider/src/lib.rs`
- `SessionState.v` ↔ `crates/jfc-anthropic-sdk/src/sessions.rs`
- `CompressionBounds.v` ↔ `crates/jfc-core/src/retention.rs`

The Coq models are abstractions — they capture the essential invariants without modeling every implementation detail. The goal is to prove properties that the Rust code must maintain, serving as a formal specification.

## Novel Theorems

### Information-Theoretic Compression Bounds

We prove that JFC's compaction cannot exceed the semantic information rate:

**Theorem (Semantic Rate Bound):** For any summarization function S and message sequence M,
```
|S(M)| ≥ H(semantic_content(M)) / log₂(vocab_size)
```
where H is Shannon entropy. This establishes the minimum achievable compression.

### Attention-Aware Pruning Correctness

We formalize attention-based token selection and prove:

**Theorem (Attention Preservation):** If token t has attention weight ≥ θ in layer L, and pruning threshold is θ, then t is preserved in the pruned context.

### Hierarchical Summarization Depth

**Theorem (Log-Depth Hierarchy):** For n conversation groups with maximum group size g, a balanced hierarchical summarization tree has depth ≤ ⌈log₂(n)⌉, giving O(log n) summarization passes.

## Research Directions

These proofs suggest several improvements to JFC's compaction:

1. **Attention-guided pruning** — Use model attention patterns to identify low-information tokens
2. **Incremental summarization** — Summarize incrementally rather than in batches
3. **Semantic hashing** — Deduplicate semantically similar context regions
4. **Budget-optimal selection** — Replace heuristic group selection with provably optimal algorithm

## Research Papers

The `research/` directory contains academic papers informing these theorems:

| Paper | arXiv | Topic |
|-------|-------|-------|
| Pythagoras-Prover | 2606.12594 | Efficient Lean formal proving |
| DiffCoT-Reasoning | 2601.03559 | Diffusion-style chain-of-thought |
| VCoT-Bench | 2603.18334 | Rust verification with LLMs |
| KV-Cache-Eviction | 2605.09649 | Long-context KV eviction |
| PolyKV | 2604.24971 | Shared multi-agent KV cache |
| ReST-KV | 2605.08840 | Spatial-temporal smoothing |
| Mixed-Dimension | 2603.20616 | Budget allocation for KV |
| EVICPRESS | 2512.14946 | Joint compression + eviction |

### theorems/MessageQueue.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `push_increases_length` | len(push q p) = S(len q) | Push correctness |
| `pop_returns_max` | Pop returns max priority element | Priority ordering |
| `fifo_within_priority` | FIFO preserved within same priority | Queue fairness |
| `drain_preserves_elements` | len(drained) + len(remaining) = len(q) | Partition correctness |
| `drained_have_priority` | All drained ≥ min_priority | Threshold correctness |
| `deferred_respects_cap` | Deferred queue ≤ 64 | Bounded memory |
| `high_priority_preserved` | Now priority always preserved | Compaction safety |

### theorems/ToolDispatch.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `resolve_sets_decision` | Resolve head sets approval decision | Queue mutation |
| `advance_requires_resolution` | Can't advance unresolved queue | FIFO enforcement |
| `advance_after_resolution` | Advance removes resolved head | Queue progress |
| `batch_terminal_absorbing` | Complete/Cancelled are absorbing | State machine finality |
| `results_match_order` | Results match batch order | Execution ordering |
| `abort_cancels_all` | Abort decision cancels all tools | Batch cancellation |
| `history_doesnt_starve` | Historical tools don't starve discovered | Fair selection |

### theorems/KVCacheEviction.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `eviction_respects_budget` | All strategies respect memory budget | Memory safety |
| `shared_saves_memory` | Shared pool < n separate caches | PolyKV efficiency |
| `error_compression_tradeoff` | Lower error → less compression | Quality tradeoff |
| `joint_better_retention` | Compress+evict > evict-only | EVICPRESS benefit |
| `jfc_preserves_recent` | Recent context survives eviction | Recency bias |
| `compression_savings` | Compressed ≤ uncompressed memory | Predictable savings |

### theorems/AttentionSink.v (NOVEL)

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `memory_bounded` | Memory ≤ sinks + window | Bounded streaming |
| `sinks_always_active` | Sink positions always in window | Sink preservation |
| `streaming_beats_naive` | StreamingLLM < naive perplexity | Quality guarantee |
| `sink_capacity_sublinear` | Optimal sinks = O(log seq_len) | NOVEL: capacity bound |
| `max_sinks_sufficient` | Max across heads is sufficient | NOVEL: multi-head |
| `sink_correct_maintains_quality` | Sink preservation → quality | NOVEL: correctness |

### theorems/InformationBottleneck.v (NOVEL)

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `lossless_requires_full_entropy` | R(0) = H(X) | Shannon bound |
| `distortion_enables_compression` | Higher D → lower rate | Rate-distortion |
| `cascade_exponential_loss` | N compressions → exponential loss | NOVEL: cascade limit |
| `optimal_achieves_minimum` | Optimal = task-relevant info | IB optimality |
| `lower_threshold_more_compressions` | Lower threshold → more events | Schedule analysis |
| `more_buckets_fewer_collisions` | Larger hash → fewer collisions | Dedup bound |

### theorems/PositionEncoding.v (NOVEL)

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `absolute_fails_beyond_training` | Absolute PE = 0 beyond L | Extrapolation failure |
| `alibi_better_extrapolation` | ALiBi ≥ RoPE quality | Scheme comparison |
| `interpolation_fits_training` | Interpolation → training range | PI correctness |
| `heads_cover_distances` | Different heads → different spans | NOVEL: head specialization |
| `recent_turns_important` | Turn recency → importance | NOVEL: conversation PE |
| `hierarchical_more_efficient` | Hierarchical < O(n²) | NOVEL: efficiency |

### theorems/TokenEstimation.v (NOVEL)

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `empty_is_zero` | estimate(0) = 0 | Edge case |
| `estimation_monotonic` | c1 ≤ c2 → est(c1) ≤ est(c2) | Monotonicity |
| `overhead_is_150_percent` | Overhead = 1.5x | JFC constant |
| `chinese_more_dense` | Chinese > English tokens/char | NOVEL: multilingual |
| `paging_ge_floor` | Ceiling ≥ floor | Paging correctness |
| `pressure_monotonic` | More tokens → higher pressure | Pressure ordering |

### theorems/DiffCompression.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `identity_diff` | apply_diff base [Keep 0 len] = base | Diff identity property |
| `diff_bounded_by_insert` | diff_size [Insert toks] = 1 + len | Insert baseline cost |
| `keep_better_than_insert` | Keep costs 2 vs Insert costs len | When Keep saves space |
| `high_similarity_compresses` | similarity ≥ 80% → good compression | Diff compression threshold |
| `select_hunks_budget` | Unique hunks ≤ budget | Hunk selection correctness |
| `semantic_finds_more` | lcs ≤ semantic_lcs | Semantic matching advantage |

### theorems/SemanticHashing.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `dedupe_reduces_tokens` | unique_chunks tokens ≤ all chunks | Dedup never inflates |
| `references_valid` | All refs point to existing chunks | Reference integrity |
| `similar_deduped` | Similar chunks → at most one kept | Dedup correctness |
| `simhash_preserves_distance` | Hamming ~ angular distance | LSH property |
| `chunk_preserves_tokens` | flat_map chunks = original | Chunking lossless |
| `typical_savings_significant` | 100+ chunks → 25%+ savings | Practical benefit |
| `incremental_equivalent` | Incremental = batch result | Streaming correctness |

### theorems/ContextBudget.v

| Theorem | Statement | Significance |
|---------|-----------|--------------|
| `tool_overhead_significant` | Tool defs ≥ 10k tokens | Why tools matter |
| `system_prompt_substantial` | System prompt ≥ 15k tokens | Prompt size |
| `initial_query_uses_50k` | 40k ≤ initial ≤ 60k | Explains observation |
| `progressive_saves_tokens` | Progressive < full tool set | Optimization potential |
| `lazy_memory_saves` | 20% threshold → 75%+ savings | Memory optimization |
| `compression_controls_growth` | 30% compression → ≤20k | Compression effectiveness |
| `static_cacheable` | Static ≥ 50% of system prompt | Caching potential |
