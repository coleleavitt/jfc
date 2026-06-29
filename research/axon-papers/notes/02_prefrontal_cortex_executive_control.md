# Prefrontal Cortex — Executive Control ("Alfonso" / the planner)

> **Axon SDK mapping:** the *executive / orchestration plane*. The PFC is the "air traffic
> controller" (MIT Picower framing): it never flies the planes (executes), it holds goals/rules
> and directs the flow of activity across other systems. In the SDK this is the planner/router
> that decomposes work, holds task context, decides what fires next, and gates actions —
> the module CortexKit calls **Alfonso**.

---

## 1. The integrative theory of PFC (Miller & Cohen, 2001)

The canonical account: the PFC provides **top-down "bias signals"** that guide the flow of
activity through other brain systems along the pathways needed to perform a task. It does not
itself perform perception or motor output — it represents **goals and the rules/mappings** that
get you there, and biases competition among lower systems in favor of task-relevant pathways.

> This is *exactly* the "ultra-thin core that decides what fires next" idea. The PFC is a
> controller that holds an abstract task representation and routes, not an executor.

## 2. Functional subdivisions (each → an SDK responsibility)

| PFC subregion | Executive function | Axon SDK analog |
|---|---|---|
| **Dorsolateral (dlPFC)** | Working memory, planning, task-switching, manipulation of information | Holding task context across steps; the execution graph / plan |
| **Ventrolateral (vlPFC)** | Response selection, inhibition, retrieval control | Choosing which tool/agent to call; *deciding when NOT to act* |
| **Medial (mPFC) / ACC** | Monitoring, conflict & error detection, updating goals | Detecting a failing plan and re-routing; verification trigger |
| **Orbitofrontal (OFC)** | Value, risk, social/affective reasoning | Risk assessment; when to ask the user vs. commit |

The three dlPFC primitives map cleanly to the SDK core:
- **Working memory** → typed context/scratchpad schema
- **Task-switching** → routing table
- **Planning / decomposition** → execution graph

## 3. Hierarchical cognitive control (corticostriatal loops)

Control is **hierarchically organized**: rostral (anterior) PFC handles more abstract, temporally
extended goals; caudal (posterior) PFC handles concrete stimulus–response rules. Higher-order
regions *gate* lower-order ones. Frank & Badre's hierarchical reinforcement-learning model of
**cortico-striatal** circuits formalizes this: PFC maintains rules; the basal ganglia provide a
**gating signal** (see `04_routing_basal_ganglia_thalamus.md`) that decides *when* to update or
act on a PFC representation. This is a planner (PFC) + dispatcher (basal ganglia) — precisely the
Alfonso-over-core split.

## 4. Working memory: how the PFC "holds" context

- **Persistent activity**: delay-period firing in dlPFC neurons sustains information across a gap —
  the classic neural correlate of working memory (line/bump-attractor mechanism; see
  `05_attractors_population_geometry.md`).
- Limited capacity & active maintenance ≈ a small, typed, bounded context window in the SDK.
- Susceptible to interference → the system needs **gating** to protect maintained items (basal
  ganglia "close the gate" to hold, "open the gate" to update).

## 5. Executive dysfunction — what breaks when the controller fails

> Downloaded: `executive_dysfunction_transdiagnostic.pdf` — executive dysfunction as a
> *transdiagnostic* mechanism across psychopathology (i.e. a failure of the control layer shows up
> everywhere downstream); `pfc_spine_loss_cognition.pdf` — prefrontal dendritic-spine loss and its
> circuit consequences for cognition (the controller degrades when its connectivity thins).

**SDK reading:** "executive dysfunction happens when the coordination mechanism tries to also be
the execution mechanism." Keep Alfonso thin: it plans, gates, monitors, and routes — it must not
become the thing that does memory, tools, and UI. That conflation is what killed JFC.

## 6. The air-traffic-controller summary

```
            goals / rules / task-set
                     │ (top-down bias)
   ┌─────────────────┼─────────────────┐
   ▼                 ▼                 ▼
 perception       memory            action
 (sensory)     (hippocampus)    (motor/tools)
```
PFC sets the bias; it does not occupy any of the three boxes. Alfonso = this controller, built
**last**, sitting on top of the core, reading context from memory and writing plans to the graph.

---

### Downloaded papers for this topic
- `executive_dysfunction_transdiagnostic.pdf` — Executive dysfunction as a transdiagnostic mechanism (2026)
- `pfc_spine_loss_cognition.pdf` — Prefrontal spine loss in neuropsychiatric disorders: circuit consequences (2026)

### Canonical references (cite from primary literature)
- Miller & Cohen (2001), *An integrative theory of prefrontal cortex function*, Annu. Rev. Neurosci.
- Frank & Badre (2012), *Mechanisms of hierarchical reinforcement learning in corticostriatal circuits*, Cereb. Cortex.
- Fuster, *The Prefrontal Cortex* (textbook).
