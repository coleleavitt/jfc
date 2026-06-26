# Brain-Inspired AI Architectures & Cognitive Agents

> The bridge: how the neuroscience above has actually been turned into agent/AI architectures.
> This is the literature directly relevant to the Axon SDK design (the "what to build" layer).

---

## 1. Global Workspace Theory (Baars) → the shared blackboard / context bus

**GWT (Baars, 1988).** Many specialized, parallel, unconscious processors compete to **broadcast**
information onto a single **global workspace**; whatever wins is made available system-wide. Its
neural form, **Global Neuronal Workspace** (Dehaene, Changeux), localizes broadcast to long-range
fronto-parietal "ignition."

> This is *literally a blackboard architecture* — the most direct neuroscience→software mapping.
> Currently being tested **adversarially against Integrated Information Theory (IIT)** in a
> preregistered protocol (Cogitate consortium).

> Downloaded: `gnwt_iit_adversarial_protocol.pdf` — the GNWT-vs-IIT adversarial test protocol.

**Axon reading:** the core can expose a **shared context bus** that modules write salient items to
and that broadcasts to all subscribers — but keep it *bounded* (workspace = limited capacity, just
like working memory). Broadcast is the integration mechanism; specialized modules stay encapsulated.

## 2. Modularity of mind (Fodor) → encapsulated modules + a thin central router

Fodor (1983): **input systems are modules** — domain-specific, **informationally encapsulated**,
fast, mandatory — while **central cognition** is non-modular and integrative. The lesson Axon
should take: make the *peripheral* modules (tools, perception, memory) encapsulated and typed; keep
the *central* router thin and general. Don't try to make the router domain-specific.

## 3. Classic cognitive architectures (SOAR, ACT-R) → the prior art on "agent = modules + control"

- **SOAR** (Laird, Newell, Rosenbloom): problem-solving as search in problem spaces, with
  production rules and a learning mechanism (chunking). Long-lived agents with procedural + episodic
  + semantic memory.
- **ACT-R** (Anderson): modular cognition — separate **declarative** (facts) and **procedural**
  (rules) memory, perceptual-motor modules, all coordinated by a central production system through
  buffers. Maps remarkably onto basal-ganglia/PFC gating.

> These are 40 years of evidence that the right shape is **specialized memory/perception/motor
> modules + a central control loop that routes between them via buffers**. CoALA (below) explicitly
> revives this for LLMs.

## 4. CoALA — Cognitive Architectures for Language Agents (Sumers, Yao, Narasimhan, Griffiths 2023)

The key paper for Axon. CoALA frames an LLM agent with three axes:
1. **Memory** — working (context) + long-term: **episodic, semantic, procedural** (straight from
   cognitive architectures / the hippocampus–cortex split).
2. **Action space** — **internal** (reasoning, retrieval, learning) vs **external** (tools,
   environment) actions.
3. **Decision-making loop** — a structured **plan → act → observe** cycle that selects the next
   action (the executive/PFC role).

> Downloaded: `ai_coala_cognitive_architectures_language_agents.pdf`.
> **This is essentially the Axon target architecture** stated in agent terms: typed memory modules
> + a typed action space + a thin decision loop that routes. Cole's "core = context schema + routing
> table + execution graph" is CoALA's memory + action-space + decision-loop, minus the LLM-in-core.

## 5. MemGPT — memory as an OS (Packer et al. 2023)

Treats the limited context window like **RAM** and external stores like **disk**, with the LLM
**paging** information in/out via function calls — an **OS-style virtual memory** for agents.

> Downloaded: `ai_memgpt_llms_as_operating_systems.pdf`.
> **Axon reading:** the memory module should own a *hierarchy* (hot context ↔ warm store ↔ cold
> archive) and expose explicit page-in/page-out (write/consolidate/recall) ops to the core — exactly
> the hippocampal capture→consolidate→recall pipeline, implemented as an OS pager.

## 6. Generative Agents (Park et al. 2023) — memory stream + reflection + retrieval

Believable agents built on a **memory stream** (append-only episodic log), a **retrieval** function
(recency × importance × relevance), and **reflection** (periodically synthesize higher-level
insights from raw memories). This is the hippocampus (episodic write + cued recall) + a "dreamer"
(reflection/consolidation) in code.

> Downloaded: `ai_generative_agents_simulacra.pdf`.
> **Axon reading:** validates CortexKit's design — append-only memory + scored retrieval +
> periodic reflection/consolidation. The scoring function (recency×importance×relevance) is a
> concrete recipe for the recall primitive.

## 7. LLM-agent landscape & brain-inspired efficiency

- `ai_llm_agent_survey_methodology.pdf` — survey of LLM-agent methodology, applications, challenges
  (construction, collaboration, evolution) — the broad map of the design space.
- `brain_inspired_efficient_ai.pdf`, `brain_inspired_multimodal_learning.pdf` — brain-inspired
  mechanisms (sparsity, event-driven/spiking compute, neuromodulation, predictive coding) for
  efficient and multimodal AI; the hardware/efficiency angle.

---

## Synthesis → the Axon architecture (what the brain literature endorses)

```
            ┌───────────────────────────────────────────────┐
  Alfonso → │ EXECUTIVE / PFC: plan, decompose, gate, monitor │  (built last, sits on top)
            └───────────────────────────────────────────────┘
                              ▲  writes plans / reads context
  ┌───────────────────────────┴────────────────────────────────┐
  │ AXON CORE: typed message schema + routing table + exec graph│  ← thalamus + basal ganglia
  │   (NO LLM calls, NO memory, NO UI — just typed routing)     │     (route + gate, default-deny)
  └───────┬───────────────┬───────────────┬────────────────────┘
          ▼               ▼               ▼
   MEMORY (hippocampus) TOOLS (sensorimotor) UI (any skin: TUI/GUI/watch)
   capture→consolidate   typed, stateless    pluggable, reads the bus
   →recall (MemGPT/      perception+action
   GenAgents/CoALA)      primitives
```

**The non-negotiable rule (every framework agrees):** the controller/router must NOT also be the
executor. Keep the core thin and LLM-free; cognition lives in pluggable, encapsulated, typed
modules; integration happens via a bounded broadcast workspace. That is simultaneously: GWT
(blackboard), Fodor (encapsulated modules + thin center), ACT-R/SOAR (modules + control loop),
CoALA (memory + action-space + decision-loop), and the connectome (modular graph + routing plane).

---

### Downloaded papers for this topic
- `ai_coala_cognitive_architectures_language_agents.pdf` — Sumers et al., CoALA (2023)
- `ai_memgpt_llms_as_operating_systems.pdf` — Packer et al., MemGPT (2023)
- `ai_generative_agents_simulacra.pdf` — Park et al., Generative Agents (2023)
- `ai_llm_agent_survey_methodology.pdf` — Luo et al., LLM Agent survey (2025)
- `gnwt_iit_adversarial_protocol.pdf` — GNWT vs IIT adversarial test protocol
- `brain_inspired_efficient_ai.pdf` — Brain-inspired strategies for efficient AI (2026)
- `brain_inspired_multimodal_learning.pdf` — Brain-inspired mechanisms for multimodal learning (2026)

### Canonical references
- Baars (1988), *A Cognitive Theory of Consciousness*; Dehaene & Changeux (2011), global neuronal workspace.
- Fodor (1983), *The Modularity of Mind*.
- Laird (2012), *The Soar Cognitive Architecture*; Anderson (2007), *How Can the Human Mind Occur in the Physical Universe?* (ACT-R).
