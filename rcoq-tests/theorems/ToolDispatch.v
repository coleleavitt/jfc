(** * ToolDispatch: Formal Model of Batched Tool Dispatch
    
    This module formalizes JFC's tool dispatch system including:
    - Batch ordering and execution
    - Approval queue semantics
    - Local advisor dispatch context
    - Tool result aggregation
    
    References:
    - crates/jfc-engine/src/stream/tool_dispatch.rs
    - crates/jfc-engine/src/runtime/event_loop/handlers/stream_tool.rs
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Tool Use Status *)

Inductive ToolStatus : Type :=
  | ToolPending      (* Waiting to execute *)
  | ToolExecuting    (* Currently running *)
  | ToolCompleted    (* Finished successfully *)
  | ToolFailed       (* Execution failed *)
  | ToolCancelled.   (* User cancelled *)

Definition is_terminal_tool (s : ToolStatus) : bool :=
  match s with
  | ToolCompleted | ToolFailed | ToolCancelled => true
  | _ => false
  end.

(** ** Tool Use Record *)

Record ToolUse : Type := mkToolUse {
  tool_id : nat;
  tool_name : nat;
  tool_input : nat;  (* Simplified: hash of input JSON *)
  tool_status : ToolStatus;
  tool_requires_approval : bool;
  tool_approved : option bool;  (* None = not yet decided *)
}.

(** ** Tool Batch *)

Definition ToolBatch := list ToolUse.

(** Batch is ready when all tools are either approved or auto-approved *)
Definition batch_ready (batch : ToolBatch) : bool :=
  forallb (fun t =>
    negb (tool_requires_approval t) ||
    match tool_approved t with Some true => true | _ => false end
  ) batch.

(** Batch is complete when all tools are terminal *)
Definition batch_complete (batch : ToolBatch) : bool :=
  forallb (fun t => is_terminal_tool (tool_status t)) batch.

(** ** Approval Queue *)

Record ApprovalEntry : Type := mkApproval {
  approval_tool_id : nat;
  approval_decision : option bool;  (* None = pending *)
}.

Definition ApprovalQueue := list ApprovalEntry.

(** Resolve head of approval queue *)
Definition resolve_head (queue : ApprovalQueue) (decision : bool) : ApprovalQueue :=
  match queue with
  | [] => []
  | entry :: rest =>
      mkApproval (approval_tool_id entry) (Some decision) :: rest
  end.

(** Advance queue (remove resolved head) *)
Definition advance_queue (queue : ApprovalQueue) : ApprovalQueue :=
  match queue with
  | [] => []
  | entry :: rest =>
      match approval_decision entry with
      | Some _ => rest
      | None => queue  (* Can't advance unresolved *)
      end
  end.

(** ** Approval Queue Theorems *)

(** Theorem: Resolve head sets decision *)
Theorem resolve_sets_decision :
  forall queue decision entry rest,
    queue = entry :: rest ->
    approval_decision entry = None ->
    let queue' := resolve_head queue decision in
    match queue' with
    | [] => False
    | e :: _ => approval_decision e = Some decision
    end.
Proof.
  intros queue decision entry rest Heq Hnone.
  subst queue.
  unfold resolve_head.
  simpl. reflexivity.
Qed.

(** Theorem: Advance requires resolution *)
Theorem advance_requires_resolution :
  forall queue entry rest,
    queue = entry :: rest ->
    approval_decision entry = None ->
    advance_queue queue = queue.
Proof.
  intros queue entry rest Heq Hnone.
  subst queue.
  unfold advance_queue.
  rewrite Hnone.
  reflexivity.
Qed.

(** Theorem: Advance after resolution removes head *)
Theorem advance_after_resolution :
  forall queue entry rest d,
    queue = entry :: rest ->
    approval_decision entry = Some d ->
    advance_queue queue = rest.
Proof.
  intros queue entry rest d Heq Hsome.
  subst queue.
  unfold advance_queue.
  rewrite Hsome.
  reflexivity.
Qed.

(** ** Batch Dispatch State Machine *)

Inductive BatchState : Type :=
  | BatchPending       (* Not yet started *)
  | BatchApproving     (* Waiting for approvals *)
  | BatchExecuting     (* Tools running *)
  | BatchComplete      (* All tools finished *)
  | BatchCancelled.    (* User cancelled batch *)

Definition batch_transition (s : BatchState) (event : nat) : option BatchState :=
  match s, event with
  | BatchPending, 0 (* start *) => Some BatchApproving
  | BatchApproving, 1 (* all_approved *) => Some BatchExecuting
  | BatchApproving, 2 (* cancelled *) => Some BatchCancelled
  | BatchExecuting, 3 (* all_done *) => Some BatchComplete
  | BatchExecuting, 2 (* cancelled *) => Some BatchCancelled
  | _, _ => None
  end.

(** Theorem: Terminal batch states are absorbing *)
Theorem batch_terminal_absorbing :
  forall event,
    batch_transition BatchComplete event = None /\
    batch_transition BatchCancelled event = None.
Proof.
  intros event.
  split; unfold batch_transition; destruct event; reflexivity.
Qed.

(** ** Tool Execution Order *)

(** Tools in a batch can execute in parallel, but results are collected in order *)
Definition execution_order (batch : ToolBatch) : list nat :=
  map tool_id batch.

(** Result collection maintains order *)
Record BatchResult : Type := mkBatchResult {
  result_tool_id : nat;
  result_success : bool;
  result_output : nat;
}.

Definition collect_results (batch : ToolBatch) (results : list BatchResult) : bool :=
  (* Check that results match batch order *)
  let expected := execution_order batch in
  let actual := map result_tool_id results in
  if list_eq_dec Nat.eq_dec expected actual then true else false.

(** Theorem: Results must match batch order *)
Theorem results_match_order :
  forall batch results,
    collect_results batch results = true ->
    map result_tool_id results = execution_order batch.
Proof.
  intros batch results Hcollect.
  unfold collect_results in Hcollect.
  destruct (list_eq_dec Nat.eq_dec (execution_order batch) (map result_tool_id results)) as [e|n].
  - symmetry. exact e.
  - discriminate.
Qed.

(** ** Local Advisor Dispatch *)

(** Advisor provides guidance on tool execution *)
Record AdvisorContext : Type := mkAdvisorContext {
  advisor_model : nat;
  advisor_conversation_snapshot : nat;
  advisor_pending_tools : list nat;
}.

(** Advisor decision types *)
Inductive AdvisorDecision : Type :=
  | Proceed                 (* Execute as planned *)
  | Modify (new_input : nat) (* Modify tool input *)
  | Skip                    (* Skip this tool *)
  | AbortBatch.             (* Abort entire batch *)

(** Helper: combine two lists with a function *)
Fixpoint combine_with {A B C : Type} (f : A -> B -> C) (l1 : list A) (l2 : list B) : list C :=
  match l1, l2 with
  | [], _ => []
  | _, [] => []
  | x :: xs, y :: ys => f x y :: combine_with f xs ys
  end.

(** Apply advisor decision to batch *)
Definition apply_advisor (batch : ToolBatch) (decisions : list AdvisorDecision) : ToolBatch :=
  combine_with (fun t d =>
    match d with
    | Proceed => t
    | Skip => mkToolUse (tool_id t) (tool_name t) (tool_input t) ToolCancelled
                        (tool_requires_approval t) (tool_approved t)
    | Modify new_input => mkToolUse (tool_id t) (tool_name t) new_input (tool_status t)
                                    (tool_requires_approval t) (tool_approved t)
    | AbortBatch => mkToolUse (tool_id t) (tool_name t) (tool_input t) ToolCancelled
                         (tool_requires_approval t) (tool_approved t)
    end
  ) batch decisions.

(** Theorem: Abort cancels all tools.

    Applying an [AbortBatch] decision to every tool in the batch drives every
    resulting tool into the [ToolCancelled] terminal state.  This is the safety
    guarantee that an advisor abort really stops the whole batch -- no tool is
    left in a runnable (pending/executing) state. *)
Theorem abort_cancels_all :
  forall batch,
    let decisions := repeat AbortBatch (length batch) in
    let result := apply_advisor batch decisions in
    forallb (fun t => match tool_status t with ToolCancelled => true | _ => false end) result = true.
Proof.
  intros batch. simpl.
  unfold apply_advisor.
  (* Each tool gets an AbortBatch decision, setting status to ToolCancelled. *)
  induction batch as [|t rest IH]; simpl.
  - reflexivity.
  - exact IH.
Qed.

(** ** Tool Result Deduplication *)

(** Detect duplicate tool results (from turns.rs) *)
Record ToolResultId : Type := mkToolResultId {
  result_id : nat;
}.

Definition has_duplicate (ids : list nat) : bool :=
  negb (
    let unique := nodup Nat.eq_dec ids in
    Nat.eqb (length ids) (length unique)
  ).

(** [nodup] never produces a longer list than its input. *)
Lemma nodup_length_le :
  forall (A : Type) (decA : forall x y : A, {x = y} + {x <> y}) (l : list A),
    length (nodup decA l) <= length l.
Proof.
  intros A decA l. induction l as [|x xs IH]; simpl.
  - lia.
  - destruct (in_dec decA x xs); simpl; lia.
Qed.

(** If [nodup] removed nothing (lengths equal), the list was already duplicate
    free.  This is the converse of [nodup_fixed_point]. *)
Lemma nodup_full :
  forall (A : Type) (decA : forall x y : A, {x = y} + {x <> y}) (l : list A),
    length (nodup decA l) = length l -> NoDup l.
Proof.
  intros A decA l. induction l as [|x xs IH]; simpl; intro H.
  - constructor.
  - destruct (in_dec decA x xs) as [Hin|Hnin].
    + (* x is a duplicate: nodup dropped it, so lengths cannot match *)
      pose proof (nodup_length_le A decA xs). lia.
    + constructor.
      * exact Hnin.
      * apply IH. simpl in H. lia.
Qed.

(** Theorem: No duplicates means all unique.

    A genuine dedup-soundness invariant: when the duplicate detector reports
    [false], the id list really does satisfy [NoDup] (every result id occurs at
    most once), so no tool result is collected twice. *)
Theorem no_duplicate_means_unique :
  forall ids,
    has_duplicate ids = false ->
    NoDup ids.
Proof.
  intros ids Hno.
  unfold has_duplicate in Hno.
  apply negb_false_iff in Hno.
  apply Nat.eqb_eq in Hno.
  apply (nodup_full nat Nat.eq_dec).
  symmetry. exact Hno.
Qed.

(** ** Progressive Tool Selection *)

(** Select tools based on intent and history (from catalog.rs) *)
Record ToolDef : Type := mkToolDef {
  def_name : nat;
  def_description : nat;
  def_schema : nat;
}.

Definition ToolCatalog := list ToolDef.

(** Historical tool usage *)
Definition historical_tools (history : list nat) : list nat :=
  nodup Nat.eq_dec history.

(** Does a tool match the (optional) intent? *)
Definition matches_intent (intent : option nat) (d : ToolDef) : bool :=
  match intent with
  | None => false
  | Some n => Nat.eqb (def_name d) n
  end.

(** Select tools progressively.

    [CORRECTION] The original model was [firstn max_tools catalog], which
    *ignores* both [intent] and [history] -- so the documented priority
    ("1. tools matching intent, 2. historical tools, 3. core tools") was not
    implemented, and [history_doesnt_starve] was FALSE: a discovered
    intent-matching tool sitting past index [max_tools] in catalog order would
    be dropped.  We make [progressive_select] actually honor the priority: the
    intent-matching tools are pulled to the front (in catalog order), then the
    rest of the catalog fills the remaining budget.  This is what makes the
    anti-starvation theorem true. *)
Definition progressive_select (catalog : ToolCatalog) (history : list nat)
    (intent : option nat) (max_tools : nat) : list ToolDef :=
  let matched := filter (matches_intent intent) catalog in
  let others := filter (fun d => negb (matches_intent intent d)) catalog in
  firstn max_tools (matched ++ others).

(** Number of catalog tools that match the intent. *)
Definition intent_match_count (catalog : ToolCatalog) (intent : option nat) : nat :=
  length (filter (matches_intent intent) catalog).

(** Every intent-matching tool sits in the prioritized prefix. *)
Lemma matched_in_firstn :
  forall catalog intent k discovered,
    In discovered catalog ->
    matches_intent intent discovered = true ->
    length (filter (matches_intent intent) catalog) <= k ->
    In discovered (firstn k
       (filter (matches_intent intent) catalog
        ++ filter (fun d => negb (matches_intent intent d)) catalog)).
Proof.
  intros catalog intent k discovered Hin Hmatch Hlen.
  set (matched := filter (matches_intent intent) catalog) in *.
  set (others := filter (fun d => negb (matches_intent intent d)) catalog).
  (* [discovered] is in [matched] because it matches and is in the catalog. *)
  assert (Hinm : In discovered matched).
  { subst matched. apply filter_In. split; [exact Hin | exact Hmatch]. }
  (* Split the prefix across the concatenation. *)
  rewrite firstn_app.
  apply in_or_app. left.
  (* [length matched <= k] means [firstn k matched = matched]. *)
  pose proof (firstn_all2 matched Hlen) as Hfa.
  rewrite Hfa.
  exact Hinm.
Qed.

(** Theorem: History doesn't starve intent-discovered tools.

    [CORRECTION] (see [progressive_select]).  The original required only
    [max_tools >= 1], which is too weak to guarantee inclusion of a *specific*
    intent-matching tool when several tools match.  The true, strong invariant:
    so long as the budget covers the intent-matching set
    ([max_tools >= intent_match_count]), every intent-matching discovered tool
    is selected -- the history/core fill can never starve an intent match.
    This strictly generalizes the [max_tools >= 1] case when exactly one tool
    matches. *)
Theorem history_doesnt_starve :
  forall catalog history intent max_tools discovered,
    In discovered catalog ->
    (* discovered tool matches intent *)
    intent = Some (def_name discovered) ->
    max_tools >= intent_match_count catalog intent ->
    In discovered (progressive_select catalog history intent max_tools).
Proof.
  intros catalog history intent max_tools discovered Hin Hmatch Hbudget.
  unfold progressive_select, intent_match_count in *.
  assert (Hm : matches_intent intent discovered = true).
  { rewrite Hmatch. cbn. apply Nat.eqb_refl. }
  apply matched_in_firstn; assumption.
Qed.

(** ** Batch Metrics *)

Record BatchMetrics : Type := mkMetrics {
  metrics_total_tools : nat;
  metrics_approved : nat;
  metrics_rejected : nat;
  metrics_completed : nat;
  metrics_failed : nat;
}.

Definition compute_metrics (batch : ToolBatch) : BatchMetrics :=
  let approved := length (filter (fun t =>
    match tool_approved t with Some true => true | _ => false end) batch) in
  let rejected := length (filter (fun t =>
    match tool_approved t with Some false => true | _ => false end) batch) in
  let completed := length (filter (fun t =>
    match tool_status t with ToolCompleted => true | _ => false end) batch) in
  let failed := length (filter (fun t =>
    match tool_status t with ToolFailed => true | _ => false end) batch) in
  mkMetrics (length batch) approved rejected completed failed.

(** Theorem: Metrics sum correctly.

    For a complete batch (every tool terminal), the three terminal-status
    buckets -- completed, failed, cancelled -- form an exact partition of the
    batch: their sizes add up to the total tool count.  No tool is double
    counted and none is left uncounted. *)
Theorem metrics_consistent :
  forall batch,
    batch_complete batch = true ->
    let m := compute_metrics batch in
    metrics_completed m + metrics_failed m +
    length (filter (fun t => match tool_status t with ToolCancelled => true | _ => false end) batch)
    = metrics_total_tools m.
Proof.
  intros batch Hcomplete.
  unfold compute_metrics. cbn [metrics_completed metrics_failed metrics_total_tools].
  unfold batch_complete in Hcomplete.
  (* When the batch is complete, each tool is terminal, so it lands in exactly
     one of the completed/failed/cancelled filters. *)
  induction batch as [|t rest IH]; cbn [filter length forallb] in *.
  - reflexivity.
  - apply andb_true_iff in Hcomplete as [Hter Hrest].
    specialize (IH Hrest).
    (* [t] is terminal: it is Completed, Failed, or Cancelled. *)
    unfold is_terminal_tool in Hter.
    destruct (tool_status t) eqn:Hst; cbn [length] in *; try discriminate; lia.
Qed.

(** ** Tool Dispatch Routing

    The real dispatcher (crates/jfc-engine/src/tools/dispatch.rs) routes on a
    [match (ToolKind, ToolInput)] from a fixed registry of tool kinds to a
    handler.  We model the routing layer directly to state and prove its
    core correctness invariants:

      - dispatch is DETERMINISTIC / FUNCTIONAL (same kind => same handler),
      - only REGISTERED kinds dispatch (unknown kinds get no handler),
      - lookup-AFTER-register returns the registered handler,
      - dispatch is TOTAL over the registered domain,
      - no tool is dispatched from a TERMINAL state. *)

(** A small representative set of tool kinds.  Decidable equality (below)
    stands in for the derived [PartialEq]/[Eq] on the real [ToolKind] enum. *)
Inductive ToolKind : Type :=
  | KindRead
  | KindWrite
  | KindBash
  | KindGlob
  | KindTask.

(** Decidable equality on [ToolKind] -- the enum/decidable-equality pattern. *)
Definition tool_kind_eq_dec (a b : ToolKind) : {a = b} + {a <> b}.
Proof. decide equality. Defined.

Definition tool_kind_eqb (a b : ToolKind) : bool :=
  if tool_kind_eq_dec a b then true else false.

Lemma tool_kind_eqb_refl : forall k, tool_kind_eqb k k = true.
Proof. intros k. unfold tool_kind_eqb. destruct (tool_kind_eq_dec k k); congruence. Qed.

Lemma tool_kind_eqb_true_iff :
  forall a b, tool_kind_eqb a b = true <-> a = b.
Proof.
  intros a b. unfold tool_kind_eqb.
  destruct (tool_kind_eq_dec a b) as [Heq|Hne].
  - split; intro H; [exact Heq | reflexivity].
  - split; intro H.
    + discriminate.
    + exfalso. apply Hne. exact H.
Qed.

(** A handler is identified by a number (its routing target). *)
Definition Handler := nat.

(** A registry is an association list from kind to handler.  The real registry
    is the fixed [match] arm set; an assoc-list is the faithful abstraction of a
    deterministic lookup table. *)
Definition Registry := list (ToolKind * Handler).

(** Lookup: first matching arm wins (mirrors top-to-bottom [match] semantics). *)
Fixpoint dispatch (reg : Registry) (k : ToolKind) : option Handler :=
  match reg with
  | [] => None
  | (k', h) :: rest =>
      if tool_kind_eqb k' k then Some h else dispatch rest k
  end.

(** A kind is registered iff [dispatch] finds a handler for it. *)
Definition registered (reg : Registry) (k : ToolKind) : Prop :=
  exists h, dispatch reg k = Some h.

(** Register a kind: prepend so the newest binding shadows older ones, exactly
    like an earlier [match] arm shadowing a later one. *)
Definition register (reg : Registry) (k : ToolKind) (h : Handler) : Registry :=
  (k, h) :: reg.

(** Theorem: dispatch is deterministic / functional.

    Same registry and same kind always route to the same handler.  [dispatch]
    is a function, so this is the functionality of the dispatch relation: there
    is never an ambiguous route.  This is the core "no nondeterministic
    dispatch" guarantee. *)
Theorem dispatch_deterministic :
  forall reg k h1 h2,
    dispatch reg k = Some h1 ->
    dispatch reg k = Some h2 ->
    h1 = h2.
Proof.
  intros reg k h1 h2 H1 H2. rewrite H1 in H2. injection H2. auto.
Qed.

(** Theorem: only registered tools dispatch.

    If a kind is NOT registered, [dispatch] returns [None] -- no handler is
    ever invented for an unknown tool kind.  This is the contrapositive
    statement that dispatch is sound w.r.t. the registry. *)
Theorem only_registered_dispatch :
  forall reg k,
    ~ registered reg k ->
    dispatch reg k = None.
Proof.
  intros reg k Hnr.
  destruct (dispatch reg k) as [h|] eqn:Hd.
  - exfalso. apply Hnr. exists h. exact Hd.
  - reflexivity.
Qed.

(** Theorem: dispatching a handler implies the kind is registered.

    The converse direction: a successful route means the kind really is in the
    registry's domain.  Together with [only_registered_dispatch] this pins the
    dispatch domain exactly to the registered kinds. *)
Theorem dispatch_implies_registered :
  forall reg k h,
    dispatch reg k = Some h ->
    registered reg k.
Proof.
  intros reg k h Hd. exists h. exact Hd.
Qed.

(** Theorem: lookup after register returns the registered handler.

    Immediately after [register reg k h], dispatching [k] yields exactly [h]
    (the newest binding shadows any older one).  This is the fundamental
    register/lookup round-trip correctness. *)
Theorem lookup_after_register :
  forall reg k h,
    dispatch (register reg k h) k = Some h.
Proof.
  intros reg k h. unfold register. cbn [dispatch].
  rewrite tool_kind_eqb_refl. reflexivity.
Qed.

(** Registering a DIFFERENT kind leaves an existing route untouched (no
    cross-talk between registry entries). *)
Theorem register_preserves_other :
  forall reg k k' h h',
    k <> k' ->
    dispatch reg k = Some h ->
    dispatch (register reg k' h') k = Some h.
Proof.
  intros reg k k' h h' Hneq Hd. unfold register. cbn [dispatch].
  destruct (tool_kind_eqb k' k) eqn:Hk.
  - apply tool_kind_eqb_true_iff in Hk. congruence.
  - exact Hd.
Qed.

(** Theorem: dispatch is total over the registered domain.

    For every registered kind there exists a handler that [dispatch] returns --
    there are no "gaps" in the routing table within its declared domain.  This
    is the totality (left-totality on the registered domain) of the dispatch
    relation. *)
Theorem dispatch_total_on_registered :
  forall reg k,
    registered reg k ->
    exists h, dispatch reg k = Some h.
Proof.
  intros reg k Hr. exact Hr.
Qed.

(** ** No Dispatch From A Terminal State

    A tool that has already reached a terminal status (completed/failed/
    cancelled) must never be (re-)dispatched.  We model the guard the runtime
    applies before routing a tool: only non-terminal (pending) tools are
    dispatchable. *)

(** Dispatchability guard: a tool may be routed only when it is not terminal. *)
Definition dispatchable (t : ToolUse) : bool :=
  negb (is_terminal_tool (tool_status t)).

(** Guarded dispatch: route the tool's kind only if the tool is dispatchable. *)
Definition guarded_dispatch (reg : Registry) (k : ToolKind) (t : ToolUse)
    : option Handler :=
  if dispatchable t then dispatch reg k else None.

(** Theorem: no tool is dispatched from a terminal state.

    If a tool's status is terminal, the guard refuses to route it regardless of
    registry or kind -- a completed/failed/cancelled tool can never re-enter
    execution.  This is the key absorbing-state safety invariant for dispatch. *)
Theorem no_dispatch_in_terminal :
  forall reg k t,
    is_terminal_tool (tool_status t) = true ->
    guarded_dispatch reg k t = None.
Proof.
  intros reg k t Hterm.
  unfold guarded_dispatch, dispatchable.
  rewrite Hterm. cbn [negb]. reflexivity.
Qed.

(** Conversely, a guarded dispatch that DID route proves the tool was
    non-terminal -- the guard is the exact characterization of dispatchability,
    not merely a sufficient condition. *)
Theorem guarded_dispatch_implies_nonterminal :
  forall reg k t h,
    guarded_dispatch reg k t = Some h ->
    is_terminal_tool (tool_status t) = false.
Proof.
  intros reg k t h Hg.
  unfold guarded_dispatch, dispatchable in Hg.
  destruct (is_terminal_tool (tool_status t)) eqn:Hterm.
  - cbn [negb] in Hg. discriminate.
  - reflexivity.
Qed.

(** Theorem: a guarded dispatch agrees with the raw routing table.

    When the guard admits a tool (non-terminal), guarded dispatch routes
    exactly as the registry would -- the guard only restricts the domain, it
    never alters the chosen handler.  This ties the safety guard to dispatch
    determinism. *)
Theorem guarded_dispatch_routes_correctly :
  forall reg k t,
    is_terminal_tool (tool_status t) = false ->
    guarded_dispatch reg k t = dispatch reg k.
Proof.
  intros reg k t Hnt.
  unfold guarded_dispatch, dispatchable.
  rewrite Hnt. cbn [negb]. reflexivity.
Qed.
