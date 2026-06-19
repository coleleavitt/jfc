(** * ToolCallFlow: Formal Model of Tool Dispatch and Execution
    
    This module formalizes the tool call dispatch flow from JFC.
    We model:
    
    1. Tool batch dispatch ordering guarantees
    2. Tool status transitions during execution
    3. Approval queue semantics
    
    References:
    - crates/jfc-engine/src/stream/tool_dispatch.rs
    - crates/jfc-engine/src/runtime/approvals.rs
    - crates/jfc-core/src/tool_call.rs
*)

Require Import Coq.Lists.List.
Require Import Coq.Arith.Arith.
Require Import Coq.Bool.Bool.
Import ListNotations.

Require Import JFC.ExecutionStatus.

(** ** Tool Call Model *)

(** A simplified ToolCall record *)
Record ToolCall : Type := mkToolCall {
  tool_id : nat;
  tool_kind : nat;  (* ToolKind enum as nat for simplicity *)
  status : ExecutionStatus;
}.

(** ** Tool Batch *)

Definition ToolBatch := list ToolCall.

(** All tools in a batch start Pending *)
Definition batch_all_pending (batch : ToolBatch) : Prop :=
  forall tc, In tc batch -> status tc = Pending.

(** ** Dispatch Ordering
    
    Tools are dispatched in order: if tool i starts running,
    all tools j < i must have already started (Running or terminal).
*)
Definition dispatch_ordered (batch : ToolBatch) : Prop :=
  forall i j : nat,
    i < j ->
    j < length batch ->
    forall tc_j, nth_error batch j = Some tc_j ->
    status tc_j = Running ->
    exists tc_i, nth_error batch i = Some tc_i /\
                 (status tc_i = Running \/ is_terminal (status tc_i) = true).

(** ** Approval Queue Model
    
    Tools requiring approval enter a queue.
    Only the head of the queue can be approved/denied.
*)

Inductive ApprovalDecision : Type :=
  | Approved
  | Denied
  | PendingDecision.

Record QueuedTool : Type := mkQueuedTool {
  queued_tool : ToolCall;
  decision : ApprovalDecision;
}.

Definition ApprovalQueue := list QueuedTool.

(** Head of queue can be resolved *)
Definition can_resolve_head (q : ApprovalQueue) : bool :=
  match q with
  | [] => false
  | qt :: _ => match decision qt with
               | PendingDecision => true
               | _ => false
               end
  end.

(** Resolve head of queue *)
Definition resolve_head (q : ApprovalQueue) (d : ApprovalDecision) : ApprovalQueue :=
  match q with
  | [] => []
  | qt :: rest => mkQueuedTool (queued_tool qt) d :: rest
  end.

(** Advance queue: remove resolved head *)
Definition advance_queue (q : ApprovalQueue) : ApprovalQueue :=
  match q with
  | [] => []
  | qt :: rest => match decision qt with
                  | PendingDecision => q  (* Cannot advance unresolved *)
                  | _ => rest
                  end
  end.

(** ** Queue Theorems *)

(** Theorem: Resolving head changes the decision *)
Theorem resolve_head_sets_decision :
  forall q d qt rest,
    q = qt :: rest ->
    decision qt = PendingDecision ->
    d <> PendingDecision ->
    exists qt', hd_error (resolve_head q d) = Some qt' /\
                decision qt' = d.
Proof.
  intros q d qt rest Hq Hpend Hnotpend.
  subst q.
  simpl.
  exists (mkQueuedTool (queued_tool qt) d).
  split; reflexivity.
Qed.

(** Theorem: Queue advances only after resolution *)
Theorem advance_requires_resolution :
  forall q qt rest,
    q = qt :: rest ->
    decision qt = PendingDecision ->
    advance_queue q = q.
Proof.
  intros q qt rest Hq Hpend.
  subst q. simpl.
  rewrite Hpend. reflexivity.
Qed.

(** ** Tool Status Flow Invariants *)

(** A tool batch is well-formed if all tools start pending *)
Definition well_formed_batch (batch : ToolBatch) : Prop :=
  batch_all_pending batch.

(** After dispatch completes, all tools are terminal *)
Definition dispatch_complete (batch : ToolBatch) : Prop :=
  forall tc, In tc batch -> is_terminal (status tc) = true.

(** Partial progress: some tools may still be running *)
Definition dispatch_in_progress (batch : ToolBatch) : Prop :=
  exists tc, In tc batch /\ is_alive (status tc) = true.

(** Helper: a batch dichotomy by list induction.

    For ANY tool batch, either some tool is alive, or every tool is
    terminal.  This is the structural fact the progress/complete theorem
    needs; the [terminal_alive_partition] lemma from ExecutionStatus gives
    the per-element classification, and induction lifts it to the list. *)
Lemma batch_alive_or_all_terminal :
  forall batch : ToolBatch,
    (exists tc, In tc batch /\ is_alive (status tc) = true) \/
    (forall tc, In tc batch -> is_terminal (status tc) = true).
Proof.
  induction batch as [|tc rest IH].
  - (* empty batch: vacuously all-terminal *)
    right. intros tc Hin. inversion Hin.
  - destruct (is_alive (status tc)) eqn:Halive.
    + (* head is alive *)
      left. exists tc. split; [left; reflexivity | exact Halive].
    + (* head not alive => head is terminal (by partition) *)
      assert (is_terminal (status tc) = true) as Hterm.
      { pose proof (terminal_alive_partition (status tc)) as Hpart.
        rewrite Halive in Hpart.
        destruct (is_terminal (status tc)); [reflexivity | discriminate Hpart]. }
      destruct IH as [Halive_rest | Hterm_rest].
      * (* some tool in the tail is alive *)
        left. destruct Halive_rest as [tc' [Hin' Halive']].
        exists tc'. split; [right; exact Hin' | exact Halive'].
      * (* every tool in the tail is terminal => whole list terminal *)
        right. intros tc' Hin'.
        destruct Hin' as [Heq | Hin_rest].
        -- subst tc'. exact Hterm.
        -- apply Hterm_rest. exact Hin_rest.
Qed.

(** Theorem: A batch is either in-progress or complete

    Note: the third disjunct (some tool is [Pending]) is implied by the
    first, since [Pending] is alive ([is_alive Pending = true]); it is kept
    in the statement as given.  The proof never needs to weaken it. *)
Theorem batch_progress_or_complete :
  forall batch : ToolBatch,
    batch <> [] ->
    (dispatch_in_progress batch \/ dispatch_complete batch) \/
    exists tc, In tc batch /\ status tc = Pending.
Proof.
  intros batch _.
  destruct (batch_alive_or_all_terminal batch) as [Halive | Hterm].
  - (* some tool is alive: dispatch is in progress *)
    left. left. exact Halive.
  - (* every tool is terminal: dispatch is complete *)
    left. right. exact Hterm.
Qed.

(** ** Batched Dispatch Monotonicity
    
    Once a tool transitions to a terminal state, it stays there.
*)
Definition monotonic_progress (before after : ToolBatch) : Prop :=
  length before = length after /\
  forall i tc_b tc_a,
    nth_error before i = Some tc_b ->
    nth_error after i = Some tc_a ->
    tool_id tc_b = tool_id tc_a /\
    (is_terminal (status tc_b) = true -> status tc_a = status tc_b).

(** Theorem: Terminal states are preserved *)
Theorem terminal_preserved :
  forall before after i tc_b tc_a,
    monotonic_progress before after ->
    nth_error before i = Some tc_b ->
    nth_error after i = Some tc_a ->
    is_terminal (status tc_b) = true ->
    status tc_a = status tc_b.
Proof.
  intros before after i tc_b tc_a [Hlen Hmono] Hbefore Hafter Hterm.
  destruct (Hmono i tc_b tc_a Hbefore Hafter) as [Hid Hpreserve].
  apply Hpreserve. exact Hterm.
Qed.
