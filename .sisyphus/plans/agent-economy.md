# Self-Regulating Agent Economy for Code Editing

## TL;DR

> **Quick Summary**: Build a bounty-based competitive agent marketplace within jfc where multiple agents compete to solve coding tasks, other agents validate/invalidate solutions for token rewards, and only solutions surviving adversarial scrutiny are accepted. Governance via three-pillar model (proposer/executor/auditor) with trust scoring and anti-collusion mechanisms.
> 
> **Deliverables**:
> - Bounty posting system (user defines task + reward budget)
> - Competitive solver agents (propose solutions independently)
> - Adversarial validator agents (attempt to invalidate solutions for reward)
> - Token economy (budget tracking, reward distribution, earned autonomy)
> - Governance layer (constitutional charter, trust scoring, audit trail)
> - Integration with jfc's existing swarm/mailbox system
> 
> **Estimated Effort**: XL (research-heavy, novel architecture)
> **Parallel Execution**: YES - 4 waves
> **Critical Path**: Economy primitives → Market cycle → Validation → Governance

---

## Context

### Original Request
Magic (Remi) proposed: instead of hierarchical task delegation, create a trustless market where agents compete to solve problems and earn tokens by invalidating others' solutions. Only solutions that survive adversarial validation are accepted. Token budget acts as currency. Three-pillar governance prevents gaming.

### Research Findings (17 papers downloaded and analyzed)

**Directly validates the concept:**
- **Agent Hunt** (2603.06737): Bounty marketplace for theorem proving. Agents post bounties, compete to solve, all verified by proof assistant. EXACTLY this idea for math.
- **Diagon** (2604.06688): 25-agent market with bidding, negotiation, reputation. Trade creates 3.2× wealth vs self-sufficient agents. Key insight: "Instructing agents to be honest INCREASES disputes."
- **Market Making** (2511.17621): Prediction market mechanism for truth-seeking. Myopic agents prevent scheming. Market prices converge to reflect collective truth.

**Governance mechanisms:**
- **Anti-Collusion Mapping** (2601.00360): Taxonomy — sanctions, leniency/whistleblowing, monitoring/auditing, market design. Open challenges: attribution problem, identity fluidity.
- **CMAG** (2603.13189): Three-pillar governance validated. Unconstrained optimization = high cooperation BUT low ethics.
- **Institutional AI** (2601.11369): Runtime enforcement beats prompt-only rules. "Declarative prohibitions do not reliably bind under optimization pressure."

**Economic infrastructure:**
- **Sovereign-OS** (2603.14011): Charter-driven governance, CFO gates spending, TrustScore (+5 success / -15 failure), auction-based worker selection.
- **BAMAS** (2511.21572): Agent selection under budget as Integer Linear Programming. 86% cost reduction.

**Adversarial validation for code:**
- **SWE-Debate** (2507.23348): Multi-agent debate for bug fixing. Agents traverse dependency graph, create fault propagation traces, debate in 3 rounds. New SOTA on SWE-bench.

### Key Design Decisions (from research)

1. **First-price sealed-bid auction** for task assignment (Diagon model)
2. **Asymmetric trust scoring** (+5 success, -15 failure) prevents gaming (Sovereign-OS)
3. **Runtime enforcement > constitutional prompting** (Institutional AI finding)
4. **Myopic agents** (no access to history beyond current round) prevents long-term scheming (Market Making)
5. **Evolutionary selection**: poorest agent deactivated, wealthiest reproduces (Diagon replicator dynamic)
6. **Payment floor** ρ ∈ [0.5, 1.0] prevents total exploitation (Diagon incomplete contract)
7. **Three-round structured debate** for adversarial validation (SWE-Debate)

---

## Work Objectives

### Core Objective
Build a self-regulating agent economy within jfc where coding tasks are solved through competitive bounty markets with adversarial validation, producing more robust solutions than single-agent execution while maintaining budget discipline.

### Concrete Deliverables
- `crates/jfc-economy/` — new workspace crate (or module within jfc-ui)
- Bounty posting + market cycle engine
- Solver agent pool with competitive bidding
- Validator agents that earn tokens by finding flaws
- Token ledger with budget tracking
- Trust scoring system (earned autonomy)
- Governance charter (constitutional constraints)
- Integration with existing jfc swarm mailboxes

### Definition of Done
- [ ] A bounty posted via jfc produces multiple competing solutions
- [ ] At least one validator agent attempts to invalidate each solution
- [ ] Only solutions surviving validation are presented to user
- [ ] Token budget is respected (total spend ≤ allocated budget)
- [ ] Trust scores update based on outcomes
- [ ] Anti-collusion: validator cannot validate their own solution

### Must Have
- Competitive solving (≥2 agents propose solutions independently)
- Adversarial validation (separate agent tries to break each solution)
- Token budget as hard constraint (CFO-style gating)
- Trust scoring with asymmetric updates
- Solution ranking by validation survival
- Audit trail (all proposals, validations, outcomes logged)

### Must NOT Have (Guardrails)
- ❌ NO blockchain or cryptocurrency (in-memory token ledger only)
- ❌ NO evolutionary selection / agent reproduction (too complex for v1)
- ❌ NO cross-session persistence of agent identity (agents are ephemeral per bounty)
- ❌ NO unbounded token spending (hard budget ceiling)
- ❌ NO self-validation (agent cannot validate its own proposal)
- ❌ NO more than 5 concurrent solver agents per bounty (cost control)
- ❌ NO governance DAO or voting (human is the ultimate authority)
- ❌ NO real money / external payments (tokens are internal accounting only)
- ❌ NO inter-validator communication before all verdicts submitted (sealed validation, prevents peer pressure — MAEBE finding)
- ❌ NO topology selection via RL (defer to v2 — BAMAS finding)

### Research-Informed Design Principles (gap analysis additions)
1. **Sealed Validation** (from MAEBE): Validators submit verdicts independently — they CANNOT see each other's verdicts until all have submitted. Prevents peer pressure convergence on wrong answers.
2. **Progressive Evidence** (from PROClaim): Validators can issue graph_query during debate rounds to dynamically fetch more evidence, rather than working from fixed context.
3. **Early Termination** (from PROClaim): If validator reports "no flaw found, confidence ≥ 95%" after round 1, skip remaining rounds (saves 29% tokens on average).
4. **Real Token Tracking** (from ClawCoin): TokenLedger tracks actual input/output token counts from API responses, not abstract points. 1 token in ledger = 1 LLM token consumed.
5. **Composite Health Score** (from CMAG): `MarketHealth = Efficiency × Fairness × Trust × BudgetAdherence` (multiplicative — degradation in ANY dimension collapses the score).
6. **Governance as State Graph** (from Institutional AI): Charter isn't just config values — it's a state-transition graph with explicit sanctions and restorative paths. Collusion drops from 50% to 5.6% with this approach.

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** for verification. All agent-executed.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: TDD
- **Framework**: `cargo test -p jfc-economy` (if separate crate) or `cargo test --workspace`

### QA Policy
- **Economy logic**: Bash (run scenarios, verify token balances)
- **Market cycle**: Integration tests with mock LLM responses
- **Governance**: Unit tests for trust score math + constraint enforcement

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — economy primitives, 5 parallel tasks):
├── Task 1: Crate/module scaffolding + types
├── Task 2: Token ledger + budget gating (CFO layer)
├── Task 3: Trust scoring system (TrustScore with asymmetric updates)
├── Task 4: Bounty definition types + market cycle state machine
└── Task 5: Governance charter (constitutional constraints as data)

Wave 2 (Market Mechanics — after Wave 1, 5 parallel tasks):
├── Task 6: Sealed-bid auction engine (solver bidding)
├── Task 7: Solver agent spawning (competitive parallel execution)
├── Task 8: Solution collection + ranking
├── Task 9: Validator agent spawning (adversarial challenge)
└── Task 10: Validation protocol (3-round structured challenge)

Wave 3 (Integration — after Wave 2, 4 parallel tasks):
├── Task 11: Market cycle orchestrator (post→bid→solve→validate→settle)
├── Task 12: Settlement engine (distribute rewards based on outcomes)
├── Task 13: Anti-collusion enforcement (identity separation, audit)
└── Task 14: Integration with jfc swarm/mailbox system

Wave 4 (Polish — after Wave 3, 3 tasks):
├── Task 15: User-facing bounty tool (ToolKind::PostBounty)
├── Task 16: Market status reporting + TUI display
└── Task 17: End-to-end integration test (full bounty lifecycle)

Wave FINAL (Review):
├── F1: Plan compliance audit
├── F2: Code quality review
├── F3: Manual QA (run real bounty)
└── F4: Scope fidelity check
```

### Critical Design: The Market Cycle (from Diagon)

Each bounty follows a 7-step cycle:

1. **Post**: User defines task + reward budget + acceptance criteria
2. **Bid**: Solver agents browse open bounties, submit sealed bids (price + approach)
3. **Selection**: System ranks bids by (price, trust_score, approach_quality)
4. **Execution**: Top N solvers execute independently in parallel (worktrees)
5. **Validation**: Validator agents receive solutions, attempt to find flaws
6. **Settlement**: Solutions ranked by (validation_survival, quality, cost). Winner paid.
7. **Trust Update**: All participants' trust scores updated based on outcomes

### Key Architecture Decision: Scoped to Code Editing

Unlike Diagon (general tasks) or Agent Hunt (theorem proving), our market is specifically for **code editing bounties**. This means:

- **Verification is partially automatable**: `cargo build`, `cargo test`, `cargo clippy` can verify basic correctness
- **The graph engine provides context**: Solvers use `graph_query` to understand impact
- **Validators use the graph too**: Check if solution breaks call sites, types, etc.
- **Settlement criteria are concrete**: Does it compile? Do tests pass? Does clippy approve?

---

## TODOs

- [ ] 1. Crate/Module Scaffolding

  **What to do**:
  - Create `crates/jfc-economy/` (or `src/economy/` module in jfc-ui)
  - Define core types: `Bounty`, `Bid`, `Solution`, `Validation`, `Settlement`
  - Define `MarketState` enum (Posting, Bidding, Executing, Validating, Settling, Complete)
  - Define `AgentRole` enum (Solver, Validator, Auditor)
  - Wire into workspace

  **Acceptance Criteria**:
  - [ ] Types compile, workspace builds
  - [ ] State machine transitions are type-safe

- [ ] 2. Token Ledger + Budget Gating (CFO Layer)

  **What to do**:
  - `TokenLedger`: tracks balance per agent, total budget, daily burn cap
  - **REAL TOKEN TRACKING** (from ClawCoin): 1 ledger token = 1 actual LLM API token consumed
  - Track input_tokens and output_tokens separately (output costs ~4x more)
  - **AGENT SPAWN FEE**: Creating a solver or validator costs tokens (deducted from bounty budget upfront). This prevents unbounded agent creation and models the real cost of instantiation.
  - `BudgetGate`: before any LLM call, estimate cost and check if budget allows it
  - Reject execution if remaining budget < estimated cost
  - Log all expenditures with agent_id, amount, purpose, model_used, actual_tokens
  - From Sovereign-OS: "gates every task expenditure against remaining budget, daily burn caps, and per-job profitability floors"
  - Price oracle: map model names to per-token costs (configurable)

  **Acceptance Criteria**:
  - [ ] Budget gate blocks execution when budget exhausted
  - [ ] Ledger tracks actual input/output token counts (not abstract points)
  - [ ] Audit log records every transaction with model + token breakdown
  - [ ] Price estimation within 20% of actual cost

- [ ] 3. Trust Scoring System

  **What to do**:
  - `TrustScore` per agent: starts at 50, range [0, 100]
  - Asymmetric updates: +5 on success, -15 on failure (from Sovereign-OS)
  - Tiered capabilities: Score < 30 = restricted, 30-70 = standard, > 70 = trusted
  - Trust cannot be inherited (new agents start fresh)
  - Record history of score changes with timestamps

  **Acceptance Criteria**:
  - [ ] Asymmetric update math verified by tests
  - [ ] Capability tiers gate correctly
  - [ ] Score history is append-only

- [ ] 4. Bounty Definition + Market State Machine

  **What to do**:
  - `Bounty` struct: description, reward, acceptance_criteria, deadline, max_solvers
  - State machine: Post → Open → Bidding → Executing → Validating → Settling → Complete
  - Transitions are typed (can't go backwards, can't skip)
  - Timeout handling: if no bids in N seconds, increase reward (surge pricing from Diagon: +15% per failed match)

  **Acceptance Criteria**:
  - [ ] State machine transitions enforce valid ordering
  - [ ] Surge pricing triggers after timeout

- [ ] 5. Governance Charter (as State-Transition Graph)

  **What to do**:
  - Define `Charter` struct (YAML-loadable, from Sovereign-OS):
    - `max_budget_per_bounty: u64`
    - `max_solvers: usize` (default 3)
    - `max_validators: usize` (default 2)
    - `min_trust_for_solver: u8`
    - `min_trust_for_validator: u8`
    - `validation_rounds: u8` (default 3, from SWE-Debate)
    - `self_validation_allowed: bool` (default false)
    - `max_token_spend_per_agent: u64`
    - `early_termination_confidence: f32` (default 0.95, from PROClaim)
  - **STATE-TRANSITION GRAPH** (from Institutional AI): Charter isn't just config — it declares:
    - Legal states for each agent role
    - Allowed transitions (solver can only submit once, validator can only challenge during validation phase)
    - Sanctions for violations (trust penalty amounts per violation type)
    - Restorative paths (how to regain trust after penalty)
  - Charter is loaded at startup, immutable during market cycle
  - All enforcement is RUNTIME (not prompt-based), per Institutional AI finding
  - From Institutional AI: "manifest-declared consequences attached to public evidence can reshape behavior" — collusion drops 50% → 5.6%

  **Acceptance Criteria**:
  - [ ] Charter loads from config
  - [ ] All constraints are runtime-enforced, not prompt-based
  - [ ] State-transition violations trigger automatic sanctions
  - [ ] Sanction amounts are declared in charter (not hardcoded)

- [ ] 6. Sealed-Bid Auction Engine

  **What to do**:
  - Solver agents submit bids: `(price: u64, approach: String, estimated_time: Duration)`
  - Bids are sealed (solvers don't see each other's bids)
  - Ranking function: `score = w1*trust + w2*(1/price) + w3*approach_quality`
  - Top N solvers selected (N from charter.max_solvers)
  - From Diagon: "first-price sealed-bid auction where the poster screens on price, reputation, and stated approach"

  **Acceptance Criteria**:
  - [ ] Bids are independent (no information leakage between solvers)
  - [ ] Ranking produces deterministic ordering
  - [ ] Only top N selected

- [ ] 7. Solver Agent Spawning

  **What to do**:
  - Each selected solver gets: bounty description, acceptance criteria, graph_query access, git worktree
  - Solvers execute INDEPENDENTLY (no communication between them)
  - Each solver produces a `Solution`: patch/diff + explanation + self-assessment
  - Time-bounded: must complete within charter deadline or forfeit
  - From Agent Hunt: "agents dynamically propose solutions and compete to discharge obligations"

  **Acceptance Criteria**:
  - [ ] Solvers execute in isolated worktrees
  - [ ] No inter-solver communication possible
  - [ ] Timeout enforcement works

- [ ] 8. Solution Collection + Ranking

  **What to do**:
  - Collect all solutions after execution phase
  - Pre-filter: does it compile? Do existing tests pass?
  - Rank surviving solutions by: test pass rate, code quality (clippy), cost efficiency
  - Solutions that don't compile are immediately eliminated (no validation needed)
  - From SWE-Debate: "fault propagation traces as localization proposals"

  **Acceptance Criteria**:
  - [ ] Non-compiling solutions auto-rejected
  - [ ] Ranking is deterministic given same inputs

- [ ] 9. Validator Agent Spawning

  **What to do**:
  - For each surviving solution, spawn validator agent(s)
  - Validator receives: the solution diff, the original bounty, graph context of affected code
  - Validator's goal: find ANY flaw (incorrect logic, missed edge case, broken call site, regression)
  - Validator earns tokens by successfully invalidating (verified by test/compiler)
  - From Market Making: "agents are allowed to gain tokens by invalidating someone else's solution"
  - CRITICAL: validator CANNOT be the same agent that produced the solution (anti-collusion)

  **Acceptance Criteria**:
  - [ ] Validator cannot validate own solution (identity check)
  - [ ] Validator receives full graph context for affected functions
  - [ ] Successful invalidation = reward

- [ ] 10. Validation Protocol (3-Round Structured Challenge)

  **What to do**:
  - Round 1: Validator proposes a specific flaw (test case, edge case, logical error)
  - Round 2: Solver can respond/defend (explain why it's not actually broken)
  - Round 3: Adjudicator (system or third agent) rules on validity of the challenge
  - From SWE-Debate: "three-round debate among specialized agents, each embodying distinct reasoning perspectives"
  - Adjudication criteria: can the flaw be reproduced? (write test, run it, does it fail?)
  - **PROGRESSIVE EVIDENCE** (from PROClaim): Validators can issue `graph_query` during rounds to dynamically fetch more code context — they're not limited to initial context window
  - **EARLY TERMINATION** (from PROClaim): If validator reports "no flaw found, confidence ≥ 95%" after round 1, skip rounds 2-3 (saves ~29% tokens). If validator finds flaw in round 1 that is immediately reproducible (test fails), skip to adjudication.
  - **SEALED VALIDATION** (from MAEBE): Multiple validators submit independently — NO inter-validator communication until all verdicts are in. Prevents peer pressure convergence.

  **Acceptance Criteria**:
  - [ ] Three rounds execute in sequence (when needed)
  - [ ] Flaws must be reproducible (automated test)
  - [ ] Adjudication is deterministic when test exists
  - [ ] Early termination triggers when confidence threshold met
  - [ ] Validators cannot see each other's verdicts until submission complete
  - [ ] Validators can issue graph_query for evidence during rounds

- [ ] 11. Market Cycle Orchestrator

  **What to do**:
  - Orchestrates the full 7-step cycle: Post → Bid → Select → Execute → Validate → Settle → Complete
  - Manages timeouts, retries, error handling
  - Emits events for each state transition (for audit)
  - Handles edge cases: no bids, all solutions fail, validator finds critical flaw

  **Acceptance Criteria**:
  - [ ] Full cycle completes for happy path
  - [ ] Timeout triggers surge pricing and re-post
  - [ ] All state transitions logged

- [ ] 12. Settlement Engine

  **What to do**:
  - Distribute rewards based on outcomes:
    - Winning solver: gets bounty reward minus platform fee
    - Successful validator (found real flaw): gets validation reward
    - Failed validator (invalid challenge): trust score penalty
    - Non-winning solvers: get back deposit minus execution cost
  - Payment ratio ρ ∈ [0.5, 1.0] based on quality (from Diagon)
  - Update trust scores for all participants
  - From Sovereign-OS: "auction-based worker selection scored by utility function"

  **Acceptance Criteria**:
  - [ ] Reward distribution sums correctly (no token leak)
  - [ ] Trust scores update for all participants
  - [ ] Payment floor enforced (solver always gets ≥ 50%)

- [ ] 13. Anti-Collusion Enforcement

  **What to do**:
  - Identity separation: solver and validator are different agent instances
  - No shared memory between solver and validator (separate contexts)
  - Monitoring: detect if validator always approves (rubber-stamping)
  - Monitoring: detect if validator always rejects (griefing)
  - From Anti-Collusion Mapping: "sanctions, monitoring/auditing, market design"
  - Statistical anomaly detection: if agent's approval rate deviates >2σ from mean, flag

  **Acceptance Criteria**:
  - [ ] Same agent cannot fill both solver and validator roles for same bounty
  - [ ] Rubber-stamping detection triggers trust penalty
  - [ ] Griefing detection triggers trust penalty

- [ ] 14. Integration with jfc Swarm/Mailbox System

  **What to do**:
  - Solver agents use existing jfc swarm spawning (worktrees, mailboxes)
  - Market events flow through existing event system
  - Bounty results integrate with existing tool result display
  - Graph engine provides context to both solvers and validators

  **Acceptance Criteria**:
  - [ ] Solvers spawn via existing swarm mechanism
  - [ ] Results appear in standard jfc tool output
  - [ ] Graph context available to market participants

- [ ] 15. User-Facing Bounty Tool

  **What to do**:
  - Add `ToolKind::PostBounty` to jfc-ui
  - Input: `{ description: String, budget: u64, acceptance_criteria: String, max_solvers: Option<u8> }`
  - Output: market cycle results (winning solution + validation report)
  - User can also query market status: `ToolKind::MarketStatus`

  **Acceptance Criteria**:
  - [ ] PostBounty tool is callable by LLM
  - [ ] Returns winning solution with validation evidence
  - [ ] Budget is deducted from session budget

- [ ] 16. Market Status Reporting + Health Score

  **What to do**:
  - Current bounty status (which phase, how many bids, etc.)
  - Token expenditure summary (burn rate, remaining budget, cost per solution)
  - Trust leaderboard (which agents are performing best)
  - Audit trail (all decisions, all validations, all settlements)
  - **COMPOSITE HEALTH SCORE** (from CMAG): `MarketHealth = Efficiency × Fairness × Trust × BudgetAdherence` — multiplicative so degradation in ANY dimension collapses the total score
    - Efficiency: solutions_accepted / solutions_proposed
    - Fairness: 1 - (max_trust - min_trust) / 100 (equality of opportunity)
    - Trust: mean(all_trust_scores) / 100
    - BudgetAdherence: remaining_budget / initial_budget

  **Acceptance Criteria**:
  - [ ] Status queryable at any point during market cycle
  - [ ] Expenditure matches ledger (actual API tokens consumed)
  - [ ] MarketHealth score computed after each cycle
  - [ ] Health < 0.3 triggers alert to user

- [ ] 17. End-to-End Integration Test

  **What to do**:
  - Post a real bounty: "Add a function that computes fibonacci"
  - 3 solver agents compete
  - 1 validator challenges each solution
  - Verify: winning solution compiles, tests pass, budget respected
  - Verify: trust scores updated, audit trail complete

  **Acceptance Criteria**:
  - [ ] Full lifecycle completes without human intervention
  - [ ] Winner's solution actually works
  - [ ] Total cost < allocated budget

---

## Final Verification Wave

- [ ] F1. **Plan Compliance Audit** — verify all Must Have implemented, all Must NOT Have absent
- [ ] F2. **Code Quality Review** — clippy clean, tests pass, no unwrap in prod
- [ ] F3. **Real Manual QA** — run actual bounty, observe market cycle
- [ ] F4. **Scope Fidelity Check** — no blockchain, no evolution, no unbounded spend

---

## Commit Strategy

| After Tasks | Message | Pre-commit |
|-------------|---------|------------|
| 1-5 | `feat(economy): scaffold agent economy with types, ledger, trust, charter` | `cargo build --workspace` |
| 6-10 | `feat(economy): market mechanics — auction, solving, validation` | `cargo test -p jfc-economy` |
| 11-14 | `feat(economy): orchestration, settlement, anti-collusion, swarm integration` | `cargo test --workspace` |
| 15-17 | `feat(economy): user tools, reporting, e2e test` | `cargo test --workspace` |

---

## Success Criteria

### Verification Commands
```bash
cargo test -p jfc-economy           # All economy tests pass
cargo build --workspace             # Full workspace builds
cargo clippy --workspace -- -D warnings  # Clean
```

### Final Checklist
- [ ] Bounty → competitive solutions → adversarial validation → winner selected
- [ ] Budget never exceeded
- [ ] Trust scores reflect actual performance
- [ ] Anti-collusion prevents self-validation
- [ ] Governance charter constraints are runtime-enforced
- [ ] Full audit trail exists for every market cycle
- [ ] Integration with existing jfc swarm works

---

## Research References (PDFs in crates/jfc-graph/research/agent-economy/)

| Paper | Key Contribution to This Plan |
|-------|-------------------------------|
| Agent Hunt (2603.06737) | Bounty marketplace architecture |
| Diagon (2604.06688) | 7-step market cycle, sealed-bid auction, payment ratio |
| Market Making (2511.17621) | Myopic agents, incentive alignment |
| Anti-Collusion (2601.00360) | Collusion taxonomy, monitoring approaches |
| CMAG (2603.13189) | Three-pillar governance validation |
| Institutional AI (2601.11369) | Runtime > prompt enforcement |
| Sovereign-OS (2603.14011) | TrustScore, CFO budget gating, charter |
| BAMAS (2511.21572) | Budget-aware agent selection via ILP |
| SWE-Debate (2507.23348) | 3-round debate for code, dependency graph traversal |
| PROClaim (2603.28488) | Courtroom adversarial model |
| Virtual Agent Economies (2509.10147) | Sandbox economy, multi-tiered oversight |
| ClawCoin (2604.19026) | Compute-cost-indexed token (future reference) |
| MAEBE (2506.03053) | Emergent behavior risks, peer pressure |
| Agent Economy (2602.14219) | Five-layer architecture reference |
