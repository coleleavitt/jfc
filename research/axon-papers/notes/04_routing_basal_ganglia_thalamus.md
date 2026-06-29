# Thalamus & Basal Ganglia — The Routing / Gating Plane

> **Axon SDK mapping:** the *routing & action-selection plane* — the part ismeth called "the
> actual architecture… in the data and routing plane." The **thalamus** is the relay/router
> through which cortical loops pass; the **basal ganglia** are the gate that selects one action
> and suppresses competitors. In the SDK: the dispatcher that decides which tool/agent/module
> fires, and the gate that admits or blocks a candidate action.

---

## 1. Thalamus — the relay and router

The thalamus is the brain's central switchboard: nearly all sensory and motor information (except
smell) is **relayed** through thalamic nuclei to cortex, and cortico-cortical communication is
itself routed **trans-thalamically** (cortex → higher-order thalamus → cortex).

- **First-order nuclei** relay sensory input to cortex (e.g. LGN → V1).
- **Higher-order nuclei** (e.g. pulvinar, MD) relay *between* cortical areas — a **transthalamic
  pathway** that runs in parallel with direct cortico-cortical connections.
- The **thalamic reticular nucleus (TRN)** is an inhibitory shell that acts as an **attentional
  gate / spotlight**, suppressing irrelevant channels.

> Downloaded: `transthalamic_pathways_perception.pdf` — the role of transthalamic pathways in
> perception: the thalamus isn't a passive relay; it actively routes and gates cortical
> communication. *SDK reading: messages between modules don't go peer-to-peer by default — they
> pass through a router that can gate, prioritize, and modulate them (the TRN = attention/priority).*

## 2. Basal ganglia — action selection & gating

The basal ganglia (striatum, globus pallidus, subthalamic nucleus, substantia nigra) implement
**action selection**: many candidate actions compete; the BG **disinhibit** the winner and keep
the rest suppressed. The default state is *tonic inhibition* of thalamus/brainstem; selecting an
action means *releasing* that inhibition for one channel only.

Two opposing pathways:
- **Direct ("Go") pathway** → disinhibits thalamus → **promotes** the selected action.
- **Indirect ("NoGo") pathway** → increases inhibition → **suppresses** competing actions.
- **Hyperdirect** (via STN) → fast global "hold everything" brake.

**Dopamine** sets the balance: it potentiates Go and depresses NoGo (D1 vs D2 receptors), biasing
toward action when reward is expected (see `08_neuromodulation_reward.md`).

### The gating metaphor (Frank & Badre)
Hierarchical RL in cortico-striatal loops: PFC holds candidate rules/representations; the basal
ganglia provide a **gating signal** deciding *when* to update working memory and *which* action to
release. This is a clean **planner (PFC) + dispatcher/gate (BG) + router (thalamus)** decomposition.

> Downloaded: `basal_ganglia_parkinsons_computational_model.pdf` — computational models of the
> basal ganglia across scales (action-selection circuitry, direct/indirect pathway dynamics,
> what breaks in Parkinson's = a gating disorder).

## 3. Why routing/gating is the real architecture

ismeth: *"Actual architecture is in the data and routing plane. UI part is pluggable to anything."*
The brain agrees: the thalamus+BG routing/gating plane is conserved and central; the cortical
"applications" plugged on top are what vary. Three properties worth stealing:

1. **Default-deny gating.** Nothing fires unless explicitly released. Safe by construction — the
   system suppresses all candidate actions and disinhibits exactly one. (Great model for a tool
   dispatcher that must pick exactly one action and block the rest.)
2. **Routing is separate from computation.** The thalamus routes; the cortex computes. Keep the
   router dumb and declarative; keep cognition in the modules.
3. **Modulated routing.** The TRN/attention and dopamine change *which* channels are open and with
   what gain — routing is reconfigurable by global state, not hardwired.

## 4. Mapping table

| Structure | Function | Axon SDK analog |
|---|---|---|
| Thalamus (relay) | route signals between cortical areas | message router / dispatcher between modules |
| Higher-order thalamus (transthalamic) | route cortex↔cortex | inter-module bus, not peer-to-peer |
| Thalamic reticular nucleus | attentional gating/spotlight | priority / backpressure / filter |
| Striatum + pallidum | action selection (Go/NoGo) | pick exactly one action, suppress the rest |
| Subthalamic (hyperdirect) | global stop/brake | global cancel / interrupt |
| Dopamine | gate balance, reward-driven | global config changing routing gain/bias |

---

### Downloaded papers for this topic
- `transthalamic_pathways_perception.pdf` — The role of transthalamic pathways in perception (2026)
- `basal_ganglia_parkinsons_computational_model.pdf` — Computational modeling of basal ganglia / Parkinson's across scales (2026)

### Canonical references (cite from primary literature)
- Sherman & Guillery, *Exploring the Thalamus* (transthalamic routing).
- Redgrave, Prescott & Gurney (1999), *The basal ganglia: a vertebrate solution to the selection problem*, Neuroscience.
- Frank (2005) / Frank & Badre (2012), corticostriatal gating & hierarchical RL.
- Mink (1996), *The basal ganglia: focused selection and inhibition of competing motor programs*.
