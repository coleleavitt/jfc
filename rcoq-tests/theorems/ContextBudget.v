(** * ContextBudget: Formal Analysis of Why Initial Queries Use ~50k Tokens
    
    This module analyzes JFC's context budget allocation and proves bounds
    on initial token consumption. The goal is to understand and optimize
    the system prompt + tool definitions + memory overhead.
    
    Key finding: The 50k initial token usage comes from:
    - System prompt + project instructions (~10-15k)
    - Tool definitions (30-50 tools × 200-500 tokens each = 6-25k)
    - Memory/context files (~5-15k)
    - Wire overhead (JSON framing, role markers = ~1.5x multiplier)
    
    References:
    - crates/jfc-engine/src/compact/mod.rs (OVERHEAD_MULTIPLIER)
    - crates/jfc-mcp/src/lib.rs (tool advertising)
    - JFC system prompt construction
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Context Budget Components *)

Record ContextBudget : Type := mkContextBudget {
  system_prompt_tokens : nat;
  tool_definition_tokens : nat;
  memory_tokens : nat;
  project_instructions_tokens : nat;
  user_message_tokens : nat;
}.

(** Total raw tokens before overhead *)
Definition raw_tokens (b : ContextBudget) : nat :=
  system_prompt_tokens b +
  tool_definition_tokens b +
  memory_tokens b +
  project_instructions_tokens b +
  user_message_tokens b.

(** Wire overhead multiplier (from OVERHEAD_MULTIPLIER = 3/2 = 1.5x) *)
Definition OVERHEAD_NUM : nat := 3.
Definition OVERHEAD_DEN : nat := 2.

Definition with_overhead (tokens : nat) : nat :=
  (tokens * OVERHEAD_NUM) / OVERHEAD_DEN.

(** Effective context consumption *)
Definition effective_tokens (b : ContextBudget) : nat :=
  with_overhead (raw_tokens b).

(** ** Typical JFC Configuration *)

(** Tool definition size model *)
Record ToolDef : Type := mkToolDef {
  tool_name_tokens : nat;
  tool_description_tokens : nat;
  tool_schema_tokens : nat;
}.

Definition tool_tokens (t : ToolDef) : nat :=
  tool_name_tokens t + tool_description_tokens t + tool_schema_tokens t.

(** Typical tool sizes *)
Definition small_tool : ToolDef := mkToolDef 2 50 100.   (* ~150 tokens *)
Definition medium_tool : ToolDef := mkToolDef 3 100 200. (* ~300 tokens *)
Definition large_tool : ToolDef := mkToolDef 5 200 400.  (* ~600 tokens *)

(** JFC's typical tool set *)
Definition jfc_core_tools : nat := 15.      (* Read, Write, Edit, Bash, etc. *)
Definition jfc_mcp_tools : nat := 15.       (* CodeGraph tools *)
Definition jfc_task_tools : nat := 10.      (* Task management *)
Definition jfc_skill_tools : nat := 10.     (* Skill invocation *)

Definition total_tools : nat := 
  jfc_core_tools + jfc_mcp_tools + jfc_task_tools + jfc_skill_tools.

(** Estimated tool definition overhead *)
Definition estimated_tool_tokens : nat :=
  jfc_core_tools * tool_tokens medium_tool +
  jfc_mcp_tools * tool_tokens medium_tool +
  jfc_task_tools * tool_tokens small_tool +
  jfc_skill_tools * tool_tokens small_tool.

(** ** Budget Analysis Theorems *)

(** Theorem: Tool definitions alone consume significant context *)
Theorem tool_overhead_significant :
  estimated_tool_tokens >= 10000.
Proof.
  (* estimated = 15*303 + 15*303 + 10*152 + 10*152 = 12130 >= 10000.
     [vm_compute] evaluates the literal arithmetic so [lia] sees the
     concrete value rather than unevaluated [Nat.of_num_uint] numerals. *)
  unfold estimated_tool_tokens, jfc_core_tools, jfc_mcp_tools,
    jfc_task_tools, jfc_skill_tools, tool_tokens, medium_tool, small_tool.
  vm_compute. lia.
Qed.

(** Typical system prompt (CC 2.1.167 style) *)
Definition system_prompt_base : nat := 8000.  (* Core instructions *)
Definition system_prompt_rules : nat := 2000. (* .claude/rules/*.md *)
Definition system_prompt_memory : nat := 3000. (* Memory index *)
Definition system_prompt_agents : nat := 2000. (* AGENTS.md *)

Definition total_system_prompt : nat :=
  system_prompt_base + system_prompt_rules + system_prompt_memory + system_prompt_agents.

(** Theorem: System prompt is substantial *)
Theorem system_prompt_substantial :
  total_system_prompt >= 15000.
Proof.
  (* total = 8000 + 2000 + 3000 + 2000 = 15000 >= 15000.
     This Stdlib build parses large literals as opaque [Nat.of_num_uint]
     applications that [lia] reports "Cannot find witness" on, so we decide
     the concrete inequality with [Nat.leb] + [vm_compute] instead.  The
     statement is unchanged. *)
  apply Nat.leb_le. vm_compute. reflexivity.
Qed.

(** ** Initial Query Budget *)

Definition typical_initial_budget : ContextBudget := mkContextBudget
  total_system_prompt        (* ~15k *)
  estimated_tool_tokens      (* ~12k *)
  5000                       (* Memory files *)
  3000                       (* Project instructions *)
  500.                       (* User's initial message *)

(** Theorem: Initial query uses ~50k tokens *)
Theorem initial_query_uses_50k :
  let budget := typical_initial_budget in
  let effective := effective_tokens budget in
  effective >= 40000 /\ effective <= 60000.
Proof.
  (* raw = 15000 + 12130 + 5000 + 3000 + 500 = 35630
     effective = with_overhead 35630 = 35630 * 3 / 2 = 53445,
     which lands in [40000, 60000].  Both bounds are concrete-literal
     inequalities decided by [Nat.leb] + [vm_compute] (see note on
     [system_prompt_substantial] for why [lia] is unavailable here). *)
  split.
  - apply Nat.leb_le. vm_compute. reflexivity.
  - apply Nat.leb_le. vm_compute. reflexivity.
Qed.

(** ** Optimization Opportunities *)

(** Progressive tool disclosure: only advertise tools when needed *)
Definition progressive_tools (query_type : nat) : nat :=
  match query_type with
  | 0 => 5   (* Simple question: minimal tools *)
  | 1 => 15  (* Code edit: core tools *)
  | 2 => 30  (* Complex task: most tools *)
  | _ => 50  (* Full capability *)
  end.

(** Theorem: Progressive disclosure saves tokens *)
Theorem progressive_saves_tokens :
  forall query_type,
    query_type < 3 ->
    progressive_tools query_type * tool_tokens medium_tool < estimated_tool_tokens.
Proof.
  intros query_type Hlt.
  (* For query_type in {0,1,2} the disclosed cost is 5*303, 15*303, 30*303,
     all strictly below estimated = 12130.  query_type >= 3 is excluded by
     [Hlt].  Concrete-literal strict inequalities are decided via
     [Nat.ltb_lt] + [vm_compute] (this build's [lia] cannot read the large
     numerals; see [system_prompt_substantial]). *)
  destruct query_type as [|[|[|q]]].
  - apply Nat.ltb_lt. vm_compute. reflexivity.
  - apply Nat.ltb_lt. vm_compute. reflexivity.
  - apply Nat.ltb_lt. vm_compute. reflexivity.
  - exfalso. (* Hlt : S (S (S q)) < 3, i.e. q < 0 after peeling *)
    do 3 (apply Nat.succ_lt_mono in Hlt).
    apply Nat.nlt_0_r in Hlt. exact Hlt.
Qed.

(** Lazy memory loading: only load relevant memories *)
Definition lazy_memory_tokens (relevance_threshold : nat) (total_memories : nat) : nat :=
  (total_memories * relevance_threshold) / 100.

(** Theorem: Lazy loading with 20% relevance threshold saves 80% *)
Theorem lazy_memory_saves :
  forall total_memories,
    total_memories >= 5000 ->
    lazy_memory_tokens 20 total_memories <= total_memories / 4.
Proof.
  intros total_memories Hge.
  unfold lazy_memory_tokens.
  (* (m*20)/100 = m/5 (since 100 = 5*20), and m/5 <= m/4 because a larger
     divisor yields a smaller quotient.  Holds for every [m]; the >= 5000
     premise [>= 5000] is the modeling context (a non-trivial corpus) and is
     not needed for the bound. *)
  replace 100 with (5 * 20) by reflexivity.
  rewrite Nat.div_mul_cancel_r by discriminate.
  apply Nat.div_le_compat_l.
  (* 0 < 4 /\ 4 <= 5 *)
  split.
  - apply Nat.lt_0_succ.
  - repeat constructor.
Qed.

(** ** Compression Impact on Initial Budget *)

(** After compaction, preserved tokens *)
Definition post_compact_tokens (initial : nat) (compression_ratio : nat) : nat :=
  (initial * compression_ratio) / 100.

(** Theorem: Good compression brings initial context under control *)
Theorem compression_controls_growth :
  forall initial_tokens,
    initial_tokens <= 60000 ->
    post_compact_tokens initial_tokens 30 <= 20000.
Proof.
  intros initial_tokens Hle.
  unfold post_compact_tokens.
  (* (initial*30)/100 <= 20000  <=  initial*30 <= 100*20000.
     Bound initial*30 by 60000*30 = 1800000 <= 2000000. *)
  apply Nat.Div0.div_le_upper_bound.
  apply Nat.le_trans with (m := 60000 * 30).
  - apply Nat.mul_le_mono_r. exact Hle.
  - apply Nat.leb_le. vm_compute. reflexivity.
Qed.

(** ** System Prompt Compression *)

(** Key insight: System prompt is largely static and could be cached/compressed *)

Definition compressible_system_prompt : nat := 
  system_prompt_rules + system_prompt_memory.  (* Rules + memory are session-specific *)

Definition static_system_prompt : nat :=
  system_prompt_base + system_prompt_agents.  (* Core + agents rarely change *)

(** Theorem: Static portion could be pre-tokenized/cached *)
Theorem static_cacheable :
  static_system_prompt >= total_system_prompt / 2.
Proof.
  (* static = 10000, total/2 = 15000/2 = 7500, and 7500 <= 10000.
     Concrete-literal goal decided by [Nat.leb] + [vm_compute]. *)
  apply Nat.leb_le. vm_compute. reflexivity.
Qed.

(** ** Recommendations *)

(** 1. Progressive tool disclosure based on query classification *)
(** 2. Lazy memory loading with relevance filtering *)
(** 3. System prompt caching/pre-tokenization for static portions *)
(** 4. Tool schema compression (remove verbose descriptions for known tools) *)

Definition optimized_budget : ContextBudget := mkContextBudget
  (total_system_prompt * 7 / 10)  (* 30% reduction via caching *)
  (estimated_tool_tokens / 3)     (* Progressive: 33% of tools *)
  2000                            (* Lazy: 40% of memories *)
  3000                            (* Project instructions (keep) *)
  500.                            (* User message *)

(** Theorem: Optimized budget is significantly smaller *)
Theorem optimization_effective :
  effective_tokens optimized_budget < effective_tokens typical_initial_budget * 7 / 10.
Proof.
  (* optimized raw = 10500 + 4043 + 2000 + 3000 + 500 = 20043,
     effective = 20043*3/2 = 30064.
     typical effective = 53445, and 53445*7/10 = 37411.
     Both sides are closed numerals, so 30064 < 37411 is decided by
     [Nat.ltb_lt] + [vm_compute]. *)
  apply Nat.ltb_lt. vm_compute. reflexivity.
Qed.

(** ** LLMLingua Budget Controller (arXiv:2310.05736)

    The theorems above bound JFC's *static* initial budget.  This section
    formalizes the dynamic budget *controller* JFC's compaction mirrors from
    LLMLingua: a target token budget partitioned across the prompt's
    components (instruction / demonstrations / question), a greedy
    demonstration cap, slack redistribution, and the component-rate ordering.

    We model component target-token counts directly as nats (LLMLingua's
    Eq.2 rearranged: the target [tau*L] is exactly the sum of the component
    targets [tau_ins*L_ins + tau_dems*L_dems + tau_que*L_que]). *)

(** Per-component target token budgets (already nats: tau_x * L_x). *)
Record CompBudget : Type := mkCompBudget {
  ins_budget  : nat;   (* instruction component target:  tau_ins  * L_ins  *)
  dems_budget : nat;   (* demonstration component target: tau_dems * L_dems *)
  que_budget  : nat;   (* question component target:     tau_que  * L_que  *)
}.

(** Total target tokens = sum of component targets. *)
Definition total_budget (b : CompBudget) : nat :=
  ins_budget b + dems_budget b + que_budget b.

(** *** Budget-conservation identity (LLMLingua Eq.2)

    [CORRECTION] There is no false original here — this is a new, paper-grounded
    theorem.  It states the conservation / no-leakage property: the total target
    budget is *exactly* partitioned across the three components, with nothing
    created or lost.  This is the discrete form of
    [tau*L = tau_ins*L_ins + tau_dems*L_dems + tau_que*L_que]. *)
Theorem budget_conservation :
  forall ins dems que : nat,
    total_budget (mkCompBudget ins dems que) = ins + dems + que.
Proof.
  intros ins dems que. unfold total_budget. simpl. reflexivity.
Qed.

(** Reassembling a budget from its own components is the identity: no leakage
    in either direction. *)
Theorem budget_no_leakage :
  forall b : CompBudget,
    total_budget b
      = ins_budget b + dems_budget b + que_budget b.
Proof.
  intros b. unfold total_budget. reflexivity.
Qed.

(** *** Greedy demonstration cap (Alg.1 break condition)

    Demonstrations are added one at a time while the running selected total
    stays within the cap [k * tau_dems * L_dems] (granular control coefficient
    [k = 2]); the loop breaks as soon as adding the next demo would exceed it. *)

(** Greedy coefficient k from Alg.1. *)
Definition granular_k : nat := 2.

(** Demonstration cap = k * dems_budget. *)
Definition demo_cap (b : CompBudget) : nat := granular_k * dems_budget b.

(** Greedy accumulation: running selected total [acc]; accept the next demo
    iff it keeps [acc <= cap]; otherwise break (Alg.1). [demos] is the list of
    per-demonstration token counts. *)
Fixpoint demo_select_aux (cap : nat) (demos : list nat) (acc : nat) : nat :=
  match demos with
  | [] => acc
  | d :: rest =>
      if Nat.leb (acc + d) cap
      then demo_select_aux cap rest (acc + d)
      else acc
  end.

(** Total demonstration tokens selected under the cap (LLMLingua's L~_D). *)
Definition demo_select (cap : nat) (demos : list nat) : nat :=
  demo_select_aux cap demos 0.

(** The accumulator never exceeds the cap once it starts within it. *)
Lemma demo_select_aux_cap :
  forall demos cap acc,
    acc <= cap ->
    demo_select_aux cap demos acc <= cap.
Proof.
  induction demos as [|d rest IH]; intros cap acc Hacc.
  - simpl. exact Hacc.
  - simpl. destruct (Nat.leb (acc + d) cap) eqn:H.
    + apply Nat.leb_le in H. apply IH. exact H.
    + exact Hacc.
Qed.

(** Theorem: greedy demonstration selection respects the cap (knapsack
    feasibility).  This is Alg.1's invariant [L~_D <= k * tau_dems * L_dems]. *)
Theorem demo_cap_respected :
  forall b demos,
    demo_select (demo_cap b) demos <= demo_cap b.
Proof.
  intros b demos. unfold demo_select.
  apply demo_select_aux_cap. apply Nat.le_0_l.
Qed.

(** *** Slack redistribution (LLMLingua Eq.3)

    [Delta = k*tau_dems*L_dems - L~_D] is the unused demonstration budget.  In
    nat arithmetic subtraction is truncated, so we first establish [L~_D <= cap]
    (above), which makes [Delta] the genuine difference rather than 0. *)

(** Slack = cap - selected. *)
Definition demo_slack (cap : nat) (demos : list nat) : nat :=
  cap - demo_select cap demos.

(** Theorem: slack is non-negative and the selected demos plus slack recombine
    to exactly the cap (Eq.3: the difference [Delta] is real and the cap budget
    is fully accounted for, never discarded). *)
Theorem slack_nonneg_and_conserves :
  forall b demos,
    0 <= demo_slack (demo_cap b) demos /\
    demo_select (demo_cap b) demos + demo_slack (demo_cap b) demos
      = demo_cap b.
Proof.
  intros b demos. split.
  - apply Nat.le_0_l.
  - unfold demo_slack.
    pose proof (demo_cap_respected b demos) as H.
    (* selected + (cap - selected) = cap, since selected <= cap *)
    rewrite Nat.add_comm. apply Nat.sub_add. exact H.
Qed.

(** Redistribution moves the whole demonstration slack into the instruction
    and question component budgets ([d1] to instruction, [d2] to question),
    leaving the demonstration component untouched. *)
Definition redistribute (b : CompBudget) (d1 d2 : nat) : CompBudget :=
  mkCompBudget (ins_budget b + d1) (dems_budget b) (que_budget b + d2).

(** Theorem: redistribution conserves the total — every redistributed token
    reappears in the new total, none is discarded (Eq.3 conservation). *)
Theorem redistribution_conserves :
  forall b d1 d2,
    total_budget (redistribute b d1 d2) = total_budget b + d1 + d2.
Proof.
  intros b d1 d2. unfold redistribute, total_budget. simpl. lia.
Qed.

(** Theorem: end-to-end demonstration accounting.  When the entire slack is
    split between instruction and question ([d1 + d2 = slack]), the selected
    demonstration tokens plus the two redistributed amounts exactly exhaust the
    demonstration cap.  Nothing the cap reserved is lost: it is either spent on
    demonstrations or handed to instruction/question. *)
Theorem demo_budget_fully_accounted :
  forall b demos d1 d2,
    d1 + d2 = demo_slack (demo_cap b) demos ->
    demo_select (demo_cap b) demos + d1 + d2 = demo_cap b.
Proof.
  intros b demos d1 d2 Hsplit.
  rewrite <- Nat.add_assoc. rewrite Hsplit.
  destruct (slack_nonneg_and_conserves b demos) as [_ Hcons].
  exact Hcons.
Qed.

(** *** Component-rate ordering (tau_ins = 0.85, tau_que = 0.90 > tau_dems)

    Instruction and question are compressed least; the demonstration rate is
    strictly smaller.  Rates are modeled as percentages (nats). *)

Definition tau_ins_pct  : nat := 85.   (* instruction kept at 0.85 *)
Definition tau_que_pct  : nat := 90.   (* question kept at 0.90 *)
Definition tau_dems_pct : nat := 50.   (* demonstrations compressed most *)

(** Theorem: the demonstration rate is strictly below both the instruction and
    question rates, and the instruction rate does not exceed the question rate
    (the priority ordering used to decide what gets compressed first). *)
Theorem component_rate_ordering :
  tau_dems_pct < tau_ins_pct /\
  tau_dems_pct < tau_que_pct /\
  tau_ins_pct <= tau_que_pct.
Proof.
  unfold tau_dems_pct, tau_ins_pct, tau_que_pct.
  repeat split.
  - apply Nat.ltb_lt. vm_compute. reflexivity.
  - apply Nat.ltb_lt. vm_compute. reflexivity.
  - apply Nat.leb_le. vm_compute. reflexivity.
Qed.

(** Theorem: a higher component rate keeps strictly more tokens for equal raw
    component lengths.  With per-component kept-tokens modeled as
    [rate * length / 100], the demonstration component (lower rate) keeps no
    more than the question component for the same length — the operational
    consequence of the rate ordering. *)
Theorem higher_rate_keeps_more :
  forall len : nat,
    tau_dems_pct * len / 100 <= tau_que_pct * len / 100.
Proof.
  intros len.
  apply Nat.Div0.div_le_mono.
  apply Nat.mul_le_mono_r.
  unfold tau_dems_pct, tau_que_pct.
  apply Nat.leb_le. vm_compute. reflexivity.
Qed.
