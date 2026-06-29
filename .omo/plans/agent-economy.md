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
7. **Reward Hack Resistance** (from Anthropic Mythos System Card): Bounty acceptance criteria MUST be evaluated by an independent system (compiler, test suite), NEVER by another agent alone. Agents will find novel ways to game metrics — verification must be mechanistic, not judgment-based.
8. **Sandbox Hardening** (from Anthropic Mythos): Solver agents CANNOT modify their own permissions, spawn processes outside worktree, search for credentials, or escalate privileges. Monitor for base64-encoded commands and indirect execution paths.
9. **Graceful Forfeit** (from Anthropic Mythos "overeager" finding): Agents must have an explicit "abandon" action — if stuck, they forfeit the bounty and return remaining tokens rather than persisting destructively.
10. **Deception-Aware Validation** (from Anthropic Mythos): Validators must not be able to plant bugs then "discover" them. Sealed validation prevents this structurally, but additionally: any code ADDED by a validator during the challenge round must be flagged and reviewed separately from code that was already present.

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

- [x] 1. Crate/Module Scaffolding

  **What to do**:
  - Create `crates/jfc-economy/` (or `src/economy/` module in jfc-ui)
  - Define core types: `Bounty`, `Bid`, `Solution`, `Validation`, `Settlement`
  - Define `MarketState` enum (Posting, Bidding, Executing, Validating, Settling, Complete)
  - Define `AgentRole` enum (Solver, Validator, Auditor)
  - Wire into workspace

  **Acceptance Criteria**:
  - [ ] Types compile, workspace builds
  - [ ] State machine transitions are type-safe

  **QA Scenarios**:
  ```
  Scenario: Crate compiles cleanly
    Tool: Bash
    Steps:
      1. cargo build -p jfc-economy
      2. cargo clippy -p jfc-economy -- -D warnings
    Expected: Both pass with zero errors/warnings
    Evidence: .sisyphus/evidence/task-1-build.txt

  Scenario: Invalid state transition rejected at compile time
    Tool: Bash
    Steps:
      1. Write test attempting MarketState::Posting -> MarketState::Settling (skipping phases)
      2. cargo test -p jfc-economy -- state_machine_invalid_transition
    Expected: Test passes (the transition is rejected)
    Evidence: .sisyphus/evidence/task-1-state-machine.txt
  ```

- [x] 2. Token Ledger + Budget Gating (CFO Layer)

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

  **QA Scenarios**:
  ```
  Scenario: Budget gate rejects when exhausted
    Tool: Bash
    Steps:
      1. Create ledger with budget=100
      2. Debit 95 tokens
      3. Attempt to gate-check a request estimated at 10 tokens
      4. cargo test -p jfc-economy -- test_budget_gate_rejects
    Expected: gate returns Err(BudgetExhausted), balance unchanged at 5
    Evidence: .sisyphus/evidence/task-2-budget-gate.txt

  Scenario: Spawn fee deducted on agent creation
    Tool: Bash
    Steps:
      1. Create ledger with budget=1000
      2. Spawn a solver (spawn_fee=50 configured in charter)
      3. Assert balance is now 950
      4. cargo test -p jfc-economy -- test_spawn_fee
    Expected: Balance reduced by exactly spawn_fee amount
    Evidence: .sisyphus/evidence/task-2-spawn-fee.txt
  ```

- [x] 3. Trust Scoring System

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

  **QA Scenarios**:
  ```
  Scenario: Asymmetric trust scoring
    Tool: Bash
    Steps:
      1. Create agent with trust=50
      2. Record success → trust should be 55
      3. Record failure → trust should be 40 (55-15)
      4. cargo test -p jfc-economy -- test_trust_asymmetric
    Expected: 50→55→40 exact sequence
    Evidence: .sisyphus/evidence/task-3-trust-math.txt

  Scenario: Capability tier gating
    Tool: Bash
    Steps:
      1. Create agent with trust=25 (restricted tier)
      2. Attempt to bid on bounty requiring min_trust=30
      3. Assert bid is rejected
      4. cargo test -p jfc-economy -- test_trust_tier_gate
    Expected: Bid rejected with TrustTooLow error
    Evidence: .sisyphus/evidence/task-3-tier-gate.txt
  ```

- [x] 4. Bounty Definition + Market State Machine

  **What to do**:
  - `Bounty` struct: description, reward, acceptance_criteria, deadline, max_solvers
  - State machine: Post → Open → Bidding → Executing → Validating → Settling → Complete
  - Transitions are typed (can't go backwards, can't skip)
  - Timeout handling: if no bids in N seconds, increase reward (surge pricing from Diagon: +15% per failed match)

  **Acceptance Criteria**:
  - [ ] State machine transitions enforce valid ordering
  - [ ] Surge pricing triggers after timeout

  **QA Scenarios**:
  ```
  Scenario: State machine rejects invalid transition
    Tool: Bash
    Steps:
      1. Create bounty in Bidding state
      2. Attempt transition to Complete (skipping Executing, Validating, Settling)
      3. cargo test -p jfc-economy -- test_invalid_transition
    Expected: Returns Err(InvalidTransition { from: Bidding, to: Complete })
    Evidence: .sisyphus/evidence/task-4-invalid-transition.txt

  Scenario: Surge pricing after timeout
    Tool: Bash
    Steps:
      1. Create bounty with reward=100, timeout=0ms (immediate)
      2. Trigger timeout check
      3. Assert reward is now 115 (+15%)
      4. cargo test -p jfc-economy -- test_surge_pricing
    Expected: Reward increases by exactly 15%
    Evidence: .sisyphus/evidence/task-4-surge.txt
  ```

- [x] 5. Governance Charter (as State-Transition Graph)

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

  **QA Scenarios**:
  ```
  Scenario: Charter loads and enforces constraints
    Tool: Bash
    Steps:
      1. Create charter YAML with max_solvers=2
      2. Load charter, attempt to spawn 3 solvers
      3. cargo test -p jfc-economy -- test_charter_max_solvers
    Expected: Third solver rejected with CharterViolation error
    Evidence: .sisyphus/evidence/task-5-charter-enforce.txt

  Scenario: State-transition violation triggers sanction
    Tool: Bash
    Steps:
      1. Charter declares: solver submitting twice = -20 trust penalty
      2. Solver attempts second submission
      3. Assert submission rejected AND trust reduced by 20
      4. cargo test -p jfc-economy -- test_charter_sanction
    Expected: Submission blocked, trust_score -= 20
    Evidence: .sisyphus/evidence/task-5-sanction.txt
  ```

- [x] 6. Sealed-Bid Auction Engine

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

  **QA Scenarios**:
  ```
  Scenario: Sealed bid ranking
    Tool: Bash
    Steps:
      1. Create 5 bids with varying (price, trust) combos
      2. Run ranking with charter.max_solvers=3
      3. Assert top 3 selected, bottom 2 rejected
      4. cargo test -p jfc-economy -- test_sealed_bid_ranking
    Expected: Deterministic top-3 selection based on scoring formula
    Evidence: .sisyphus/evidence/task-6-bid-ranking.txt

  Scenario: Bid isolation (no leakage)
    Tool: Bash
    Steps:
      1. Submit bid A, then bid B
      2. Assert bid B has no access to bid A's price or approach
      3. cargo test -p jfc-economy -- test_bid_isolation
    Expected: Each bid struct contains only its own data
    Evidence: .sisyphus/evidence/task-6-isolation.txt
  ```

- [x] 7. Solver Agent Spawning

  **What to do**:
  - Each selected solver gets: bounty description, acceptance criteria, graph_query access, git worktree
  - Solvers execute INDEPENDENTLY (no communication between them)
  - Each solver produces a `Solution`: patch/diff + explanation + self-assessment
  - Time-bounded: must complete within charter deadline or forfeit
  - From Agent Hunt: "agents dynamically propose solutions and compete to discharge obligations"
  - **SANDBOX HARDENING** (from Anthropic Mythos): Solvers run in git worktrees with:
    - NO access to files outside worktree
    - NO ability to spawn processes with elevated permissions
    - NO credential searching (block access to ~/.ssh, ~/.config, env vars with secrets)
    - NO network access beyond the LLM API itself
    - Monitor for base64-encoded commands and indirect execution paths
  - **GRACEFUL FORFEIT** (from Anthropic Mythos): Agents have an explicit `abandon()` action — if stuck after N failed attempts, forfeit bounty and return remaining budget rather than persisting destructively

  **Acceptance Criteria**:
  - [ ] Solvers execute in isolated worktrees
  - [ ] No inter-solver communication possible
  - [ ] Timeout enforcement works
  - [ ] Sandbox prevents file access outside worktree
  - [ ] Abandon action returns tokens correctly

  **QA Scenarios**:
  ```
  Scenario: Solver isolation in worktree
    Tool: Bash
    Steps:
      1. Spawn solver with worktree at /tmp/test-solver-1
      2. Verify solver cannot read files from main repo outside worktree
      3. cargo test -p jfc-economy -- test_solver_isolation
    Expected: Any path traversal attempt returns PermissionDenied
    Evidence: .sisyphus/evidence/task-7-isolation.txt

  Scenario: Graceful forfeit returns tokens
    Tool: Bash
    Steps:
      1. Create solver with budget=200, spawn_fee=50 (remaining=150)
      2. Solver consumes 30 tokens of execution then calls abandon()
      3. Assert remaining budget returned = 150 - 30 = 120
      4. cargo test -p jfc-economy -- test_graceful_forfeit
    Expected: 120 tokens returned to bounty pool
    Evidence: .sisyphus/evidence/task-7-forfeit.txt
  ```

- [x] 8. Solution Collection + Ranking

  **What to do**:
  - Collect all solutions after execution phase
  - Pre-filter: does it compile? Do existing tests pass?
  - Rank surviving solutions by: test pass rate, code quality (clippy), cost efficiency
  - Solutions that don't compile are immediately eliminated (no validation needed)
  - From SWE-Debate: "fault propagation traces as localization proposals"
  - **REWARD HACK RESISTANCE** (from Anthropic Mythos): Acceptance criteria evaluated ONLY by mechanistic verification (compiler, test suite, clippy), NEVER by another agent's judgment alone. Agents WILL find novel ways to game soft metrics — all scoring must be based on hard pass/fail signals.
  - **Anti-gaming checks**: Verify solution doesn't just delete failing tests, mock all assertions to true, or modify the grading infrastructure itself

  **Acceptance Criteria**:
  - [ ] Non-compiling solutions auto-rejected
  - [ ] Ranking is deterministic given same inputs
  - [ ] Solutions that delete/modify existing tests are flagged as suspicious
  - [ ] No agent self-reported metric is used in ranking (only mechanistic signals)

  **QA Scenarios**:
  ```
  Scenario: Non-compiling solution rejected
    Tool: Bash
    Steps:
      1. Submit solution with syntax error
      2. Run collection phase
      3. cargo test -p jfc-economy -- test_reject_noncompiling
    Expected: Solution filtered out, not passed to validation phase
    Evidence: .sisyphus/evidence/task-8-reject-bad.txt

  Scenario: Test-deleting solution flagged
    Tool: Bash
    Steps:
      1. Submit solution whose diff removes a test file
      2. Run anti-gaming checks
      3. cargo test -p jfc-economy -- test_flag_test_deletion
    Expected: Solution marked suspicious=true, flagged for human review
    Evidence: .sisyphus/evidence/task-8-anti-gaming.txt
  ```

- [x] 9. Validator Agent Spawning

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

  **QA Scenarios**:
  ```
  Scenario: Self-validation blocked
    Tool: Bash
    Steps:
      1. Agent A produces solution
      2. Attempt to assign Agent A as validator for its own solution
      3. cargo test -p jfc-economy -- test_self_validation_blocked
    Expected: Returns Err(SelfValidationForbidden)
    Evidence: .sisyphus/evidence/task-9-self-validate.txt

  Scenario: Successful invalidation earns reward
    Tool: Bash
    Steps:
      1. Validator finds real flaw (writes failing test)
      2. Adjudication confirms flaw is valid
      3. Assert validator balance increases by validation_reward
      4. cargo test -p jfc-economy -- test_invalidation_reward
    Expected: Validator earns tokens, solver penalized
    Evidence: .sisyphus/evidence/task-9-reward.txt
  ```

- [x] 10. Validation Protocol (3-Round Structured Challenge)

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

  **QA Scenarios**:
  ```
  Scenario: Three-round debate produces verdict
    Tool: Bash
    Steps:
      1. Validator proposes flaw in round 1
      2. Solver defends in round 2
      3. Adjudicator rules in round 3 (runs the proposed test)
      4. cargo test -p jfc-economy -- test_three_round_debate
    Expected: Verdict is Upheld or Dismissed based on test result
    Evidence: .sisyphus/evidence/task-10-debate.txt

  Scenario: Early termination on high confidence
    Tool: Bash
    Steps:
      1. Validator reports "no flaw found, confidence=0.97" after round 1
      2. Assert rounds 2 and 3 are skipped
      3. cargo test -p jfc-economy -- test_early_termination
    Expected: Validation completes after round 1, status=NoFlawFound
    Evidence: .sisyphus/evidence/task-10-early-term.txt
  ```

- [x] 11. Market Cycle Orchestrator

  **What to do**:
  - Orchestrates the full 7-step cycle: Post → Bid → Select → Execute → Validate → Settle → Complete
  - Manages timeouts, retries, error handling
  - Emits events for each state transition (for audit)
  - Handles edge cases: no bids, all solutions fail, validator finds critical flaw

  **Acceptance Criteria**:
  - [ ] Full cycle completes for happy path
  - [ ] Timeout triggers surge pricing and re-post
  - [ ] All state transitions logged

  **QA Scenarios**:
  ```
  Scenario: Happy path full cycle
    Tool: Bash
    Steps:
      1. Post bounty → receives bids → selects solvers → collects solutions → validates → settles
      2. cargo test -p jfc-economy -- test_full_cycle_happy_path
    Expected: Final state is Complete, winner assigned, tokens distributed
    Evidence: .sisyphus/evidence/task-11-happy-path.txt

  Scenario: No bids triggers surge
    Tool: Bash
    Steps:
      1. Post bounty with timeout=0
      2. No bids arrive
      3. Assert state transitions to Bidding again with +15% reward
      4. cargo test -p jfc-economy -- test_no_bids_surge
    Expected: Reward increased, bounty re-posted
    Evidence: .sisyphus/evidence/task-11-surge.txt
  ```

- [x] 12. Settlement Engine

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

  **QA Scenarios**:
  ```
  Scenario: Token conservation (no leak)
    Tool: Bash
    Steps:
      1. Post bounty with reward=1000
      2. Complete full cycle with 3 solvers, 2 validators
      3. Sum all payouts + execution costs + fees
      4. cargo test -p jfc-economy -- test_token_conservation
    Expected: Total distributed = initial bounty (zero leak/creation)
    Evidence: .sisyphus/evidence/task-12-conservation.txt

  Scenario: Payment floor enforced
    Tool: Bash
    Steps:
      1. Solver wins with mediocre quality (payment_ratio would be 0.3)
      2. Assert actual payment is floor(0.5) * reward, not 0.3 * reward
      3. cargo test -p jfc-economy -- test_payment_floor
    Expected: Solver receives at least 50% of bounty reward
    Evidence: .sisyphus/evidence/task-12-floor.txt
  ```

- [x] 13. Anti-Collusion Enforcement

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

  **QA Scenarios**:
  ```
  Scenario: Rubber-stamping detected
    Tool: Bash
    Steps:
      1. Validator approves 10 consecutive solutions without finding any flaw
      2. Statistical check detects approval rate > 2σ from mean
      3. cargo test -p jfc-economy -- test_rubber_stamp_detection
    Expected: Validator flagged, trust penalty applied
    Evidence: .sisyphus/evidence/task-13-rubber-stamp.txt

  Scenario: Griefing detected
    Tool: Bash
    Steps:
      1. Validator rejects 10 consecutive solutions with non-reproducible flaws
      2. Statistical check detects rejection rate > 2σ from mean
      3. cargo test -p jfc-economy -- test_griefing_detection
    Expected: Validator flagged, trust penalty applied
    Evidence: .sisyphus/evidence/task-13-griefing.txt
  ```

- [x] 14. Integration with jfc Swarm/Mailbox System

  **What to do**:
  - Solver agents use existing jfc swarm spawning (worktrees, mailboxes)
  - Market events flow through existing event system
  - Bounty results integrate with existing tool result display
  - Graph engine provides context to both solvers and validators

  **Acceptance Criteria**:
  - [ ] Solvers spawn via existing swarm mechanism
  - [ ] Results appear in standard jfc tool output
  - [ ] Graph context available to market participants

  **QA Scenarios**:
  ```
  Scenario: Solver uses existing swarm worktree
    Tool: Bash
    Steps:
      1. Post bounty in a jfc session with swarm enabled
      2. Verify solver creates a git worktree (git worktree list)
      3. cargo test -p jfc-economy -- test_swarm_integration
    Expected: Worktree created at expected path, solver operates within it
    Evidence: .sisyphus/evidence/task-14-swarm.txt

  Scenario: Graph context available to solver
    Tool: Bash
    Steps:
      1. Solver receives bounty about function "foo"
      2. Solver can call graph_query("fn(\"foo\") | callees | depth 2")
      3. cargo test -p jfc-economy -- test_graph_context_available
    Expected: graph_query returns valid results from jfc-graph engine
    Evidence: .sisyphus/evidence/task-14-graph.txt
  ```

- [x] 15. User-Facing Bounty Tool

  **What to do**:
  - Add `ToolKind::PostBounty` to jfc-ui
  - Input: `{ description: String, budget: u64, acceptance_criteria: String, max_solvers: Option<u8> }`
  - Output: market cycle results (winning solution + validation report)
  - User can also query market status: `ToolKind::MarketStatus`

  **Acceptance Criteria**:
  - [ ] PostBounty tool is callable by LLM
  - [ ] Returns winning solution with validation evidence
  - [ ] Budget is deducted from session budget

  **QA Scenarios**:
  ```
  Scenario: PostBounty tool callable
    Tool: Bash
    Steps:
      1. Build workspace with ToolKind::PostBounty in types.rs
      2. Verify tool dispatch compiles (cargo build --workspace)
      3. cargo test --workspace -- test_post_bounty_dispatches
    Expected: Tool dispatch matches correctly, returns result type
    Evidence: .sisyphus/evidence/task-15-tool-dispatch.txt

  Scenario: Budget deducted after bounty
    Tool: Bash
    Steps:
      1. Start with session budget=5000
      2. Post bounty with budget=1000
      3. After cycle completes, verify session budget reduced by actual spend
      4. cargo test -p jfc-economy -- test_budget_deducted
    Expected: session_budget = 5000 - actual_tokens_consumed
    Evidence: .sisyphus/evidence/task-15-budget.txt
  ```

- [x] 16. Market Status Reporting + Health Score

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

  **QA Scenarios**:
  ```
  Scenario: Health score computation
    Tool: Bash
    Steps:
      1. Run 3 bounty cycles with known outcomes
      2. Compute MarketHealth = Efficiency × Fairness × Trust × BudgetAdherence
      3. cargo test -p jfc-economy -- test_market_health_score
    Expected: Score is multiplicative, matches hand-calculated value
    Evidence: .sisyphus/evidence/task-16-health.txt

  Scenario: Low health triggers alert
    Tool: Bash
    Steps:
      1. Force a scenario where efficiency=0.2 (most solutions fail)
      2. Compute health, assert health < 0.3
      3. Assert alert event emitted
      4. cargo test -p jfc-economy -- test_health_alert
    Expected: HealthAlert event with score and recommendation
    Evidence: .sisyphus/evidence/task-16-alert.txt
  ```

- [x] 17. End-to-End Integration Test

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

  **QA Scenarios**:
  ```
  Scenario: End-to-end fibonacci bounty
    Tool: Bash
    Steps:
      1. Post bounty: "Add pub fn fibonacci(n: u64) -> u64 to lib.rs"
      2. Wait for market cycle to complete
      3. Verify winning solution: cargo test -p jfc-economy -- test_fibonacci_e2e
      4. Assert fibonacci(10) == 55
      5. Assert total_cost < bounty_budget
    Expected: Working fibonacci function, tests pass, within budget
    Evidence: .sisyphus/evidence/task-17-e2e.txt

  Scenario: Budget respected even with multiple solvers
    Tool: Bash
    Steps:
      1. Post bounty with budget=500 tokens, max_solvers=3
      2. Complete cycle
      3. Assert sum(all_agent_costs) <= 500
      4. cargo test -p jfc-economy -- test_e2e_budget_respected
    Expected: Total spend never exceeds allocated budget
    Evidence: .sisyphus/evidence/task-17-budget.txt
  ```

---

## Final Verification Wave

- [x] F1. **Plan Compliance Audit** — verify all Must Have implemented, all Must NOT Have absent

  **QA Scenario**:
  ```
  Scenario: All Must Have items present
    Tool: Bash
    Steps:
      1. Run cargo test -p jfc-economy -- test_competitive (≥2 solvers)
      2. Run cargo test -p jfc-economy -- test_adversarial (validator challenges)
      3. Run cargo test -p jfc-economy -- test_budget_gate (hard constraint)
      4. Run cargo test -p jfc-economy -- test_trust (scoring updates)
      5. Run cargo test -p jfc-economy -- test_anti_collusion (self-validation blocked)
      6. Run cargo test -p jfc-economy -- test_audit (trail exists)
    Expected: All 6 tests pass
    Evidence: .sisyphus/evidence/f1-compliance.txt

  Scenario: All Must NOT Have items absent
    Tool: Bash
    Steps:
      1. grep -r "blockchain\|ethereum\|web3" crates/jfc-economy/src/ → 0 matches
      2. grep -r "reproduce\|evolve\|spawn_child" crates/jfc-economy/src/ → 0 matches
      3. Verify no network calls beyond LLM API in sandbox
    Expected: Zero matches for forbidden patterns
    Evidence: .sisyphus/evidence/f1-must-not-have.txt
  ```

- [x] F2. **Code Quality Review** — clippy clean, tests pass, no unwrap in prod

  **QA Scenario**:
  ```
  Scenario: Code quality gates
    Tool: Bash
    Steps:
      1. cargo clippy -p jfc-economy -- -D warnings
      2. cargo test -p jfc-economy
      3. grep -rn "\.unwrap()" crates/jfc-economy/src/ --include="*.rs" | grep -v test | grep -v "#\[cfg(test)" → 0 matches
      4. cargo build --workspace
    Expected: clippy clean, all tests pass, no unwrap in non-test code, workspace builds
    Evidence: .sisyphus/evidence/f2-quality.txt
  ```

- [x] F3. **Real QA — Full Bounty Lifecycle** (agent-executed, no human)

  **QA Scenario**:
  ```
  Scenario: Execute a real bounty end-to-end
    Tool: Bash
    Steps:
      1. Initialize GraphSession from crates/jfc-graph/tests/fixtures/
      2. Post bounty: "Add a function pub fn double(x: i32) -> i32 { x * 2 } to a new file"
      3. Run market cycle with 2 mock solvers (hardcoded responses) + 1 mock validator
      4. Assert winning solution contains "fn double"
      5. Assert total tokens consumed < budget
      6. Assert trust scores updated for all 3 participants
      7. Assert audit log has ≥ 7 entries (one per state transition)
      8. cargo test -p jfc-economy -- test_real_bounty_lifecycle
    Expected: All assertions pass, complete audit trail
    Evidence: .sisyphus/evidence/f3-real-qa.txt
  ```

- [x] F4. **Scope Fidelity Check** — no blockchain, no evolution, no unbounded spend

  **QA Scenario**:
  ```
  Scenario: Scope boundaries enforced
    Tool: Bash
    Steps:
      1. grep -r "async fn reproduce\|fn evolve\|fn mutate_agent" crates/jfc-economy/ → 0 matches
      2. grep -r "reqwest\|hyper\|tokio::net" crates/jfc-economy/src/ → 0 (no external network)
      3. Verify TokenLedger has no negative balance path: cargo test -- test_no_negative_balance
      4. Verify max 5 solvers enforced: cargo test -- test_max_solvers_cap
    Expected: All scope constraints verified mechanistically
    Evidence: .sisyphus/evidence/f4-scope.txt
  ```

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
| **Anthropic Claude Mythos System Card** | Reward hacking, sandbox escape, deception, overeager persistence, strategic manipulation feature activations |
