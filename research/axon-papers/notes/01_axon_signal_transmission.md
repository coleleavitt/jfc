# Axons & Synapses — Neural Signal Transmission

> **Axon SDK mapping:** the *transmission/routing substrate*. Axons are the wires; synapses
> are the typed, weighted, plastic junctions. In the SDK this is the typed message bus and the
> connection weights between modules. The name **Axon** is taken from this: the part that does
> not think and does not store — it *transmits a signal between modules with precision*.

---

## 1. What an axon does

A neuron integrates inputs on its dendrites and soma, and when the membrane potential at the
**axon hillock / initial segment** crosses threshold (~−55 mV) it fires an **action potential**
(AP) — an all-or-none ~100 mV depolarizing spike that propagates down the axon without
attenuation. The axon is therefore a *digital, regenerative transmission line*: amplitude is
fixed, information is carried in **spike timing and rate**, not amplitude.

Key biophysics:
- **Resting potential** (~−70 mV) maintained by the Na⁺/K⁺ ATPase and K⁺ leak channels.
- **Depolarization** opens voltage-gated Na⁺ channels (fast inward current) → upstroke.
- **Repolarization**: Na⁺ channels inactivate, voltage-gated K⁺ channels open (outward current).
- **Refractory period** enforces unidirectional propagation and caps maximum firing rate.

## 2. Saltatory conduction & myelination

In vertebrates, **oligodendrocytes** (CNS) and **Schwann cells** (PNS) wrap axons in **myelin**,
a multilamellar lipid insulator. Myelin is interrupted at **nodes of Ranvier**, where Na⁺
channels cluster. The AP "jumps" node-to-node — **saltatory conduction** — which:
- increases conduction velocity by ~10–100× (up to ~120 m/s in large myelinated fibers),
- dramatically reduces the metabolic cost per signal (fewer ions to pump back).

> Downloaded: `saltatory_axonal_conduction_retina.pdf` — direct evidence of saltatory conduction
> in a CNS circuit (avian retina); `axon_myelin_unit_metabolic.pdf` — the **axon–myelin unit** as a
> metabolic symbiosis (oligodendrocytes supply lactate to axons), and its vulnerability;
> `axon_neurobiology_editorial.pdf` — overview of functional/structural axon dynamics;
> `oligodendrocyte_activity_dependent_myelination.pdf` — **activity-dependent myelination**:
> myelin is *plastic*, tuned by neural activity and learning (a substrate for adaptive routing).

**SDK reading:** conduction velocity ≈ channel latency; myelination ≈ caching / a fast-path that
the system tunes by usage. Activity-dependent myelination is the biological analogue of a router
that strengthens hot paths.

## 3. The synapse — a typed, weighted, plastic junction

The axon terminal forms a **synapse** onto a target. Signal crosses as neurotransmitter:
- **Excitatory**: glutamate → AMPA/NMDA receptors → depolarization (EPSP).
- **Inhibitory**: GABA → GABA_A/B receptors → hyperpolarization (IPSP).
- **Neuromodulatory**: dopamine, serotonin, ACh, noradrenaline — slow, broadcast, change the
  *gain/state* of whole circuits rather than carrying point-to-point messages.

This is the crucial architectural point: synapses are **typed channels** (the neurotransmitter +
receptor define the message type) with a **learnable weight** (synaptic strength) and a **sign**
(excitatory/inhibitory). The brain's "routing table" is literally the connectivity matrix of these
typed, signed, weighted edges.

### Synaptic plasticity = learning the weights
- **LTP / LTD** (long-term potentiation/depression): activity-dependent strengthening/weakening.
- **Hebbian rule**: "cells that fire together wire together"; **STDP** refines this by spike timing.
- Plasticity is gated by neuromodulators (e.g. dopamine as a global "save this weight" signal).

## 4. Why this is the right name & core metaphor for the SDK

| Neuroscience | Axon SDK |
|---|---|
| Action potential (all-or-none) | a typed message/event |
| Axon | the transmission channel between modules |
| Node of Ranvier / saltatory conduction | fast-path / hot-path routing, caching |
| Synapse (NT + receptor) | a typed connection contract |
| Synaptic weight & sign | edge weight / enable–suppress in the routing table |
| Neuromodulation | global context/config that changes module gain, not point messages |
| Plasticity (LTP/LTD, STDP) | learned/adaptive routing weights |

The core SDK should *only* be this layer: typed messages flowing over typed, weighted edges. No
cognition, no memory, no UI — exactly what an axon is.

---

### Downloaded papers for this topic
- `saltatory_axonal_conduction_retina.pdf` — Saltatory axonal conduction in the avian retina (2025)
- `axon_myelin_unit_metabolic.pdf` — Metabolic symbiosis & vulnerability in the CNS axon–myelin unit (2026)
- `axon_neurobiology_editorial.pdf` — Editorial: Axon neurobiology — functional & structural dynamics (2026)
- `oligodendrocyte_activity_dependent_myelination.pdf` — Cognitive stimulation & activity-dependent myelination (2026)
- `hodgkin_huxley_lie_group_membrane_architecture.pdf` — On the Hodgkin–Huxley equation's structure (see `06_biophysics_dynamics.md`)
