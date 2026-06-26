# Axon — Neuroscience → Architecture Synthesis

> The one-page map. Axon is an agentic-coding SDK whose architecture is taken, part-for-part, from
> how the brain is organized: a thin transmission/routing core, with cognition, memory, perception,
> and UI as separately pluggable, typed modules. This doc ties the seven research notes to a concrete
> SDK shape. It is a *design rationale grounded in neuroscience* — not an implementation spec, and
> not code.

---

## The thesis (ismeth + Cole, restated in brain terms)

> "Actual architecture is in the data and routing plane. The UI part is pluggable to anything."

The brain agrees. The **conserved, central** thing is the **routing/gating plane** (thalamus +
basal ganglia) and the **transmission substrate** (axons + typed synapses). The "applications" —
cortical areas, and ultimately the body/UI — are **plugged on top** and are the part that varies.
JFC died because it conflated the router, the executor, the memory, and the UI into one monolith.
The brain never does this. **Neither should Axon.**

## The layer map

| Brain system | Function | Axon module | Research note |
|---|---|---|---|
| **Axons + synapses** | typed, weighted, plastic signal transmission | **core: typed message bus + edge weights** | `01` |
| **Thalamus + basal ganglia** | route + gate (default-deny action selection) | **core: routing table + dispatcher/gate** | `04` |
| **Prefrontal cortex** | plan, decompose, hold task context, monitor, gate | **Alfonso: executive/planner (built last)** | `02` |
| **Hippocampus** | encode → consolidate → recall; dual fast/slow store | **memory module (capture/dream/recall)** | `03` |
| **Cerebellum** | forward/inverse internal models; fast self-correction | **predictor/verifier module** | `07` |
| **Sensorimotor cortex** | perceive structure, act precisely | **tools module (typed, stateless I/O)** | `07` |
| **Neuromodulators** | global state: learning rate, exploration, attention | **global config / mode knobs** | `07` |
| **Cortical column** | repeated canonical compute unit, tiled & specialized | **the module template ("Lego brick")** | `07` |
| **Global workspace** | bounded broadcast of salient info | **bounded shared context bus** | `08` |
| **Connectome** | modular graph, connector hubs, rich-club backbone | **the module graph + routing backbone** | `06` |

## The core (and only the core)

Per the PFC literature ("executive dysfunction = the coordinator trying to also execute") and the
connectome (routing plane is separate from cortical computation), the **Axon core owns exactly
three primitives and nothing else**:

```
core = {
  typed message schema   # what a signal is              (axon = action potential)
  routing table          # where a signal goes           (thalamus relay + TRN priority)
  execution graph        # what fires, and in what order  (basal-ganglia gating, default-deny)
}
```

**Hard constraint (from every framework):** the core makes **no LLM calls**, holds **no memory**,
renders **no UI**. The moment an LLM call enters the core, you've rebuilt JFC. LLM calls live only
in modules (planner, memory historian/dreamer, tool agents) that plug into the core.

## Principles worth stealing (and which to ignore)

**Borrow:**
1. **Default-deny gating** (basal ganglia): nothing fires unless explicitly released; pick exactly
   one action, suppress competitors → safe-by-construction dispatch.
2. **Send the diff, not the state** (predictive coding): modules propagate *prediction errors* /
   deltas upward, predictions downward → a cheap, hierarchical message bus.
3. **Content-addressable recall from partial cues** (CA3 / Hopfield attractors): the memory
   retrieval primitive.
4. **Pattern separation on write** (dentate gyrus): dedup/orthogonalize memories on ingest.
5. **Orthogonal subspaces** (neural manifolds): keep prepare/execute or task-A/task-B
   representations non-interfering.
6. **Dual fast/slow memory** (complementary learning systems): a fast episodic store + a slow
   schema store + an offline consolidator — avoids catastrophic forgetting.
7. **Bounded broadcast workspace** (GWT): integration via a *capacity-limited* shared bus.
8. **Global mode knobs** (neuromodulation): a few scalars (learning rate, exploration temperature,
   attention gain, encode-vs-recall) that retune all modules without rewiring.
9. **Hub robustness / circuit breakers** (network medicine): failures propagate along the graph and
   concentrate at hubs — design backpressure and isolation so one module can't cascade.
10. **Repeated canonical module** (cortical column): one well-defined module template, tiled and
    specialized by its inputs — the literal Lego brick.

**Know but don't architect around:**
- **Criticality / edge-of-chaos** — real principle, but contested and hard to verify; don't bet the
  architecture on it.
- **Free energy principle** — beautiful unifier, too general to be an implementation spec; use it as
  a framing, not a contract.

## Build order (week-one scope, per CoALA + the "start small" lesson)

1. **Core** — typed message schema + routing table + execution graph. No LLM, no memory, no UI.
2. **One tool module** — typed, stateless perception/action (e.g. filesystem or shell).
3. **One agent loop** — a single plan→act→observe cycle plugged into the core.
4. *Then* **memory module** (capture→consolidate→recall, MemGPT-style pager + GenAgents-style
   scored retrieval).
5. *Last* **Alfonso** (the planner/executive) — reads context from memory, writes plans to the
   graph. Built only after core + memory are bulletproof.

> Keep it small until the three core primitives are bulletproof, then expand depth-first — not
> breadth-first bolting-on (that is precisely what killed JFC).

## Why "Axon" is the right name

The axon is the part of the neuron that does not think and does not store — it **transmits a signal
between modules with precision**. That is exactly what the core is. Everything cognitive (PFC),
mnemonic (hippocampus), perceptual (sensory cortex), and motor (motor cortex) plugs onto that
transmission/routing substrate. The name encodes the architectural discipline: *the core only
routes.*

---

### Read next
- `01_axon_signal_transmission.md` — the core/transmission metaphor in depth
- `02_prefrontal_cortex_executive_control.md` — Alfonso / the planner
- `03_hippocampus_memory_systems.md` — the memory module
- `04_routing_basal_ganglia_thalamus.md` — the routing/gating plane
- `05_theoretical_frameworks.md` — the candidate "laws" of neural computation
- `06_biophysics_dynamics_networks.md` — substrate: neuron biophysics → connectome graph
- `07_brain_anatomy_cell_types.md` — the full parts list
- `08_brain_inspired_ai_architectures.md` — GWT, Fodor, SOAR/ACT-R, CoALA, MemGPT, Generative Agents
