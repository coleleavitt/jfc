# Brain Anatomy & Cell Types — How Each Part Works

> The "parts list." Every major region and cell class, what it does, and the mechanism. This is the
> ground-truth catalogue of modules and cell types that the architectural metaphors abstract from.

---

## 1. Neurons — types & morphology

- **Pyramidal cells** — principal **excitatory (glutamatergic)** projection neurons of cortex:
  triangular soma, one apical dendrite to the pia, basal dendrites, dense spines, long myelinated
  axon. The cortex's output and long-range communicators.
- **Interneurons** — mostly **GABAergic**, locally projecting, aspiny. Three largely
  non-overlapping molecular classes:
  - **PV (parvalbumin)** — fast-spiking basket/chandelier cells; **perisomatic** inhibition;
    set gamma rhythm and gain; precise timing.
  - **SST (somatostatin)** — target **distal dendrites**; control dendritic integration & input.
  - **VIP (vasoactive intestinal peptide)** — **disinhibitory**: inhibit SST/PV → "open the gate"
    for top-down/attentional control. (A built-in disinhibitory routing switch.)
- **Spiny stellate cells** — excitatory input cells of L4 (thalamorecipient).

**Transcriptomic taxonomy (BRAIN Initiative Cell Census).** Modern cell types are defined by
single-cell/nucleus RNA-seq + chromatin accessibility, organized hierarchically
(**class → subclass → supertype → cluster**), not by morphology alone — thousands of molecularly
distinct types.

> Downloaded: `cells_multimodal_spatial_atlas_neuron_types.pdf` — a multimodal spatial atlas
> integrating transcriptomic, morphological, and electrophysiological identity of neurons.

## 2. Glia — the other half of the brain

- **Astrocytes** — the **tripartite synapse**: perisynaptic astrocyte processes sense
  neurotransmitter, buffer K⁺, clear glutamate (EAAT1/2), and release gliotransmitters that
  modulate synaptic strength. They tile the brain into domains and provide metabolic support;
  recent work shows **higher-order, network-level control of synaptic plasticity** by astrocytes.
- **Microglia** — resident immune cells; **synaptic pruning** during development & disease.
  Complement protein **C1q tags weak synapses** for elimination; **phosphatidylserine** externalized
  on synapses is an "eat-me" signal; the same machinery drives early synapse loss in Alzheimer's
  (Stevens lab, Hong et al.).
- **Oligodendrocytes** — produce CNS **myelin**; **activity-dependent myelination** makes the wiring
  plastic (tuned by experience/learning). See `01_axon_signal_transmission.md`.

> Downloaded: `glia_astrocyte_higher_order_synaptic_plasticity.pdf` — astrocyte higher-order control
> of plasticity; `oligodendrocyte_activity_dependent_myelination.pdf`.
> *SDK reading: "supporting infrastructure" is not passive — astrocytes/microglia actively tune and
> prune the network. The analog: background processes that GC dead edges and tune weights.*

## 3. Cerebellum — internal models for prediction & control

The cerebellum learns **internal models** of the body & world:
- **Forward model**: predicts the sensory consequences of a motor command (efference copy →
  predicted state) → corrects faster than feedback delays allow.
- **Inverse model**: computes the commands needed for a desired outcome.
- Circuit: granule/parallel fibers (context) × Purkinje cells, with **climbing-fiber "teaching
  signals"** from the inferior olive driving **LTD** → supervised error-correction learning.

> Downloaded: `cerebellum_consensus_models_functions.pdf` — consensus paper on models of cerebellar
> function (forward/inverse models, timing, beyond motor → cognition).
> *SDK reading: a dedicated "predictor/verifier" module that learns to anticipate outcomes and
> correct before slow feedback returns — a fast self-correction loop.*

## 4. Basal ganglia & dopamine — reward-prediction-error learning

Midbrain dopamine neurons (SNc, VTA) encode a **reward prediction error (RPE)** — the gap between
received and predicted reward (Schultz):
- Unpredicted reward → phasic **burst**.
- A reward-predicting cue → burst shifts **to the cue**.
- Omitted expected reward → **dip** below baseline.

This δ signal *is* the temporal-difference error of RL, and it trains cortico-striatal synapses
(direct/indirect pathways). See routing/gating in `04_routing_basal_ganglia_thalamus.md`.

> Downloaded: `dopamine_prediction_errors_history.pdf` — a brief history of dopamine prediction
> errors; `vta_reward_addiction_review.pdf` — VTA in reward & addiction.
> *SDK reading: a global scalar "this went better/worse than expected" signal that gates which
> weights/decisions get reinforced — the learning signal for adaptive routing.*

## 5. Amygdala & limbic system — salience & fear

The amygdala (lateral/basolateral = input + plasticity; central = output) drives **Pavlovian fear
conditioning**: convergence of conditioned (CS) and unconditioned (US) stimuli onto lateral
amygdala neurons → NMDA-dependent LTP → learned fear. Broadly: a **salience/threat tagger** that
prioritizes and gates attention & memory toward emotionally significant events.

> *SDK reading: a fast "salience/priority" tagger that can preempt normal routing for
> high-importance signals (an interrupt with priority).*

## 6. Cerebral cortex — laminae, microcircuit, columns

- **Six layers**: L1 (neuropil), L2/3 (cortico-cortical pyramidal), L4 (granular, thalamorecipient),
  L5 (subcortical-projecting pyramidal), L6 (cortico-thalamic feedback).
- **Canonical microcircuit** (Douglas–Martin): thalamus → L4 → L2/3 → L5/6, with strong recurrent
  excitation **balanced** by feedforward/feedback inhibition.
- **Cortical columns** (Mountcastle): vertically aligned neurons sharing response properties — a
  proposed repeated, modular "compute unit" tiled across cortex.
- **E/I balance**: excitation tightly tracked by inhibition; its breakdown → seizures, dysfunction.

> Downloaded: `cortical_microcircuit_interneuron_oscillations.pdf` — multi-interneuron cortical
> networks generating oscillatory patterns.
> *SDK reading: a **repeated canonical module** (the column) wired in a stereotyped internal
> pattern, tiled and specialized by its inputs — the literal "Lego brick" Cole described.*

## 7. Neuromodulatory systems — global state/config

Four diffuse, broadcast systems that change the **gain/state** of whole circuits (not point
messages):
- **Dopamine** (VTA/SNc) — reward, motivation, gating, learning rate.
- **Serotonin** (raphe) — mood, patience/time horizon, behavioral flexibility.
- **Acetylcholine** (basal forebrain) — attention, encoding vs recall mode, signal-to-noise.
- **Noradrenaline** (locus coeruleus) — arousal, the **exploration/exploitation** switch, network
  reset/reorganization.

> *SDK reading: these are **global config/feature flags** that retune all modules at once —
> learning rate, exploration temperature, attention gain, encode-vs-recall mode. A small set of
> scalar "mode" knobs that change system behavior without rewiring.*

## 8. Support systems — neurovascular unit, BBB, neurogenesis

- **Neurovascular unit / blood-brain barrier**: neurons + astrocytes + endothelium + pericytes
  couple activity to blood flow (neurovascular coupling) and gate what enters the brain. Failure
  is an upstream driver of neurodegeneration.
- **Adult neurogenesis**: ongoing new-neuron production in the **dentate gyrus** (hippocampus) and
  SVZ — supports pattern separation, new learning, mood; modulated by environment/exercise.

> Downloaded: `neurovascular_unit_bbb_failure.pdf`, `adult_neurogenesis_social_behavior.pdf`.
> *SDK reading: resourcing/admission control (BBB = what's allowed into the core) and the ability
> to **grow new modules** at runtime (neurogenesis) rather than only retraining fixed ones.*

---

### Downloaded papers for this topic
- `cells_multimodal_spatial_atlas_neuron_types.pdf` — Multimodal spatial atlas of neuron types (2026)
- `glia_astrocyte_higher_order_synaptic_plasticity.pdf` — Astrocyte higher-order control of plasticity (2026)
- `oligodendrocyte_activity_dependent_myelination.pdf` — Activity-dependent myelination (2026)
- `cerebellum_consensus_models_functions.pdf` — Consensus: models of cerebellar function (2026)
- `dopamine_prediction_errors_history.pdf` — A brief history of dopamine prediction errors (2026)
- `vta_reward_addiction_review.pdf` — VTA in reward & addiction (2026)
- `cortical_microcircuit_interneuron_oscillations.pdf` — Cortical interneuron networks & oscillations (2025)
- `neurovascular_unit_bbb_failure.pdf` — Neurovascular unit / BBB failure (2026)
- `adult_neurogenesis_social_behavior.pdf` — Adult neurogenesis & social behavior (2026)

### Canonical references
- BRAIN Initiative Cell Census Network (BICCN, 2021/2023), Nature suite.
- Araque et al. (1999), *Tripartite synapses*, Trends Neurosci.
- Stevens et al. (2007), *The classical complement cascade mediates CNS synapse elimination*, Cell.
- Wolpert, Miall & Kawato (1998), *Internal models in the cerebellum*, Trends Cogn. Sci.
- Schultz, Dayan & Montague (1997), *A neural substrate of prediction and reward*, Science.
- LeDoux (2000), *Emotion circuits in the brain*, Annu. Rev. Neurosci.
- Douglas & Martin (2004), *Neuronal circuits of the neocortex*, Annu. Rev. Neurosci.
- Mountcastle (1997), *The columnar organization of the neocortex*, Brain.
