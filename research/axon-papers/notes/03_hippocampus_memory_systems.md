# Hippocampus — Memory Systems (Capture → Consolidate → Recall)

> **Axon SDK mapping:** the *memory plane* (the "hippocampus for coding agents" — CortexKit's
> Magic Context). Three operations: **encode** (fast write of episodes), **consolidate** (offline
> batch job that compresses, verifies, and transfers to durable store), **recall** (retrieval at
> inference). Lesion → **anterograde amnesia** = an agent that can reason over context but cannot
> persist new long-term memories. This is the exact pathology of a stateless harness.

---

## 1. The three operations

### Encode (acquisition)
The hippocampus binds the elements of an experience (what/where/when) into an episodic trace
within seconds — rapid, one-shot, high-plasticity writes. Mechanism: NMDA-receptor-dependent LTP
in the trisynaptic circuit (entorhinal cortex → dentate gyrus → CA3 → CA1).
- **CA3 recurrent collaterals** form an autoassociative (attractor) network → **pattern completion**
  (recall the whole from a partial cue) and storage.
- **Dentate gyrus** performs **pattern separation** (orthogonalize similar inputs so they don't
  collide). *SDK reading: dedup/orthogonalize on write so near-duplicate memories don't overwrite.*

### Consolidate (the "dreamer" / offline job)
**Systems consolidation**: memories initially dependent on the hippocampus are gradually
transferred to **neocortex** for long-term storage, over hours–years.
- **Sleep replay**: during non-REM **sharp-wave ripples (SWRs)**, hippocampal place-cell sequences
  are **reactivated** (often time-compressed, sometimes reversed), driving transfer to cortex and
  integration with existing knowledge.
- Replay also occurs in the **awake** state (Carr, Jadhav & Frank, 2011) — the *same* mechanism
  serves both consolidation (write path) and planning/retrieval (read path).
- Consolidation **merges duplicates, abstracts schemas, and retires stale traces** — precisely
  CortexKit's "dreamer" that runs overnight: merge, verify against the codebase, promote patterns,
  retire stale memories.

> Downloaded: `systems_memory_consolidation_sleep.pdf` — systems consolidation during sleep:
> oscillations (slow oscillations + spindles + ripples), neuromodulators, and synaptic remodeling;
> `hippocampus_systems_consolidation_fear.pdf` — the hippocampus's role in systems consolidation of
> *remote* memory (when memory becomes hippocampus-independent).

### Recall (retrieval)
A partial cue drives **pattern completion** in CA3 to reinstate the full trace; relevant memories
surface automatically. *SDK reading: semantic retrieval each turn — query memories + raw history +
git commits, exactly CortexKit's `ctx_search`.*

## 2. The two-stage / complementary-learning-systems (CLS) model

McClelland, McNaughton & O'Reilly (1995): the brain needs **two** memory systems because one
cannot do both jobs:
- **Hippocampus** = fast learner, high plasticity, sparse/orthogonal codes → store specifics now,
  without catastrophic interference.
- **Neocortex** = slow learner, low plasticity, overlapping codes → extract statistical structure
  / schemas over many interleaved exposures.

> This is the architectural justification for a *separate* memory module with a *separate*
> consolidation agent: you cannot bolt fast episodic write and slow schema-formation onto the same
> store without catastrophic forgetting. Run the historian (fast, cheap model) and the dreamer
> (slow, batch) as distinct jobs over a shared store.

## 3. Anterograde amnesia — the failure mode

Bilateral hippocampal damage (canonically patient **H.M.**, Scoville & Milner 1957) causes
**anterograde amnesia**: intact old memories and intact short-term/working memory, but inability
to form *new* long-term declarative memories. Procedural learning is spared (separate system).

> Downloaded: `memory_impairments_types_causes.pdf` — taxonomy of memory impairments, types,
> causes, and molecular players across neurological dysfunction.

**SDK reading:** a harness with no memory module *is* an anterograde-amnesiac agent — every
session is a fresh hire. The fix is not a bigger context window (that's just working memory); it's
a write→consolidate→recall pipeline that persists beyond the session.

## 4. Mapping table

| Hippocampal operation | Mechanism | Axon memory-module analog |
|---|---|---|
| Encode | NMDA LTP, DG pattern separation, CA3 storage | fast write; dedup/orthogonalize on ingest |
| Consolidate | SWR replay in sleep, transfer to cortex | offline "dreamer": merge, verify, abstract, retire |
| Recall | CA3 pattern completion from partial cue | semantic retrieval over memory + history + commits |
| CLS dual system | fast hippocampus + slow cortex | separate episodic store + slow schema store |
| Anterograde amnesia | hippocampal lesion | stateless harness with no persistence |

---

### Downloaded papers for this topic
- `systems_memory_consolidation_sleep.pdf` — Systems memory consolidation during sleep (2025)
- `hippocampus_systems_consolidation_fear.pdf` — Role of hippocampus in systems consolidation of remote fear memory (2026)
- `memory_impairments_types_causes.pdf` — Memory impairments: type, causes, molecular players (2026)

### Canonical references (cite from primary literature)
- Scoville & Milner (1957), *Loss of recent memory after bilateral hippocampal lesions*, J. Neurol. Neurosurg. Psychiatry.
- McClelland, McNaughton & O'Reilly (1995), *Why there are complementary learning systems...*, Psychol. Rev.
- Carr, Jadhav & Frank (2011), *Hippocampal replay in the awake state*, Nat. Neurosci.
- Buzsáki (2015), *Hippocampal sharp wave-ripple*, Hippocampus.
