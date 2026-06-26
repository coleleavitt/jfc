<!-- Generated 2026-06-19 by a multi-agent gap-analysis workflow: 8 theme readers over the 9 research notes + 45 pdftotext-extracted papers, synthesized and completeness-critiqued. Findings were cross-checked against the source tree. -->

# Axon Implementation Gap Analysis (v2 — completeness-augmented)

Audit of the Axon Rust SDK against its neuroscience design corpus. The 59 source findings collapse into ~24 distinct engineering gaps once overlaps are merged; this revision adds **8 gaps** that surfaced from a completeness pass against under-weighted research themes (network dynamics/timing, IIT integration, temporal credit assignment, stochastic-policy reproducibility, learned-state persistence, and the async/tool integration seam), and tightens the dependency-aware ranking.

All original findings were re-verified directly against source. The verification confirms every load-bearing claim:
- `Route::weight` is an immutable `Weight(i16)` with only `new`/`get`/`weight()` accessors — no setter (`route.rs:7-24,58-60`).
- `exploration`, `learning_rate`, `risk_tolerance` have **zero callers** anywhere outside their own definitions in `axon-modulate` (grep across all crate `src/` returns nothing).
- `RoutingTable::select` is a plain argmax returning `Option<&Route>` with `AmbiguousRoute` on ties (`routing.rs:35-75`).
- `CircuitBreaker` is binary trip/reset with no half-open/degraded state (`breaker.rs`).
- `Mismatch` carries `action/expected/observed` strings but exposes no magnitude (`axon-predict/src/lib.rs:68-105`).
- `Workspace::broadcast` evicts index 0 — plain FIFO, no salience field on `Broadcast` (`axon-workspace/src/lib.rs:101-106`).
- `Executive::decide` maps `Retry` to `Advance { next: step }` — literal re-emission of the same step (`agent.rs:265`); `Decision` has no `Replan` arm (`lib.rs:72-78`).
- `RunEvent` has only `Entered/Emitted/Stopped/Dropped` — no weight-change, halt, stall, or degrade events (`event.rs`).

The single largest structural theme stands: **nothing in the SDK closes a learning loop.** The completeness pass sharpens *why* that loop cannot close even if weights were made mutable today: there is (a) no error magnitude to scale by (3.1), (b) no temporal credit-assignment path to route a terminal outcome back to the multi-hop route choices that caused it (NEW 3.3), and (c) no persistence or observability for whatever gets learned (NEW 8.2, 8.3). Plasticity (1.1) has three hidden prerequisites, not one.

---

## 1. Routing Core

### 1.1 Route weights are immutable; no plasticity or decay (HIGH)
**Missing:** `Route::weight` is a fixed `Weight(i16)` set at construction, read only in `RoutingTable::select` for argmax. No `reinforce`, no `decay`, no traversal/success counter. Merges activity-dependent myelination, LTP/LTD, RPE/TD-error plasticity, weak-tie plasticity, and astrocyte GC/prune findings.
**Why it matters:** An agent SDK that cannot strengthen paths that work or weaken paths that fail can never improve with experience — it re-derives every decision from static priors.
**Brain principle:** Synaptic plasticity (Hebbian/STDP), dopamine RPE gating learning rate, activity-dependent myelination.
**Axon step:** Make `Weight` mutable behind a `RoutingTable` API: `reinforce(from, to, delta)` / `decay(rate)`, effective weight = `base + learned`. Add a `Plasticity` trait `fn update(&self, w: Weight, outcome: &Outcome) -> Weight` reading `Modulators::learning_rate()` and consuming `axon_predict::Mismatch::magnitude()` (3.1). Pair with an astrocyte-style `RoutingTable::prune(threshold)`. **Now explicitly gated on 3.1, 3.3, 8.2.**

### 1.2 No fan-out / parallel recruitment (HIGH)
**Missing:** `RoutingTable::select` returns at most one `&Route`; `Runtime` advances exactly one signal per step. No multicast or top-k.
**Why it matters:** Real agent events drive several modules at once (an alert that both writes memory and interrupts the planner). Single-winner blocks GWT broadcast.
**Brain principle:** Multiareal thalamic relay; NNT distributed parallel recruitment; GWT ignition.
**Axon step:** Add `RoutingTable::select_all` (top-k by weight, deterministic ordering) and a `Runtime` fan-out dispatch behind a distinct API so single-winner stays the default. Wire to `axon-workspace` so `Broadcast::alert` reaches multiple subscribers.

### 1.3 Selection ignores global modulator/context state (HIGH)
**Missing:** `Gate::admits` and `select` take only the signal. `Exploration`, `RiskTolerance`, `learning_rate` are never consulted at decision time. Merges dopamine-gain, salience-weighted relay, transthalamic-context, and active-inference exploration.
**Why it matters:** Neuromodulator scalars are meant to retune routing without rewiring. Today they are dead config — an `Exploratory` mode does not change which route wins.
**Brain principle:** Tonic/phasic dopamine Go/NoGo balance; transthalamic relays; precision-weighting; FEP exploration bonus.
**Axon step:** Introduce a read-only `RoutingContext<'a>` carrying `&Modulators` + optional salience/goal cue, passed to `select`/`admits`. `exploration` → softmax temperature / epsilon over weights; salience scales effective weight; high `risk` raises selection margin. Couple with 8.4 (seeded RNG) so stochastic selection stays reproducible.

### 1.4 Single-winner has no explicit NoGo / suppression margin (HIGH)
**Missing:** `select` is a plain argmax with `AmbiguousRoute` on ties. Losers are silently filtered — no competition margin, no Go/NoGo restraint, no record of what was inhibited. Merges Go/NoGo and signed-edge findings.
**Why it matters:** "Deciding when NOT to act" is a core executive function. Without a margin, a marginally-best route fires even when no option is clearly right.
**Brain principle:** Basal-ganglia direct/indirect pathway; default tonic inhibition.
**Axon step:** Add a `SelectionPolicy` trait (default = argmax) + NoGo margin on `RoutingTable`: a winner must beat runners-up by a configurable margin or `select` returns `None`. Add edge `Sign { Excitatory, Inhibitory }` to `Route` so active inhibitory edges subtract from competitors. Surface suppressed candidates in `RunEvent`/`TraceStep`.

### 1.5 No global stop / cancellation primitive (MEDIUM)
**Missing:** `Runtime::run_observed` has only `StepLimit` and per-route `CircuitBreaker`. No way to interrupt an in-flight run atomically.
**Why it matters:** A safety primitive every agent dispatcher needs — distinct from a budget cap or a single route tripping. For a coding agent, this is the user-pressed-Ctrl-C / "stop, that command is destructive" path.
**Brain principle:** STN hyperdirect "hold everything" global brake.
**Axon step:** Add a cloneable `StopToken` to `Runtime`/`AsyncRuntime`, checked at loop top and before `module.handle`; emit `RunEvent::Halted`, return `RunStatus::Halted`. On `AsyncRuntime` this must also drop/cancel in-flight boxed futures.

### 1.6 No per-signal cost/energy budget (MEDIUM)
**Missing:** `Signal` carries only payload + `Priority`; `Route` has no cost. `StepLimit` caps step count, not weighted spend.
**Why it matters:** Agent hops cost tokens/latency/dollars; routing should prefer cheaper paths and shed load under budget pressure — directly relevant for a coding SDK calling paid LLM/tool endpoints.
**Brain principle:** Metabolic cost of action potentials; myelination cuts per-signal energy.
**Axon step:** Add a `Cost` newtype field to `Route` and a running budget on `Runtime` alongside `StepLimit`; refuse/deprioritize routes that would exceed remaining budget.

### 1.7 Graceful degradation / hub-aware breakers missing (MEDIUM)
**Missing:** `CircuitBreaker` is binary trip/reset — no half-open/degraded state, no per-module health score, no fallback route, no hub awareness. Merges AMU-robustness, spine-loss, hub-robustness.
**Why it matters:** Failures concentrate at hubs and cascade; the SDK should degrade to a reduced-capability path rather than hard-stop.
**Brain principle:** Axon-myelin unit keeps feeding the axon after the oligodendrocyte dies; network degeneration concentrates at connectome hubs.
**Axon step:** Extend `breaker` with half-open/degraded state + per-module health score; emit a `RunEvent` when a hub edge degrades; add a lower-weight fallback route. Once topology exists (1.8), auto-place stricter breakers on high-degree modules.

### 1.8 No explicit graph/topology object (HIGH)
**Missing:** `RoutingTable` is a flat `Vec<Route<P>>`. No node model, no hub/degree/path-length/modularity metrics, no community detection, no composite (subgraph-as-module). Merges connectome-graph and multiscale-parcellation.
**Why it matters:** "The module graph IS a connectome." Without a graph object the SDK cannot validate wiring, identify hubs for breaker placement, or reason about reachability.
**Brain principle:** Small-world modular connectome with rich-club hubs; multiscale hierarchical parcellation.
**Axon step:** Add a read-only `ModuleGraph` (adjacency over `ModuleId`) derived from registered `Route`s + a `topology` module computing degree/hub-ness, reachability, connected components. Later: composite `Module` wrapping a subgraph behind one `EndpointId`.

### 1.9 No refractory / rate limiting or loop detection (LOW)
**Missing:** No per-route min-inter-fire interval, no last-fired tracking, no oscillation detection. A reinforced feedback loop (once weights mutate) could thrash to `StepLimit`. Merges refractory-period, over-synchrony, E/I-balance.
**Why it matters:** Backpressure beyond all-or-nothing breaker trips; prevents one hot module being hammered every step.
**Brain principle:** Na+-channel refractory period; STN-GPe desynchronization; E/I homeostasis.
**Axon step:** Add an optional `Refractory` gate (field on `Route`) tracking last-fired step + min-interval. Add route-history tracking in `run_observed` that emits `RunEvent::Stalled` when an edge repeats beyond a threshold and optionally raises `exploration` to break the loop.

### 1.10 No metastable routing profiles per mode (MEDIUM)
**Missing:** `Mode` never reconfigures which routes are active or their weights — it only bumps priority/verification gain (`apply_attention`/`verification_gain`).
**Why it matters:** One fixed graph should yield many context-dependent routing modes selected by global state.
**Brain principle:** Kuramoto/chimera metastability — fixed structure, many transient functional configurations.
**Axon step:** Add a `RoutingProfile` per `Mode` (or have gates consult `Mode` via `RoutingContext` from 1.3) so admission/weight is a function of mode.

---

## 2. Memory

### 2.1 Recall is brittle lexical overlap, not similarity-based pattern completion (HIGH)
**Missing:** `EpisodicStore::recall` scores by exact lowercased-token intersection + tag overlap (`lexical_overlap`, lib.rs:143-151). A paraphrased cue scores 0. No embeddings, no cosine, no graded completion. Merges CA3 pattern-completion, dense-Hopfield, Generative-Agents relevance.
**Why it matters:** Content-addressable recall from noisy/partial cues is the core memory primitive; literal matching fails the moment wording drifts (synonyms, reordered tokens, a renamed symbol in a codebase).
**Brain principle:** CA3 autoassociative attractor; dense Hopfield ≡ softmax attention.
**Axon step:** Add a `Retriever`/`Embedder` trait (deterministic mock + `axon-provider` impl behind `openai`), an embedding field on `Episode`, `RecallResult::score` from cosine; keep `lexical_overlap` as a hybrid fallback. Optionally softmax top-k.

### 2.2 No importance/salience and no true recency decay (HIGH)
**Missing:** `Episode` has only `text` + `tags`. No importance, no `last_access`; recency is just an id tie-break (lib.rs:116-121). `Modulators` never consulted by `axon-memory`. Merges salience-scoring and scored-retrieval.
**Why it matters:** Generative-Agents retrieval = weighted(recency-decay × importance × relevance). Two of three factors absent.
**Brain principle:** NE/dopamine tag memories by salience; exponential recency decay over last access.
**Axon step:** Add `importance: f32` + `last_access` to `Episode` (importance derivable from `Modulators::attention` at encode); recall score = normalized weighted sum behind a `Relevance` trait; update `last_access` on recall.

### 2.3 Store grows unboundedly — no decay/forgetting/paging (HIGH)
**Missing:** `EpisodicStore` is an unbounded `Vec` that only pushes (lib.rs:104). `Consolidator` only counts tags, never retires episodes. No capacity bound, no MemGPT tiering/page-fault. Merges forgetting, MemGPT-paging.
**Why it matters:** Unbounded growth degrades recall relevance and footprint; an agent needs hot/warm/cold tiers with summarize-on-evict and fault-back-in.
**Brain principle:** NREM delta-wave active forgetting; CLS mapped to OS virtual memory (MemGPT).
**Axon step:** Add a `Tier` enum + `HierarchicalStore: MemoryStore` with a bounded working tier + `MemoryPressure` signal; `page_out` (summarize lowest-value into schema, drop) and `page_in` (fault cold items back). Per-episode strength + decay in `Consolidator`.

### 2.4 Slow/schema store is write-only; consolidation is a tag histogram (MEDIUM)
**Missing:** `SchemaMemory` is produced by `Consolidator` but never read — `recall` queries only the episodic `Vec`. `consolidate` just counts tags (lib.rs:224-239); no replay, no merge/abstraction, no synthesized insights. Merges CLS-read-path, SWR-replay, reflection.
**Why it matters:** The "dreamer" arc (replay → abstract → write back → consult) is half-built; generalization from accumulated experience does not exist.
**Brain principle:** CLS slow neocortical schema; SWR sleep replay; Generative-Agents reflection.
**Axon step:** Add a `SchemaStore`; make `recall` query both. Extend `Consolidator` with a `reflect()` step behind a `Reflector` trait (deterministic clustering default, LLM-backed optional) that clusters similar episodes, emits insight memories, marks consolidated episodes for retirement.

### 2.5 Dedup is exact-match, not near-duplicate orthogonalization (MEDIUM)
**Missing:** `same_memory` collapses episodes only on trim + ASCII case-fold equality (lib.rs:128-130). Reworded near-duplicates accumulate.
**Why it matters:** The store fills with redundant overlapping traces — the opposite of pattern separation.
**Brain principle:** Dentate-gyrus orthogonalization.
**Axon step:** Replace the dedup predicate with a similarity threshold (token Jaccard now, embedding cosine once `Embedder` from 2.1 exists); collapse above threshold by returning/merging the existing id.

### 2.6 No working-memory → long-term encode bridge; no semantic/procedural stores (MEDIUM)
**Missing:** `axon-workspace` and `axon-memory` are disconnected — nothing transfers salient evicted broadcasts into the episodic store. Only episodic + thin schema; no `SemanticStore` or `ProceduralStore`. Merges WM-vs-LTM, heterogeneous-recall, CoALA-three-way-memory.
**Why it matters:** CoALA prescribes episodic + semantic + procedural; without the encode bridge and procedural store the agent re-plans from scratch every time and never persists skills.
**Brain principle:** WM (dlPFC) feeds hippocampal LTM; CoALA/SOAR/ACT-R three-way split.
**Axon step:** Add an encode bridge (adapter in `axon-exec`) calling `MemoryStore::encode` on salient `Broadcast`s before FIFO eviction, gated by `Modulators` salience. Add `SemanticStore` (typed `Fact`) and `ProceduralStore` (named `Plan` keyed by goal pattern); `Planner` consults and writes back. Add a `RecallSource` trait with git-log/history adapters via `axon-tools::GitStatus`.

---

## 3. Prediction / Learning

### 3.1 Mismatch is categorical — no error magnitude (HIGH)
**Missing:** `Correction::Escalate(Mismatch)` is boolean; `Mismatch` exposes `action/expected/observed` but no distance method (axon-predict lib.rs:68-105). Merges graded-error, precision-weighting.
**Why it matters:** Prerequisite for §1.1 — reinforcement has nothing to scale by; the system knows *that* it was wrong, not *how* wrong.
**Brain principle:** RPE and cerebellar climbing-fiber errors are graded; precision = inverse-variance weighting.
**Axon step:** Add `Mismatch::magnitude() -> f32` (normalized token/edit distance between expected and observed) and surface it on `Broadcast::alert`. Have `Verifier::verify` accept `Modulators` so `verification_gain`/`attention` scale escalate-vs-retry (precision-weighted threshold).

### 3.2 No learned/hierarchical forward model (MEDIUM)
**Missing:** `axon-predict` implements only the verifier half — `Expected` is an author-supplied matcher; `verify` is static string comparison (lib.rs:110-122). No `predict(context)`, no model update, no stacking. Merges cerebellar-forward-model, hierarchical-predictive-coding.
**Why it matters:** Predictions are hand-written and never improve; "send the diff, not the state" has no hierarchy and the climbing-fiber teaching signal has no analog.
**Brain principle:** Cerebellar forward/inverse models trained by climbing-fiber LTD; Rao-Ballard hierarchical predictive coding.
**Axon step:** Add a `ForwardModel`/`Predictor` trait: `predict(context) -> Prediction` and `observe(prediction, outcome)` updating internal state from the `Mismatch`. Higher-level predictor consumes a lower's mismatch (stacking). Route `Correction::Escalate(Mismatch)` back through `wire_loop` to the Executive/Planner.

### 3.3 No temporal credit assignment / eligibility trace across hops (HIGH — NEW)
**Missing:** When `Executive` finally verifies an `Observe`, the outcome is attributed to nothing upstream. There is no trace of *which* route choices (Planner→Tool, the specific `Act`) led there, and no mechanism to discount reward back over the multi-hop path. `TraceStep` records `from→to` transitions but is never fed back into anything that adjusts weights. **This is the hidden second prerequisite for 1.1:** even with mutable weights and a graded error, the SDK cannot decide *which edge* to reinforce when the consequence is several hops removed from the choice.
**Why it matters:** A coding agent's failure ("tests still red") surfaces many steps after the bad decision (a wrong file edit five hops back). Without eligibility traces / TD-style discounting, plasticity can only reinforce the last edge, learning the wrong lesson. This is the difference between a credit-assignment-correct learner and superstitious reinforcement.
**Brain principle:** Eligibility traces in TD(λ); dopamine RPE bridging the temporal gap between action and reward; synaptic tagging-and-capture holding a synapse "eligible" until a later neuromodulatory signal.
**Axon step:** Have `run_observed` maintain a decaying eligibility trace over recently-traversed `(from,to)` edges (a `Vec<(EdgeId, f32)>` discounted by λ each step). On a terminal/observed outcome, apply the graded error (3.1) scaled by each edge's eligibility through the `Plasticity` trait (1.1). Surface the assignment in a new `RunEvent::Reinforced { edge, delta }` (ties to 8.2). Keep the trace opt-in (only allocated when a `Plasticity` policy is installed) so default runs stay allocation-free.

---

## 4. Neuromodulation

### 4.1 Knobs are declarative, not load-bearing (MEDIUM — cross-cutting)
**Missing:** `learning_rate`, `exploration`, `risk` defined on `Modulators` but read by zero callers (verified: only `mode` flows through `apply_attention`/`verification_gain`/`decide`). No ACh-style encode-vs-recall mode in memory.
**Why it matters:** "A few scalars retune all modules without rewiring" is borrow #8; today changing a knob changes almost nothing.
**Brain principle:** Four diffuse neuromodulator systems (dopamine/5-HT/ACh/NE) set global gain.
**Axon step:** Integration umbrella for 1.1/1.3/2.2/3.1/3.3 — thread `&Modulators` through `Runtime`/`AsyncRuntime` step context so all modules see one global state: `exploration` → routing temperature, `learning_rate` → weight updates, `risk` → breaker/selection sensitivity, `attention` → workspace admission + memory importance, an encode/recall `Mode` → memory recall breadth.

### 4.2 No disinhibitory gate combinator (LOW)
**Missing:** Gates are flat predicates (`Allow`/`DropSignal`/`MinPriority`/`CircuitBreaker`); no inhibitor another gate can suppress. PV/SST/VIP collapse to one priority number.
**Why it matters:** Attention/top-down state should *release* an otherwise-closed route, distinct from a flat threshold.
**Brain principle:** VIP interneurons inhibit SST/PV to open the gate for top-down signals.
**Axon step:** Add a `Disinhibit { inhibitor, release }` gate combinator to `axon-core::gate` that admits when the inhibitor would block *unless* modulator state releases it; wire `Mode::Focused`/`Salient` as the VIP release.

---

## 5. Workspace / Attention

### 5.1 FIFO buffer, not a salience-gated ignition bus (HIGH)
**Missing:** `Workspace::broadcast` evicts index 0 (plain FIFO, lib.rs:101-106) — a low-value Observation can evict a high-value Alert by arrival order. No salience field on `Broadcast`, no attention-weighted retention, no subscriber/push (modules must poll `broadcasts()`). Merges GWT-ignition, orthogonal-subspaces.
**Why it matters:** GWT requires competition for admission and system-wide broadcast to subscribers; the current buffer is passive storage that loses important items.
**Brain principle:** GWT/GNWT ignition; orthogonal subspaces keep prepare/execute non-interfering.
**Axon step:** Add salience/priority to `Broadcast` (reuse `axon-core::Priority`); admit/evict by salience (lowest evicted, recency tie-break); add a subscriber/observer stream so a successful broadcast pushes system-wide. Partition the buffer by a `GoalId`/`Phase { Prepare, Execute }` key. Wire `axon-exec` to read the bus.

### 5.2 No integration measure — workspace cannot tell broadcast from genuine integration (LOW — NEW)
**Missing:** Nothing measures whether a broadcast was actually *integrated* by downstream modules versus merely deposited in a buffer. The workspace has no notion of how many distinct modules consumed an item, nor any cross-module coherence signal.
**Why it matters:** Under-weighted IIT theme. For an agentic SDK the practical, non-mystical reading of IIT/Φ is an **observability metric**: "did this alert actually reach and change behavior in ≥k modules, or did it ignite and fizzle?" That distinguishes a real GWT ignition from a no-op broadcast and is directly useful for debugging why an agent ignored a warning. (Like criticality in 8.1, this is "measure, don't architect-around.")
**Brain principle:** IIT integrated information Φ — system state irreducible to its parts; GNWT ignition as system-wide availability.
**Axon step:** Once subscribers exist (5.1), track per-broadcast consumer count and emit it as an observability metric on the workspace's subscriber stream (e.g. `Ignition { item, reached: usize }`). Do **not** compute true Φ or make it a control law — surface reach/coherence counters only.

---

## 6. Executive / Planning

### 6.1 No working-memory scratchpad with HOLD/UPDATE gate (HIGH)
**Missing:** No maintained typed context. `Planner` walks a fixed precomputed `Plan` and is stateless between steps (agent.rs:107-137); the only persisted context is the unbounded `EpisodicStore` and FIFO `Workspace`. No protected task-context set, no HOLD-vs-UPDATE gate. Merges dlPFC-WM, BG-WM-gating, continuous-attractor-integrator.
**Why it matters:** The agent cannot keep a goal active while integrating new evidence — the core WM function. `decide` reads only the current Observe payload, not live context.
**Brain principle:** dlPFC persistent delay-period activity + BG HOLD/UPDATE gate; MD→PFC maintenance; continuous-attractor integrator.
**Axon step:** Add `WorkingMemory<T>` (`axon-core` or new `axon-context`): a bounded typed slot map with `hold`/`update`/`evict` gated by a `GatePolicy` (threshold driven by `Modulators::attention`). Optional leaky `Integrator`. Wire into `Executive` so `decide` reads live context.

### 6.2 Monitoring never re-plans; Retry just re-emits the same step (HIGH)
**Missing:** `decide` maps `Correction` to Continue/Retry/Escalate/AskUser, but `Retry` re-emits the identical step (`Advance { next: step }`, agent.rs:265) — literal perseveration — and Escalate halts. No `Replan`, no alternative route, no retry-count bound. Merges conflict-monitoring/re-route and cognitive-flexibility/set-shift.
**Why it matters:** mPFC/ACC detects a failing plan and *re-routes*; Axon detects but only retries-identically or halts — the exact perseveration/rigidity failure mode the transdiagnostic literature warns about.
**Brain principle:** mPFC/ACC conflict-and-error monitoring triggering control adjustment; dlPFC+ACC+BG set-shifting.
**Axon step:** Add a `Replan` arm to `Decision`; on repeated/severe mismatch have `Executive` call `propose_plan`/`replan(goal, failure_context)` and swap the `Planner`'s plan instead of re-emitting. Track per-step retry counts in working memory (6.1) to bound retries and demote a repeatedly-failing route (set-shift, ties to 1.1).

### 6.3 Risk knob dead; no value/risk appraisal before committing (MEDIUM)
**Missing:** `RiskTolerance` is stored and never read; `AskUser` is only an alias of Escalate (lib.rs:266), never produced from a risk threshold. Actions admitted purely by payload-variant match — no proactive inhibition.
**Why it matters:** OFC "commit vs ask the user" is unimplemented; high-risk low-confidence actions execute instead of deferring. For a coding agent this is exactly the `rm -rf` / force-push gate.
**Brain principle:** vlPFC response inhibition; OFC value/risk appraisal.
**Axon step:** Add a `RiskGate`/value appraisal in `axon-exec` (or a `Gate` in `axon-core`) scoring a proposed `Act` against `Modulators::risk_tolerance()` and routing to `AskUser`/inhibition over threshold; make `decide` consult risk.

### 6.4 Flat plan + single string goal; no hierarchy or goal stack (MEDIUM)
**Missing:** `Plan` is a flat `Vec<Step>`; `Planner` emits one `Act` per index linearly (no sub-goal expansion). `Goal` is a single `String` (workspace lib.rs:7), never read by the loop, no status/priority/progress. Merges hierarchical-control, goal-stack, internal-vs-external-action.
**Why it matters:** Long-horizon nested tasks cannot be represented; objectives cannot be maintained/prioritized/tracked; internal actions (recall/reflect/predict) are not routable alongside tools.
**Brain principle:** Rostro-caudal PFC hierarchy + corticostriatal release gate; PFC goal maintenance; CoALA internal/external action symmetry.
**Axon step:** Make `Plan` recursive (`Step::Action | Step::SubGoal(Plan)`) with a plan-frame stack in `Planner`; a higher-order gate releases a child frame only when the parent precondition fires. Promote `Goal` to a stack/queue with `status { Active, Blocked, Done }` + priority, updated each Observe. Register internal actions (Recall/Reflect/Predict) as `Module`s in the same `RoutingTable` as `RoutedTool`.

---

## 7. Provider / Tools

The original report dismissed this area as "no standalone gaps." The completeness pass disagrees for a *coding* SDK: the provider/tool seam has real integration gaps, not just leverage as a dependency.

### 7.1 Tools are sync-only and cannot integrate with AsyncRuntime; no parallel/cancellable tool calls (MEDIUM — NEW)
**Missing:** `axon-tools` exposes a synchronous `Tool` trait and `ToolModule` adapter; there is no `AsyncModule`/async-tool path even though `AsyncRuntime` exists. A real coding tool (shell command, network fetch, LLM call) blocks the single-threaded runtime, cannot run concurrently with other tools, and cannot be cancelled mid-flight (compounding 1.5).
**Why it matters:** Coding agents routinely fan out independent tool calls (read three files, run two test suites) and must cancel a runaway shell command. Sync-only tools force serialization and make the `StopToken` (1.5) unable to actually interrupt a blocked `ShellCommand`.
**Brain principle:** Parallel sensorimotor channels; peripheral effectors operate concurrently with central processing.
**Axon step:** Add an `AsyncTool` trait + `AsyncToolModule` adapter mirroring the sync pair, integrated with `AsyncRuntime`'s boxed-future dispatch. Thread cancellation from the `StopToken` (1.5) into the tool future. This is the prerequisite for opt-in fan-out (1.2) to be useful with real (I/O-bound) tools.

### 7.2 Tool outcomes never feed prediction or procedural learning (MEDIUM — NEW)
**Missing:** `RoutedTool` returns an observation string consumed only by the `Verifier`'s static match. Tool latency, exit status, cost, and success/failure are not captured as structured `Outcome` signal, so they cannot feed the forward model (3.2), the procedural store (2.6), or route plasticity (1.1).
**Why it matters:** The richest learning signal a coding agent has is "did this tool call succeed and how long/expensive was it." Discarding it to a string blocks the entire learning loop from using the one ground-truth signal available without an LLM.
**Brain principle:** Proprioceptive/interoceptive feedback as teaching signal; reafference.
**Axon step:** Give `Tool`/`AsyncTool` a structured result (status, cost, latency) flowing into `Outcome`; let `Verifier`/`ForwardModel` consume it and `Plasticity` (1.1) scale by it. Keep LLM calls confined to provider modules; the core stays LLM-free.

---

## 8. Dynamics / Observability

### 8.1 No fan-out/branching metrics, no criticality instrumentation (LOW — defer)
**Missing:** No measurement of average fan-out/branching, no homeostatic load controller.
**Why it matters:** Low priority — the synthesis verdict is "know, don't bet": do not architect the core around criticality. Branching is not definable until fan-out (1.2) exists.
**Brain principle:** Self-organized criticality (branching σ≈1) — heavily caveated.
**Axon step:** Defer. Once fan-out lands, add average-fan-out-per-step to `RunReport` as an observability metric only, not a control law.

### 8.2 RunEvent/RunReport cannot observe the learning loop (MEDIUM — NEW)
**Missing:** `RunEvent` has exactly four variants — `Entered/Emitted/Stopped/Dropped` (event.rs) — all topological. There is no event for a weight change, a reinforcement, a breaker degrade, a halt, a stall, or a suppressed candidate. Once weights become mutable (1.1) the most important behavior of the system — what it learned and why — is invisible.
**Why it matters:** A learning agent SDK is untestable and undebuggable without introspection into *what changed*. You cannot write a test asserting "the failing route was demoted" if no event reports it. Several earlier gaps (1.4 suppressed candidates, 1.5 Halted, 1.7 degrade, 1.9 Stalled, 3.3 Reinforced) all need a home here.
**Brain principle:** N/A — this is the observability counterpart to plasticity; the engineering analog of recording from the circuit while it learns.
**Axon step:** Extend `RunEvent` with `Reinforced { edge, delta }`, `Suppressed { candidate, by_margin }`, `Degraded { module }`, `Halted`, `Stalled { edge, count }`. Add learned-weight snapshots to `RunReport`. This is a cross-cutting enabler verified-by every HIGH learning gap.

### 8.3 Learned routing state is not persisted; serde covers everything except weights (MEDIUM — NEW)
**Missing:** The optional `serde` feature persists memory, workspace, and modulate state (verified: `#[cfg_attr(feature = "serde", derive(...))]` on `Episode`/`Workspace`/`Modulators`), but **not** `Route`/`Weight`/`RoutingTable` — `Weight` derives no serde, and `Route` holds a `Box<dyn Gate>` that cannot be trivially serialized. Today that is fine (weights are static priors). The moment 1.1 lands, the single most valuable thing the agent learns — its routing weights — evaporates on restart.
**Why it matters:** Learning with no persistence is a goldfish. A coding agent that re-learns which tools work every session provides no compounding value. This gap is invisible now and becomes HIGH-severity the instant plasticity ships, so it must be designed alongside 1.1, not after.
**Brain principle:** Long-term potentiation consolidates to structural (persistent) change; systems consolidation to neocortex.
**Axon step:** Make `Weight`/`base+learned` serde-serializable and add a `RoutingTable` weight snapshot/restore API keyed by `(from, to)` edge identity (decoupled from the non-serializable gate, which is reconstructed from code at wiring time). Persist alongside the existing memory/modulate snapshots under the same `serde` feature.

### 8.4 Stochastic policies have no seeded RNG; runs become non-reproducible (MEDIUM — NEW)
**Missing:** Everything is currently deterministic. The moment `exploration` becomes load-bearing as a softmax temperature / epsilon-greedy term (1.3) or loop-breaking (1.9), route selection becomes stochastic with no controlled randomness source — no seed on `Runtime`, no injected RNG.
**Why it matters:** A coding agent SDK is only trustworthy if a run is reproducible: bug reports, regression tests, and CI all require that the same goal + same seed yields the same trajectory. Ad-hoc `rand::random()` inside selection would make every test flaky and every failure unreproducible.
**Brain principle:** N/A — neural noise is real (stochastic resonance), but the engineering requirement is *controlled* noise.
**Axon step:** Add a seedable RNG handle to `Runtime`/`AsyncRuntime` (inject a `dyn RngCore` or a seed), consumed by `RoutingContext` (1.3) for any stochastic selection. Default to a fixed seed so `run` stays reproducible; expose seed in `RunReport` for replay. Design this *with* 1.3, not after.

---

## Top gaps to tackle next

Dependency-aware ordering. The completeness pass inserts three new prerequisites into the critical path: **graded error (3.1) + credit assignment (3.3) + observability (8.2) are all hard prerequisites for plasticity (1.1)**, and **seeded RNG (8.4) must land with modulator-threaded selection (1.3)**, and **weight persistence (8.3) must be designed with 1.1**.

1. **Graded prediction error** (3.1) — smallest change, unblocks all learning. `axon-predict`: `Mismatch::magnitude()`.
2. **Learning-loop observability** (8.2) — without it 1.1 is untestable/undebuggable. `axon-core`: extend `RunEvent` + `RunReport`. Cheap, unblocks verification of everything below.
3. **Temporal credit assignment / eligibility trace** (3.3) — the hidden second prerequisite for correct plasticity. `axon-core`: decaying edge trace in `run_observed`.
4. **Mutable, decaying route weights + plasticity** (1.1) — closes the learning loop, now correctly gated on 3.1/3.3/8.2. `axon-core`: `RoutingTable::reinforce`/`decay`, `Plasticity` trait.
5. **Thread modulators through selection/encode + seeded RNG** (1.3 + 4.1 + 8.4) — makes the dead `exploration`/`risk`/`learning_rate` knobs load-bearing *and* keeps stochastic selection reproducible. `axon-core`: `RoutingContext` + seedable RNG.
6. **Persist learned routing weights** (8.3) — designed with 1.1 so learning survives restart. `axon-core`: serde on `Weight` + weight snapshot API.
7. **Working-memory scratchpad with HOLD/UPDATE gate** (6.1) — `axon-core`/`axon-context`: `WorkingMemory<T>`.
8. **Re-planning on mismatch (kill perseveration)** (6.2) — `axon-exec`: `Decision::Replan`.
9. **Embedding/similarity recall** (2.1) — `axon-memory`: `Retriever`/`Embedder` trait.
10. **Importance + recency-decay scored retrieval** (2.2) — `axon-memory`: `Episode.importance`/`last_access`.
11. **Async/cancellable tools + structured outcomes** (7.1 + 7.2) — `axon-tools`: `AsyncTool`; prerequisite for fan-out to be useful and for tool results to feed learning.
12. **Bounded memory with decay/forgetting/paging** (2.3) — `axon-memory`: `HierarchicalStore`.
13. **Salience-gated workspace ignition bus** (5.1) — `axon-workspace`: salience admission + subscribers.
14. **Explicit ModuleGraph / topology** (1.8) — `axon-core`: `ModuleGraph` + `topology`.
15. **NoGo selection margin + signed edges** (1.4) — `axon-core`: `SelectionPolicy` + `Sign`.
16. **Opt-in fan-out / parallel recruitment** (1.2) — `axon-core`: `select_all`.

---

## Themes judged under-covered in the original report

- **Temporal credit assignment (TD/eligibility traces).** The original treated plasticity (1.1) as needing only a graded error (3.1). It also needs a way to route a delayed outcome back to the multi-hop choice that caused it (3.3). For a coding agent whose failures surface many steps after the bad decision, this is the difference between learning and superstition.
- **Network dynamics / async timing.** `AsyncRuntime` exists but the original report only ever discussed the sync path; concurrency, cancellation of in-flight futures (1.5/7.1), and parallel tool execution (7.1) were absent. Oscillation/timing surfaced only as deferred criticality (8.1).
- **IIT / integration measurement.** GWT (5.1) was covered; IIT was entirely absent. Practically reframed as a workspace *ignition/reach observability metric* (5.2), not mystical Φ.
- **Observability of the learning loop.** The original assumed plasticity could be added without saying how anyone would see or test what was learned (8.2). This is a cross-cutting enabler, not a nicety.
- **Persistence of learned state.** The original noted serde covers memory/workspace/modulate but never flagged that routing weights — the thing 1.1 makes worth keeping — are excluded (8.3).
- **Reproducibility of stochastic policies.** Making `exploration` load-bearing introduces randomness; the original never addressed seeded/controlled RNG (8.4), without which an agent SDK becomes untestable.
- **Provider/Tools as a first-class seam, not just a dependency.** The original declared no gaps here; for a coding SDK the sync-only, non-cancellable, string-only tool interface is itself a gap (7.1/7.2) that blocks both fan-out and the use of tool success/cost as the cheapest available learning signal.
