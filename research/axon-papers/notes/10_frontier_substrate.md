<!-- RFC / design record for the "different substrate" frontier (audit items 8–11). -->

# RFC: the substrate frontier — phase coding, dendrites, and what we deliberately don't chase

After the gap analysis (note 09) and the seven "biologically deeper" frontier
mechanisms (sleep-replay, precision-weighting, hierarchical predictive coding,
synaptic tagging & metaplasticity, homeostatic scaling, active inference,
ACh encode/recall), one cluster remained: mechanisms the corpus describes that
assume a **continuous, distributed, plastic-topology** neural substrate. Axon is
deliberately a **discrete, typed, fixed-graph router**. This note records which
of those we adopted, how, and why we stop where we stop.

The governing rule is unchanged: the core stays small, deterministic, and
dependency-free; new ideas enter as idiomatic, zero-when-unused additions (a new
gate, a new signal field, an optional module), never as changes to the routing
invariants. An idea is only worth adopting if it maps to the substrate *without
distortion*. Two of the four do; two do not.

## Adopted

### #8 — Phase coding (`Phase` + `PhaseGate`) — built

**Biology:** communication-through-coherence — neuronal groups exchange
information only when their rhythms are phase-aligned; theta-gamma nesting
multiplexes sequences.

**Why a clean fit, discretely:** axon has no continuous time, so real
oscillations don't map. But the *functional* principle — channels open and close
on a schedule rather than by static wiring — maps perfectly to a discrete phase
tag. A `Signal` now carries a `Phase` (default 0, preserved across `map`), and a
`PhaseGate` admits a signal only in its active phases. One fixed graph therefore
multiplexes different routes per phase without rewiring. This lives in the core
gate module beside `MinPriority`/`Disinhibit`, because that is exactly where
axon's gates live; it has zero impact when unused. (A wrapper-`Runtime`
"`axon-phase`" crate was considered and rejected — it duplicates the driver and
breaks from axon's established gate pattern.)

**Not built:** continuous oscillators, cross-frequency coupling, phase-precession
learning. A future `Runtime` could *advance* a phase per step and stamp emitted
signals (a phase scheduler); today the caller sets the phase, which is enough to
demonstrate multiplexing.

### #9 — Dendritic computation (`CompartmentModule`) — built

**Biology:** a pyramidal neuron is not a simple function; its dendritic branches
perform local nonlinear (coincidence / XOR-like) operations, so one cell behaves
like a small multi-layer network.

**Why a clean fit:** this is entirely *module-internal* — it never touches
routing. `CompartmentModule` is a `Module` whose firing is a nonlinear function
of several branch predicates: it passes the signal on only when at least
`threshold` branches fire (coincidence detection), else drops it. Module authors
opt in for richer leaf computation than `FnModule`; the core stays unaware.

**Not built:** branch-specific plasticity (per-branch learned weights), active
dendritic spikes with their own dynamics. Those are a deeper module-internal
concern that no current use case needs.

## Deliberately not chased (different substrate)

### #10 — Mixed selectivity / population codes / manifold attractors — not built

**Biology:** cortical information lives in the *distributed* activity across many
neurons (a point on a low-dimensional manifold); single neurons are mixed-
selective; computation is attractor dynamics over that population.

**Why it doesn't fit:** axon routes **typed schema signals** by identity, not
dense population vectors evolving on a manifold. Adopting this wholesale means a
different representation and a different (continuous, recurrent) dynamics — it
would not be axon anymore. **Already partially covered:** `axon-memory` has a
`HashEmbedder` (dense `Vec<f32>`) with cosine similarity recall, and fan-out
(`select_all`) plus an aggregator gives distributed/redundant voting. If a real
need arises, the right increment is an optional `Embedding` signal type and a
consensus readout module — *not* converting the router to manifold dynamics.

### #11 — Adult neurogenesis / runtime module birth-death — tractable form built

**Biology:** the adult brain grows new neurons (dentate gyrus, for pattern
separation) and prunes others — capacity added on demand without catastrophic
interference.

**The headline mechanism doesn't fit:** axon's modules are compiled Rust code
wired at startup; runtime "birth" means instantiating and retiring computational
units at runtime, which implies a sandboxed plugin host (WASM/process isolates),
serialization, versioning, and a security model — a large surface that fights the
deterministic, statically-reasoned design. That stays out until a concrete need
justifies the cost.

**But its *purpose* — acquiring new capability on demand — has a clean tractable
form, now built:**
- *Skill acquisition* — `SkillLibrary` (axon-exec) wires the
  `ProceduralStore` into the loop: a plan that worked is promoted
  (`learn(goal, &plan)`) and recalled (`recall`/`recall_similar`) as a runnable
  plan, so the agent reuses a known-good skill instead of re-planning. Plans
  round-trip to procedural memory via `Plan::to_procedure` / `from_procedure`.
  Opt-in; pays off when goals recur.
- *Pattern separation* (the dentate gyrus's job) is handled by near-duplicate
  orthogonalization on memory encode.
- *Virtual nodes* (shared implementation, independent learned state) already
  exist: learned weight is **per-edge**, so two routes to the same module learn
  independently.

So #11 is adopted in the form that fits the substrate (a growing skill library),
and declined in the form that doesn't (runtime code-birth). #10 remains the one
genuinely deferred item.

## Recommendation

Keep the core as it is. Three of the four frontier ideas are now in, each as the
form that fits the substrate: #8 phase coding (`PhaseGate`), #9 dendritic
computation (`CompartmentModule`), and #11's tractable form — skill acquisition
(`SkillLibrary`). In every case the *headline* mechanism that would need a
different substrate (continuous oscillations, active dendritic dynamics, runtime
code-birth) is declined, while its purpose is captured by a discrete, optional,
deterministic addition.

Only **#10** (dense population/manifold codes) remains genuinely deferred: it
needs a different *representation*, not just a different mechanism, and its
purpose is partly covered (memory embeddings; fan-out voting). Revisit it only
with a concrete workload the approximations demonstrably fail to serve — and even
then as an optional `Embedding` signal + consensus readout, never by turning the
typed router into manifold dynamics.
