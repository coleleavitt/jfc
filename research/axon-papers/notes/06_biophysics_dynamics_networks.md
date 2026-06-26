# Biophysics, Dynamics & Network Neuroscience

> The substrate level (single neuron → whole-brain graph). How a neuron computes physically, and
> how billions of them form a network with measurable topology. Less directly mappable to the SDK,
> but it's the ground truth "how the brain works" the rest sits on.

---

## 1. The Hodgkin–Huxley model & cable theory (single-neuron biophysics)

**Hodgkin–Huxley (1952).** The Nobel-winning quantitative model of the action potential. Membrane
current is the sum of capacitive + ionic currents:

```
C_m dV/dt = I_ext − [ g_Na m³h (V−E_Na) + g_K n⁴ (V−E_K) + g_L (V−E_L) ]
```

where `m, h, n` are voltage- and time-dependent **gating variables** (activation/inactivation of
Na⁺ and K⁺ channels) each obeying first-order kinetics. This is a 4-D nonlinear dynamical system
whose limit-cycle/threshold behavior *is* the spike. It launched **computational neuroscience**:
the neuron is a well-defined dynamical system, not a magic black box.

**Cable theory.** Treats dendrites/axons as leaky electrical cables; the **cable equation**
`λ² ∂²V/∂x² = τ ∂V/∂t + V` governs how potentials attenuate and delay along the neurite (space
constant λ, time constant τ). This sets *which* inputs can interact and how dendritic geometry
shapes integration.

> Downloaded: `hodgkin_huxley_lie_group_membrane_architecture.pdf` — a modern look at *why* the HH
> equation has the structure it does (Lie-group view of membrane architecture).
>
> **Reduced models** (worth knowing): integrate-and-fire, FitzHugh–Nagumo (2-D reduction of HH),
> Izhikevich (rich spiking with 2 variables) — the "good enough" abstractions used in large nets,
> analogous to picking the right fidelity for a simulation.

## 2. Dendritic computation

Dendrites are **not** passive summers. They host nonlinear events — NMDA spikes, Ca²⁺ and Na⁺
dendritic spikes — so a single pyramidal neuron behaves like a **multi-layer network**: each
dendritic branch is a nonlinear subunit, and the soma sums their outputs. Implications:
- A single neuron can compute nonlinear functions (e.g. XOR-like, coincidence detection) once
  thought to need a network.
- **Coincidence detection** and **branch-specific plasticity** vastly increase the effective
  capacity per cell.

> Downloaded: `primary_cilia_neural_computation.pdf` — primary cilia as an under-appreciated
> signaling/compute compartment of the neuron (the cell has more compute surfaces than the synapse).
> *SDK reading: even the "leaf" module is internally nonlinear and structured — don't assume the
> smallest unit is a dumb function.*

## 3. Connectome & network neuroscience (graph theory; Bullmore & Sporns)

Model the brain as a **graph**: nodes = regions/neurons, edges = structural (axonal tracts) or
functional (correlated activity) connections. Robust topological signatures:

- **Small-world**: high local clustering + short path length → efficient local *and* global
  communication at low wiring cost (Watts–Strogatz applied to brains).
- **Hubs & rich club**: a small set of highly connected hub regions are densely interconnected
  with each other → a high-capacity backbone (and a vulnerability: hub damage is costly).
- **Modularity with integration**: communities (modules) for specialized processing, linked by
  **connector hubs** for integration — *exactly* the "heavily modular but routed" architecture.
- **Cost–efficiency trade-off**: topology is near-optimal for communication efficiency per unit
  wiring cost.

> Downloaded: `network_general_intelligence_connectome.pdf` — network architecture of general
> intelligence in the human connectome; `multiscale_dynamic_causal_models_brain.pdf` — multiscale
> parcellation / dynamic causal models (effective connectivity & how to route causally).
>
> **SDK reading (this is the big one for Axon):** the brain *is* a modular graph with connector
> hubs and a rich-club backbone. The architecture ismeth describes — modular cores plugged into a
> routing plane — is literally the connectome's small-world + rich-club + modularity organization.

## 4. Whole-brain dynamics: synchronization, metastability, chimeras

- **Kuramoto model**: phase oscillators coupled on the connectome → captures large-scale
  synchronization. The brain operates in a **metastable** regime: never fully synchronized nor
  fully incoherent, but wandering between transient partially-synchronized states.
- **Chimera states**: coexistence of synchronized and desynchronized subpopulations in the *same*
  network — a candidate substrate for simultaneous integration + segregation.
- Metastability gives **flexible, reconfigurable** functional connectivity on a fixed structural
  graph — the same wiring supports many transient routing configurations.

> Downloaded: `kuramoto_chimera_states_neural_populations.pdf` — Kuramoto dynamics & origins of
> chimera states. *SDK reading: a fixed module graph can yield many dynamic routing states; the
> "route" is a transient dynamical configuration, not just static wiring.*

## 5. Pathology as architecture failure (network medicine)

How the architecture breaks is itself informative:
- **Neurodegeneration spreads along the connectome** (prion-like proteinopathy follows network
  topology; hubs are hit hard — "network degeneration hypothesis"). Tau in Alzheimer's
  preferentially accrues in the **default-mode network**.
- **Epilepsy** = a dynamical/criticality failure: pathological hypersynchrony; "seizures beget
  seizures" (kindling) — a positive-feedback routing failure.
- **Multifactorial computational models** integrate many failure factors to predict
  neurodegeneration trajectories.

> Downloaded: `tau_pathology_default_mode_network_alzheimers.pdf`,
> `multifactorial_computational_models_neurodegeneration.pdf`, `seizures_beget_seizures_kindling.pdf`,
> `neurovascular_unit_bbb_failure.pdf` (BBB/neurovascular-unit failure as an upstream driver).
> *SDK reading: failures propagate along the routing graph and concentrate at hubs — design for
> hub robustness, backpressure, and circuit-breakers, or one module's failure cascades.*

---

### Downloaded papers for this topic
- `hodgkin_huxley_lie_group_membrane_architecture.pdf` — HH equation structure / membrane architecture (2026)
- `primary_cilia_neural_computation.pdf` — Primary cilia and neural computation (2026)
- `network_general_intelligence_connectome.pdf` — Network architecture of general intelligence in the connectome (2026)
- `multiscale_dynamic_causal_models_brain.pdf` — Multiscale parcellation of dynamic causal models (2026)
- `kuramoto_chimera_states_neural_populations.pdf` — Kuramoto dynamics & chimera states (2026)
- `tau_pathology_default_mode_network_alzheimers.pdf` — Tau pathology in the default-mode network (2026)
- `multifactorial_computational_models_neurodegeneration.pdf` — Multi-factorial computational models of neurodegeneration (2024)
- `seizures_beget_seizures_kindling.pdf` — The cumulative impact of seizures / kindling (2026)
- `neurovascular_unit_bbb_failure.pdf` — Blood–brain barrier breakdown & neurovascular unit failure (2026)

### Canonical references
- Hodgkin & Huxley (1952), J. Physiol.
- Rall, cable theory of dendrites.
- London & Häusser (2005), *Dendritic computation*, Annu. Rev. Neurosci.
- Bullmore & Sporns (2009), *Complex brain networks: graph theoretical analysis*, Nat. Rev. Neurosci.
- van den Heuvel & Sporns (2011), *Rich-club organization of the human connectome*, J. Neurosci.
- Deco, Tononi, Kringelbach (2015), metastability & whole-brain dynamics.
