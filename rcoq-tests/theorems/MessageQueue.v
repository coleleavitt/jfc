(** * MessageQueue: Formal Model of Priority-Based Message Queue
    
    This module formalizes JFC's MessageQueue (prompt_queue.rs) which
    implements a priority-based prompt queue with FIFO ordering within
    each priority level.
    
    Key properties:
    - Higher priority messages are dequeued first
    - FIFO order is preserved within each priority level
    - drain_at_least respects minimum priority threshold
    
    References:
    - crates/jfc-core/src/prompt_queue.rs
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Priority Levels (matching QueuePriority) *)

Inductive QueuePriority : Type :=
  | Later   (* Priority 0 - drain at end of turn *)
  | Next    (* Priority 1 - drain between tool batches *)
  | Now.    (* Priority 2 - immediate, jump the queue *)

(** Priority comparison *)
Definition priority_leb (p1 p2 : QueuePriority) : bool :=
  match p1, p2 with
  | Later, _ => true
  | Next, Later => false
  | Next, _ => true
  | Now, Now => true
  | Now, _ => false
  end.

Definition priority_lt (p1 p2 : QueuePriority) : bool :=
  match p1, p2 with
  | Later, Next => true
  | Later, Now => true
  | Next, Now => true
  | _, _ => false
  end.

(** ** Queued Prompt Record *)

Record QueuedPrompt : Type := mkPrompt {
  prompt_text : nat;  (* Simplified: use nat as text ID *)
  prompt_is_meta : bool;
  prompt_priority : QueuePriority;
}.

Definition MessageQueue := list QueuedPrompt.

(** ** Queue Operations *)

(** Push to back of queue *)
Definition push (queue : MessageQueue) (prompt : QueuedPrompt) : MessageQueue :=
  queue ++ [prompt].

(** Find maximum priority in queue *)
Fixpoint max_priority (queue : MessageQueue) : option QueuePriority :=
  match queue with
  | [] => None
  | p :: rest =>
      match max_priority rest with
      | None => Some (prompt_priority p)
      | Some mp =>
          if priority_leb mp (prompt_priority p) then
            Some (prompt_priority p)
          else
            Some mp
      end
  end.

(** Find first element with given priority *)
Fixpoint find_priority_idx (queue : MessageQueue) (target : QueuePriority) 
    (idx : nat) : option nat :=
  match queue with
  | [] => None
  | p :: rest =>
      match prompt_priority p with
      | pr =>
          if match pr, target with
             | Later, Later | Next, Next | Now, Now => true
             | _, _ => false
             end
          then Some idx
          else find_priority_idx rest target (S idx)
      end
  end.

(** Remove element at index *)
Fixpoint remove_at {A : Type} (l : list A) (idx : nat) : list A :=
  match l, idx with
  | [], _ => []
  | _ :: rest, O => rest
  | x :: rest, S n => x :: remove_at rest n
  end.

(** Pop max priority element (FIFO within priority) *)
Definition pop_max_priority (queue : MessageQueue) : option (QueuedPrompt * MessageQueue) :=
  match max_priority queue with
  | None => None
  | Some mp =>
      match find_priority_idx queue mp 0 with
      | None => None
      | Some idx =>
          match nth_error queue idx with
          | None => None
          | Some prompt => Some (prompt, remove_at queue idx)
          end
      end
  end.

(** Drain all elements with priority >= min_priority *)
Definition drain_at_least (queue : MessageQueue) (min_priority : QueuePriority) 
    : (list QueuedPrompt * MessageQueue) :=
  let (drained, remaining) := partition 
    (fun p => negb (priority_lt (prompt_priority p) min_priority)) 
    queue in
  (* Sort drained by priority descending - simplified: just return *)
  (drained, remaining).

(** ** Structural Helper Lemmas

    These lemmas expose the structure of [find_priority_idx], [remove_at], and
    the priority order that the queue invariants below rely on. *)

(** Boolean priority equality, matching the inline match used inside
    [find_priority_idx]. *)
Definition prio_eqb (p t : QueuePriority) : bool :=
  match p, t with
  | Later, Later | Next, Next | Now, Now => true
  | _, _ => false
  end.

Lemma prio_eqb_true : forall p t, prio_eqb p t = true -> p = t.
Proof. intros p t H. destruct p, t; simpl in H; try discriminate; reflexivity. Qed.

(** The inline match inside [find_priority_idx] is exactly [prio_eqb]. *)
Lemma inline_eq_prio_eqb :
  forall p t,
    match p, t with
    | Later, Later | Next, Next | Now, Now => true
    | _, _ => false end = prio_eqb p t.
Proof. intros p t. destruct p, t; reflexivity. Qed.

(** Removing a valid index drops the length by exactly one. *)
Lemma remove_at_length :
  forall {A} (l : list A) idx,
    idx < length l ->
    length (remove_at l idx) = length l - 1.
Proof.
  induction l as [|x rest IH]; intros idx Hlt.
  - simpl in Hlt. lia.
  - destruct idx as [|n].
    + simpl. lia.
    + simpl in Hlt. simpl.
      rewrite IH by lia. simpl. lia.
Qed.

(** Removing index [idx] keeps any element living at a different index [j]. *)
Lemma remove_at_preserves :
  forall {A} (l : list A) idx j x,
    j <> idx ->
    nth_error l j = Some x ->
    In x (remove_at l idx).
Proof.
  induction l as [|a rest IH]; intros idx j x Hne Hnth.
  - destruct j; simpl in Hnth; discriminate.
  - destruct idx as [|i'].
    + destruct j as [|j']; [lia|].
      simpl. simpl in Hnth.
      apply nth_error_In in Hnth. exact Hnth.
    + simpl. destruct j as [|j'].
      * simpl in Hnth. injection Hnth as Ha. subst a. left. reflexivity.
      * simpl in Hnth. right. apply (IH i' j' x); [lia|exact Hnth].
Qed.

(** [find_priority_idx] returns an index within the queue, offset by [start]. *)
Lemma find_priority_idx_bound :
  forall queue target start i,
    find_priority_idx queue target start = Some i ->
    start <= i < start + length queue.
Proof.
  induction queue as [|p rest IH]; intros target start i H.
  - simpl in H. discriminate.
  - simpl in H. rewrite inline_eq_prio_eqb in H.
    destruct (prio_eqb (prompt_priority p) target) eqn:Heq.
    + injection H as Hi. subst i. simpl. lia.
    + apply IH in H. simpl. lia.
Qed.

(** The element [find_priority_idx] points at really has the target priority. *)
Lemma find_priority_idx_priority :
  forall queue target start i,
    find_priority_idx queue target start = Some i ->
    exists p, nth_error queue (i - start) = Some p /\ prompt_priority p = target.
Proof.
  induction queue as [|p rest IH]; intros target start i H.
  - simpl in H. discriminate.
  - simpl in H. rewrite inline_eq_prio_eqb in H.
    destruct (prio_eqb (prompt_priority p) target) eqn:Heq.
    + inversion H; subst. exists p.
      replace (i - i) with 0 by lia. simpl.
      split; [reflexivity| apply prio_eqb_true; exact Heq].
    + apply find_priority_idx_bound in H as Hb.
      apply IH in H. destruct H as [p0 [Hnth Hpr]].
      exists p0. split; [|exact Hpr].
      assert (i >= S start) by lia.
      replace (i - start) with (S (i - S start)) by lia.
      simpl. exact Hnth.
Qed.

(** [find_priority_idx] returns the FIRST matching index: every earlier index
    holds an element whose priority differs from the target. *)
Lemma find_priority_idx_first :
  forall queue target start i j p,
    find_priority_idx queue target start = Some i ->
    start <= j -> j < i ->
    nth_error queue (j - start) = Some p ->
    prompt_priority p <> target.
Proof.
  induction queue as [|x rest IH]; intros target start i j p H Hsj Hji Hnth.
  - simpl in H. discriminate.
  - simpl in H. rewrite inline_eq_prio_eqb in H.
    destruct (prio_eqb (prompt_priority x) target) eqn:Heq.
    + injection H as Hi. subst i. lia.
    + destruct (Nat.eq_dec j start) as [Hjs|Hjs].
      * subst j. replace (start - start) with 0 in Hnth by lia.
        simpl in Hnth. inversion Hnth; subst.
        intro Hcontra. subst target.
        unfold prio_eqb in Heq. destruct (prompt_priority p); discriminate.
      * assert (Hjss: j - start = S (j - S start)) by lia.
        rewrite Hjss in Hnth. simpl in Hnth.
        eapply (IH target (S start) i j p H); [lia | lia | exact Hnth].
Qed.

(** Priority order totality: if [a] is not strictly below [b] then [b <= a]. *)
Lemma priority_lt_false_leb :
  forall a b, priority_lt a b = false -> priority_leb b a = true.
Proof. intros a b H. destruct a, b; simpl in *; try discriminate; reflexivity. Qed.

(** ** Queue Invariant Theorems *)

(** Theorem: Push increases length by 1 *)
Theorem push_increases_length :
  forall queue prompt,
    length (push queue prompt) = S (length queue).
Proof.
  intros queue prompt.
  unfold push.
  rewrite app_length.
  simpl. lia.
Qed.

(** Theorem: Pop decreases length by 1 when queue is non-empty *)
Theorem pop_decreases_length :
  forall queue prompt queue',
    pop_max_priority queue = Some (prompt, queue') ->
    length queue' = length queue - 1.
Proof.
  intros queue prompt queue' Hpop.
  unfold pop_max_priority in Hpop.
  destruct (max_priority queue) as [mp|] eqn:Hmax; [|discriminate].
  destruct (find_priority_idx queue mp 0) as [idx|] eqn:Hfind; [|discriminate].
  destruct (nth_error queue idx) as [p|] eqn:Hnth; [|discriminate].
  injection Hpop as Hp Hq. subst.
  apply find_priority_idx_bound in Hfind.
  apply remove_at_length. lia.
Qed.

(** Theorem: Pop returns element with max priority *)
Theorem pop_returns_max :
  forall queue prompt queue',
    pop_max_priority queue = Some (prompt, queue') ->
    max_priority queue = Some (prompt_priority prompt).
Proof.
  intros queue prompt queue' Hpop.
  unfold pop_max_priority in Hpop.
  destruct (max_priority queue) as [mp|] eqn:Hmax; [|discriminate].
  destruct (find_priority_idx queue mp 0) as [idx|] eqn:Hfind; [|discriminate].
  destruct (nth_error queue idx) as [p|] eqn:Hnth; [|discriminate].
  injection Hpop as Hp Hq. subst prompt. subst queue'.
  apply find_priority_idx_priority in Hfind as [p0 [Hnth0 Hpr0]].
  replace (idx - 0) with idx in Hnth0 by lia.
  rewrite Hnth in Hnth0. injection Hnth0 as He. subst p0.
  rewrite Hpr0. reflexivity.
Qed.

(** Theorem: FIFO order preserved within priority *)
Theorem fifo_within_priority :
  forall queue p1 p2 idx1 idx2,
    prompt_priority p1 = prompt_priority p2 ->
    nth_error queue idx1 = Some p1 ->
    nth_error queue idx2 = Some p2 ->
    idx1 < idx2 ->
    (* Then p1 will be dequeued before p2 *)
    forall queue' prompt,
      pop_max_priority queue = Some (prompt, queue') ->
      prompt_priority prompt = prompt_priority p1 ->
      prompt = p1 \/ In p2 queue'.
Proof.
  intros queue p1 p2 idx1 idx2 Hsame H1 H2 Hlt queue' prompt Hpop Hprio.
  (* The stated disjunction is provable, and in fact the SECOND disjunct always
     holds: the popped element sits at [find_priority_idx queue mp 0], the FIRST
     index with the max priority [mp].  Since [idx1] also has priority [mp] and
     [idx1 < idx2], the removed index is <= idx1 < idx2, so p2 (at idx2) is never
     the removed element and therefore survives in [queue']. *)
  right.
  unfold pop_max_priority in Hpop.
  destruct (max_priority queue) as [mp|] eqn:Hmax; [|discriminate].
  destruct (find_priority_idx queue mp 0) as [idx|] eqn:Hfind; [|discriminate].
  destruct (nth_error queue idx) as [pp|] eqn:Hnth; [|discriminate].
  injection Hpop as Hpp Hq. subst pp. subst queue'.
  apply find_priority_idx_priority in Hfind as Hf.
  destruct Hf as [pf [Hnthf Hprf]].
  replace (idx - 0) with idx in Hnthf by lia.
  rewrite Hnth in Hnthf. injection Hnthf as Hpe. subst pf.
  assert (Hmp1 : mp = prompt_priority p1) by (rewrite <- Hprf; exact Hprio).
  assert (Hidx_le : idx <= idx1).
  { destruct (le_lt_dec idx idx1) as [Hle|Hgt]; [exact Hle|exfalso].
    assert (Hne : prompt_priority p1 <> mp).
    { apply (find_priority_idx_first queue mp 0 idx idx1 p1 Hfind);
        [ lia | exact Hgt | replace (idx1 - 0) with idx1 by lia; exact H1 ]. }
    apply Hne. symmetry. exact Hmp1. }
  assert (Hne2 : idx2 <> idx) by lia.
  eapply remove_at_preserves; [exact Hne2 | exact H2].
Qed.

(** ** Drain Theorems *)

(** Theorem: Drain partitions correctly *)
Theorem drain_partition :
  forall queue min_priority drained remaining,
    drain_at_least queue min_priority = (drained, remaining) ->
    forall p,
      (In p drained <-> In p queue /\ negb (priority_lt (prompt_priority p) min_priority) = true) /\
      (In p remaining <-> In p queue /\ negb (priority_lt (prompt_priority p) min_priority) = false).
Proof.
  intros queue min_priority drained remaining Hdrain p.
  unfold drain_at_least in Hdrain.
  rewrite partition_as_filter in Hdrain.
  injection Hdrain as Hd Hr. subst drained remaining.
  split.
  - rewrite filter_In. reflexivity.
  - rewrite filter_In.
    split.
    + intros [Hin Hng]. split; [exact Hin|].
      apply negb_true_iff in Hng. exact Hng.
    + intros [Hin Hng]. split; [exact Hin|].
      apply negb_true_iff. exact Hng.
Qed.

(** Theorem: Drain preserves total elements *)
Theorem drain_preserves_elements :
  forall queue min_priority drained remaining,
    drain_at_least queue min_priority = (drained, remaining) ->
    length drained + length remaining = length queue.
Proof.
  intros queue min_priority drained remaining Hdrain.
  unfold drain_at_least in Hdrain.
  destruct (partition
    (fun p => negb (priority_lt (prompt_priority p) min_priority)) queue)
    as [d r] eqn:Hp.
  injection Hdrain as Hd Hr. subst d r.
  apply partition_length in Hp. lia.
Qed.

(** Theorem: Drained elements have sufficient priority *)
Theorem drained_have_priority :
  forall queue min_priority drained remaining p,
    drain_at_least queue min_priority = (drained, remaining) ->
    In p drained ->
    priority_leb min_priority (prompt_priority p) = true.
Proof.
  intros queue min_priority drained remaining p Hdrain Hin.
  unfold drain_at_least in Hdrain.
  rewrite partition_as_filter in Hdrain.
  injection Hdrain as Hd Hr. subst drained remaining.
  apply filter_In in Hin. destruct Hin as [_ Hng].
  apply negb_true_iff in Hng.
  apply priority_lt_false_leb. exact Hng.
Qed.

(** ** Deferred Tool Use Queue *)

Record DeferredToolUse : Type := mkDeferred {
  deferred_id : nat;
  deferred_name : nat;
  deferred_reason : nat;
  deferred_queued_at : nat;  (* timestamp *)
}.

Definition DeferredQueue := list DeferredToolUse.

(** Cap on deferred tool uses (DEFERRED_TOOL_USES_CAP = 64) *)
Definition DEFERRED_CAP : nat := 64.

(** Push with cap - evict oldest if over capacity *)
Definition push_deferred (queue : DeferredQueue) (d : DeferredToolUse) : DeferredQueue :=
  let new_queue := queue ++ [d] in
  if Nat.leb (length new_queue) DEFERRED_CAP then
    new_queue
  else
    skipn 1 new_queue.  (* Drop oldest *)

(** Theorem: Deferred queue never exceeds cap *)
Theorem deferred_respects_cap :
  forall queue d,
    length queue <= DEFERRED_CAP ->
    length (push_deferred queue d) <= DEFERRED_CAP.
Proof.
  intros queue d Hle.
  unfold push_deferred.
  destruct (Nat.leb (length (queue ++ [d])) DEFERRED_CAP) eqn:Hcmp.
  - apply Nat.leb_le in Hcmp. exact Hcmp.
  - rewrite skipn_length.
    rewrite app_length. simpl.
    lia.
Qed.

(** ** Tool Use Summary Queue *)

Record ToolUseSummary : Type := mkSummary {
  summary_text : nat;
  summary_preceding_ids : list nat;
  summary_created_at : nat;
}.

Definition SummaryQueue := list ToolUseSummary.

Definition SUMMARY_CAP : nat := 32.

(** Theorem: Summary queue bounded *)
Theorem summary_bounded :
  forall (queue : SummaryQueue) (s : ToolUseSummary),
    length queue < SUMMARY_CAP ->
    length (queue ++ [s]) <= SUMMARY_CAP.
Proof.
  intros queue s Hlt.
  rewrite length_app. simpl. lia.
Qed.

(** ** Integration: Priority affects compaction *)

(** Higher priority messages should be preserved during compaction *)
Definition should_preserve (p : QueuedPrompt) (compaction_active : bool) : bool :=
  match compaction_active, prompt_priority p with
  | true, Now => true   (* Always preserve immediate priority *)
  | true, Next => true  (* Preserve next-batch priority *)
  | true, Later => false (* Can defer these *)
  | false, _ => true    (* No compaction, preserve all *)
  end.

(** Theorem: High priority always preserved during compaction *)
Theorem high_priority_preserved :
  forall p,
    prompt_priority p = Now ->
    should_preserve p true = true.
Proof.
  intros p Hprio.
  unfold should_preserve.
  rewrite Hprio. reflexivity.
Qed.
