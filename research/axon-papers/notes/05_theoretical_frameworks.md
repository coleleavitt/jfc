# Theoretical & Computational Neuroscience — Core Frameworks

> Pure theory of *how the brain computes*, independent of AI applications. These are the candidate
> "laws" of neural computation. For Axon, the relevant takeaway is which of these are genuine
> organizing principles (worth borrowing) vs. contested (worth knowing but not betting on).

---

## 1. Free Energy Principle & Active Inference (Friston)

**Core idea.** Any self-organizing system that maintains its boundary (a Markov blanket) must
minimize **variational free energy** *F*, an information-theoretic upper bound on "surprise"
(negative log model evidence):

```
F = E_q[ln q(s) − ln p(s,o)] = D_KL[ q(s) ‖ p(s|o) ] − ln p(o)
  = (complexity) − (accuracy)        # equivalently: surprise + a non-negative KL gap
```

Because the KL term is ≥ 0, minimizing *F* makes the recognition density *q* approximate the true
posterior (**perception as Bayesian inference**) while bounding surprise. **Active inference**
extends this to action: agents minimize *expected* free energy by both updating beliefs
(perception) and *acting to change sensations* so reality matches predictions.

**Status / criticism.** Powerful and unifying, but accused of being **unfalsifiable / tautological**
(too general), and of being reducible to standard Bayesian / "control as inference" frameworks
(Watson, Imohiosen & Peters 2020). The mapping from the abstract principle to concrete neural
mechanism remains contested.

> Downloaded: `theory_free_energy_principle_observations_friston.pdf` (Friston, Da Costa, Parr 2020).

## 2. Predictive Coding / Hierarchical Bayesian Cortex (Rao & Ballard; Jiang & Rao)

**Core idea.** The cortex is a hierarchical generative model. Each level **predicts** the activity
of the level below; only the **residual prediction error** propagates upward, predictions flow
downward. Under Gaussian assumptions, inference = gradient descent on a hierarchical Bayesian
energy — a *mechanistic realization* of free-energy minimization. Distinct neuronal populations
are posited to encode **predictions** vs **errors** across cortical laminae.

**Empirical support.** Rao & Ballard (1999) explained extra-classical receptive-field effects
(end-stopping) as emergent error suppression; predicts surround suppression, repetition
suppression, and mismatch/omission responses.

**Criticism.** The exact layer/cell assignment of prediction vs error units is underdetermined;
some effects explained equally well by normalization/adaptation.

> Downloaded: `theory_predictive_coding_cortical_function_jiang_rao.pdf` (Jiang & Rao 2021) —
> the canonical modern review; `predictive_processing_recursive_condensation.pdf` — a recent
> recursive-condensation account of predictive processing.

## 3. Criticality / Self-Organized Criticality & Neuronal Avalanches (Beggs & Plenz; Chialvo)

**Core idea.** Cortical networks self-organize near a **critical point** between subcritical
(ordered/silent) and supercritical (chaotic) regimes. At criticality, activity propagates as
**neuronal avalanches** with power-law size/duration distributions (size exponent ≈ −3/2, a
critical branching process with branching parameter σ ≈ 1).

**Why it matters functionally.** Criticality is argued to **maximize dynamic range, information
transmission, and storage capacity** (Shew et al. 2009) — the brain sits at the "edge of chaos"
for optimal computation.

**Criticism (important).** Power laws can arise from **non-critical** mechanisms (subsampling,
superposed timescales), so a power-law fit alone does *not* prove criticality. Whether the brain is
truly *at* the critical point vs slightly subcritical / in a Griffiths phase is debated.

> Downloaded: `theory_criticality_in_the_brain_foundations.pdf` (Tian et al. 2023, theoretical
> foundations + how to *properly* test for criticality); `theory_neuronal_avalanches_critical_dynamics_plenz.pdf`
> (Plenz & Chialvo).

## 4. Attractor Network Theory (Hopfield; continuous/ring/line attractors)

**Core idea.** Recurrent networks store information as **stable fixed points (attractors)** of
their dynamics.
- **Hopfield network** (1982): symmetric recurrent net with an energy (Lyapunov) function whose
  local minima are stored memories; retrieval = descent to the nearest minimum →
  **content-addressable, pattern-completing** memory. **Modern/dense** Hopfield nets (Krotov &
  Hopfield 2016/2020) store exponentially many patterns and are **mathematically equivalent to
  transformer attention** (Ramsauer et al. 2020) — a direct bridge to LLMs.
- **Continuous attractors**: a continuous manifold of marginally stable states.
  - **Ring attractor** → head-direction system (demonstrated in the *Drosophila* central complex).
  - **Line attractor** → graded persistent activity = analog **working memory** & neural integration.

**Criticism.** Continuous attractors need fine-tuning, drift under noise; alternatives (feedforward
ramping, short-term facilitation, high-dim transients) can mimic the behavior. Hopfield's symmetric
connectivity is biologically unrealistic.

> Downloaded: `theory_dense_associative_memory_krotov_hopfield.pdf` (Krotov & Hopfield) —
> dense associative memory, the Hopfield↔attention bridge; `theory_continuous_attractor_networks_adaptive.pdf`
> (Li, Chu, Wu 2024) — adaptive continuous attractor dynamics; `theory_attractor_networks_free_energy_principle.pdf`
> (Spisak & Friston 2025) — self-orthogonalizing attractors *derived from* the free energy
> principle (frameworks 1 + 4 merging).

## 5. Neural Population Geometry / Low-Dimensional Manifolds (Churchland; Gallego; Langdon & Engel)

**Core idea.** Although a population has thousands of neurons, its activity lives on a
**low-dimensional manifold** (a latent "neural state space"). Computation is the **geometry and
dynamics of trajectories** on this manifold, not single-neuron tuning. Tools: PCA/jPCA, factor
analysis, dynamical-systems fits, manifold separability.

**Empirical support.**
- Motor cortex: population activity within a **preserved manifold** underlies many behaviors
  (Gallego et al. 2018); preparatory vs movement activity occupy **orthogonal** subspaces
  (Elsayed et al. / Churchland) — orthogonalization prevents interference.
- A unifying review of manifolds & circuits for cognition (Langdon, Genkin & Engel 2023).

> Downloaded: `theory_neural_manifold_motor_behaviors.pdf` (Gallego et al. 2018).
> *SDK reading:* high-dimensional module activity collapses onto a low-dim task manifold; you can
> route/represent state in far fewer dimensions than the raw neuron/feature count — and
> **orthogonal subspaces** are how the brain runs prepare-vs-execute without crosstalk.

## 6. Oscillations & Communication-through-Coherence (Fries)

**Core idea.** Neuronal groups communicate effectively only when their excitability windows are
**phase-aligned** — *communication through coherence* (CTC). Gamma (~30–80 Hz) supports local /
feedforward, beta/alpha feedback; **cross-frequency coupling** (theta-gamma) nests fast cycles in
slow ones for multiplexing and ordering (e.g. theta-gamma phase code for sequences).

**SDK reading:** a dynamic, *temporal* routing mechanism — channels open and close by rhythm/phase,
not just by static wiring. Attention reconfigures coherence to select inputs. This is "routing as a
schedule," complementary to the thalamic/BG spatial routing.

> Downloaded: `oscillations_spectral_dependence_neural_coordination.pdf` — spectral dependence as a
> framework for neural coordination; `resonant_hierarchies_oscillatory_dynamics.pdf` — multiscale
> resonant-hierarchy framework; `kuramoto_chimera_states_neural_populations.pdf` — Kuramoto
> synchronization & **chimera states** (coexisting sync/desync) in neural populations.

---

## Quick verdict for Axon (what to actually borrow)

| Framework | Borrow for the SDK? | Why |
|---|---|---|
| Predictive coding (send only the *diff* up) | **Yes** | Propagate prediction errors / deltas, not full state — cheap message bus |
| Attractor / pattern completion | **Yes (memory module)** | Content-addressable recall from partial cues = the retrieval primitive |
| Orthogonal subspaces (manifolds) | **Yes** | Keep prepare/execute (or task A/B) representations non-interfering |
| Default-deny gating (BG) | **Yes** | Safe action selection |
| Communication-through-coherence | **Concept** | Temporal/priority scheduling of channels |
| Criticality | **Know, don't bet** | Real principle, but contested & hard to verify; don't architect around it |
| Free energy principle | **Frame, not spec** | Elegant unifier, too general to implement directly |

---

### Downloaded papers for this topic
- `theory_free_energy_principle_observations_friston.pdf` — Friston, Da Costa, Parr (2020)
- `theory_predictive_coding_cortical_function_jiang_rao.pdf` — Jiang & Rao (2021)
- `predictive_processing_recursive_condensation.pdf` — Recursive condensation for predictive processing (2026)
- `theory_criticality_in_the_brain_foundations.pdf` — Tian et al., Theoretical foundations of criticality (2023)
- `theory_neuronal_avalanches_critical_dynamics_plenz.pdf` — Plenz & Chialvo, neuronal avalanches (2009)
- `theory_dense_associative_memory_krotov_hopfield.pdf` — Krotov & Hopfield, dense associative memory (2020)
- `theory_continuous_attractor_networks_adaptive.pdf` — Li, Chu & Wu, adaptive continuous attractors (2024)
- `theory_attractor_networks_free_energy_principle.pdf` — Spisak & Friston, self-orthogonalizing attractors (2025)
- `theory_neural_manifold_motor_behaviors.pdf` — Gallego et al., preserved neural manifold (2018)
- `oscillations_spectral_dependence_neural_coordination.pdf` — Spectral dependence / neural coordination (2026)
- `resonant_hierarchies_oscillatory_dynamics.pdf` — Resonant hierarchies, multiscale oscillations (2026)
- `kuramoto_chimera_states_neural_populations.pdf` — Kuramoto dynamics & chimera states (2026)

### Canonical references
- Friston (2010), *The free-energy principle: a unified brain theory?*, Nat. Rev. Neurosci.
- Rao & Ballard (1999), *Predictive coding in the visual cortex*, Nat. Neurosci.
- Beggs & Plenz (2003), *Neuronal avalanches in neocortical circuits*, J. Neurosci.
- Hopfield (1982), PNAS; Ramsauer et al. (2020), *Hopfield Networks is All You Need*.
- Vyas, Golub, Sussillo & Shenoy (2020), *Computation through neural population dynamics*, Annu. Rev. Neurosci.
- Fries (2005, 2015), communication through coherence, Trends Cogn. Sci. / Neuron.
