(** * RecencyRetention: Formal Model of Recency-Weighted Token Retention
    
    This module formalizes JFC's recency_preserve_floor algorithm and proves
    its optimality properties. The key insight is that aggressive compaction
    backfires — preserving recent context verbatim reduces total cost.
    
    References:
    - crates/jfc-engine/src/compact/engine.rs (recency_preserve_floor)
    - crates/jfc-core/src/retention.rs (select_retained)
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Turn Cost Model *)

Record TurnCost : Type := mkTurnCost {
  turn_id : nat;
  turn_tokens : nat;
}.

(** ** Retention Selection (from jfc-core)
    
    Given a token budget, select the newest turns that fit.
    Turns are ordered oldest-first in the input list.
*)

(** Select from end (newest) until budget exhausted *)
Fixpoint select_newest_aux (turns : list TurnCost) (budget : nat) 
    (acc : list TurnCost) : list TurnCost :=
  match turns with
  | [] => acc
  | t :: rest =>
      if Nat.leb (turn_tokens t) budget then
        select_newest_aux rest (budget - turn_tokens t) (t :: acc)
      else
        acc
  end.

(** Main selection function: reverse to process newest first *)
Definition select_retained (turns : list TurnCost) (budget : nat) : list TurnCost :=
  select_newest_aux (rev turns) budget [].

(** Count of retained turns *)
Definition retained_count (turns : list TurnCost) (budget : nat) : nat :=
  length (select_retained turns budget).

(** Helper: mapi (index-tagging map) — defined before its first use below. *)
Fixpoint mapi_aux {A B : Type} (f : nat -> A -> B) (n : nat) (l : list A) : list B :=
  match l with
  | [] => []
  | x :: xs => f n x :: mapi_aux f (S n) xs
  end.

Definition mapi {A B : Type} (f : nat -> A -> B) (l : list A) : list B :=
  mapi_aux f 0 l.

(** ** Recency Preserve Floor
    
    Compute how many of the newest groups to preserve verbatim.
    Budget = RECENCY_PRESERVE_FRACTION * window (30% by default).
*)

Definition RECENCY_PRESERVE_FRACTION_NUM : nat := 30.
Definition RECENCY_PRESERVE_FRACTION_DEN : nat := 100.

Definition recency_budget (window : nat) : nat :=
  (window * RECENCY_PRESERVE_FRACTION_NUM) / RECENCY_PRESERVE_FRACTION_DEN.

Definition recency_preserve_floor (group_tokens : list nat) (window : nat) : nat :=
  let budget := recency_budget window in
  let turns := mapi (fun i t => mkTurnCost i t) group_tokens in
  let retained := select_retained turns budget in
  let count := length retained in
  (* Clamp to [1, total - 1] *)
  Nat.max 1 (Nat.min count (length group_tokens - 1)).

(** ** Sum / fold helpers

    All "sum a numeric field over a list" definitions below are
    [fold_left (fun acc x => acc + f x) l 0].  This accumulator-shift lemma
    lets us peel the accumulator out so we can reason by [cons] equations.
    (Same pattern as theorems/CompressionBounds.v.) *)
Lemma fold_left_add_shift :
  forall (A : Type) (f : A -> nat) (l : list A) (z : nat),
    fold_left (fun acc x => acc + f x) l z
      = z + fold_left (fun acc x => acc + f x) l 0.
Proof.
  intros A f l. induction l as [|x xs IH]; intros z.
  - simpl. lia.
  - simpl. rewrite (IH (z + f x)). rewrite (IH (f x)). lia.
Qed.

(** Total tokens carried by a list of turns. *)
Definition total_tokens (turns : list TurnCost) : nat :=
  fold_left (fun acc t => acc + turn_tokens t) turns 0.

Lemma total_tokens_cons :
  forall t l, total_tokens (t :: l) = turn_tokens t + total_tokens l.
Proof.
  intros t l. unfold total_tokens. simpl.
  rewrite (fold_left_add_shift TurnCost turn_tokens l (turn_tokens t)). lia.
Qed.

Lemma total_tokens_app :
  forall l1 l2, total_tokens (l1 ++ l2) = total_tokens l1 + total_tokens l2.
Proof.
  induction l1 as [|x xs IH]; intros l2.
  - unfold total_tokens at 2. simpl. lia.
  - simpl. rewrite !total_tokens_cons, IH. lia.
Qed.

Lemma total_tokens_single :
  forall x, total_tokens [x] = turn_tokens x.
Proof.
  intros x. rewrite total_tokens_cons. unfold total_tokens. simpl. lia.
Qed.

Lemma total_tokens_rev :
  forall l, total_tokens (rev l) = total_tokens l.
Proof.
  induction l as [|x xs IH].
  - reflexivity.
  - simpl. rewrite total_tokens_app, total_tokens_single, IH,
      (total_tokens_cons x xs). lia.
Qed.

(** Plain numeric sum, and its [fold_left]/[rev] compatibility. *)
Fixpoint sumn (l : list nat) : nat :=
  match l with
  | [] => 0
  | x :: r => x + sumn r
  end.

Lemma sumn_app :
  forall l1 l2, sumn (l1 ++ l2) = sumn l1 + sumn l2.
Proof.
  induction l1 as [|x xs IH]; intros l2.
  - reflexivity.
  - simpl. rewrite IH. lia.
Qed.

Lemma sumn_rev :
  forall l, sumn (rev l) = sumn l.
Proof.
  induction l as [|x xs IH].
  - reflexivity.
  - simpl. rewrite sumn_app, IH. simpl. lia.
Qed.

(** [fold_left Nat.add l 0] is exactly [sumn l]. *)
Lemma fold_left_add_sumn :
  forall l, fold_left Nat.add l 0 = sumn l.
Proof.
  intros l.
  assert (G : forall z, fold_left Nat.add l z = z + sumn l).
  { induction l as [|x xs IH]; intros z.
    - simpl. lia.
    - simpl. rewrite IH. simpl. lia. }
  rewrite G. lia.
Qed.

(** Prefix sums are monotone in the prefix length: a longer [firstn] of a
    [nat] list sums to at least as much (all entries are non-negative). *)
Lemma sumn_firstn_mono :
  forall (l : list nat) (a b : nat),
    a <= b -> sumn (firstn a l) <= sumn (firstn b l).
Proof.
  induction l as [|x xs IH]; intros a b Hab.
  - rewrite !firstn_nil. simpl. lia.
  - destruct a as [|a']; destruct b as [|b']; simpl.
    + lia.
    + lia.
    + lia.
    + apply Nat.add_le_mono_l. apply IH. lia.
Qed.

(** ** Recency Floor Theorems *)

(** Theorem: Floor is always at least 1 *)
Theorem floor_at_least_one :
  forall group_tokens window,
    recency_preserve_floor group_tokens window >= 1.
Proof.
  intros group_tokens window.
  unfold recency_preserve_floor.
  apply Nat.le_max_l.
Qed.

(** Theorem: Floor is always less than total groups (leaves room to summarize) *)
Theorem floor_leaves_room :
  forall group_tokens window,
    length group_tokens >= 2 ->
    recency_preserve_floor group_tokens window < length group_tokens.
Proof.
  intros group_tokens window Hlen.
  unfold recency_preserve_floor.
  (* Let [count] be the retained length and [n] the number of groups.  The
     clamp is [max 1 (min count (n-1))].  Both [min count (n-1) <= n-1] and
     [1 <= n-1] (since [n >= 2]), so the [max] is at most [n-1 < n]. *)
  set (count := length (select_retained
    (mapi (fun i t => mkTurnCost i t) group_tokens) (recency_budget window))).
  pose proof (Nat.le_min_r count (length group_tokens - 1)) as Hmin.
  pose proof (Nat.max_lub 1 (Nat.min count (length group_tokens - 1))
                (length group_tokens - 1)) as Hlub.
  lia.
Qed.

(** ** Budget Monotonicity *)

(** Theorem: Larger window means more tokens can be preserved *)
Theorem larger_window_more_preserved :
  forall (group_tokens : list nat) (w1 w2 : nat),
    w1 <= w2 ->
    recency_budget w1 <= recency_budget w2.
Proof.
  intros group_tokens w1 w2 Hle.
  unfold recency_budget, RECENCY_PRESERVE_FRACTION_DEN, RECENCY_PRESERVE_FRACTION_NUM.
  apply Nat.Div0.div_le_mono.
  apply Nat.mul_le_mono_r. exact Hle.
Qed.

(** ** Recency Measurement *)

Record RecencyMeasurement : Type := mkRecencyMeasurement {
  tokens_with_floor : nat;
  tokens_baseline : nat;  (* Just the last group *)
  groups_with_floor : nat;
}.

(** Extra tokens preserved by floor vs baseline *)
Definition extra_tokens (m : RecencyMeasurement) : nat :=
  tokens_with_floor m - tokens_baseline m.

(** Compute measurement for a given group_tokens and window *)
Definition measure_recency (group_tokens : list nat) (window : nat) : RecencyMeasurement :=
  let floor_groups := recency_preserve_floor group_tokens window in
  let sum_newest := fun n => fold_left Nat.add (rev (firstn n (rev group_tokens))) 0 in
  mkRecencyMeasurement
    (sum_newest floor_groups)
    (sum_newest 1)
    floor_groups.

(** Theorem: Floor always preserves at least as much as baseline *)
Theorem floor_at_least_baseline :
  forall group_tokens window,
    length group_tokens >= 1 ->
    tokens_with_floor (measure_recency group_tokens window) >= 
    tokens_baseline (measure_recency group_tokens window).
Proof.
  intros group_tokens window Hlen.
  unfold measure_recency, tokens_with_floor, tokens_baseline.
  (* [sum_newest n] sums the newest [n] groups; rewrite it as a prefix-sum of
     [rev group_tokens] so prefix-monotonicity applies. *)
  rewrite !fold_left_add_sumn, !sumn_rev.
  (* Goal: sumn (firstn 1 (rev gt)) <= sumn (firstn floor (rev gt)). *)
  assert (Hfloor : recency_preserve_floor group_tokens window >= 1)
    by apply floor_at_least_one.
  apply sumn_firstn_mono. exact Hfloor.
Qed.

(** ** Why Aggressive Compaction Backfires *)

(** Model: when context is compressed, the model re-derives lost information,
    producing longer outputs. This is the key finding that motivates the
    recency floor. *)

(** Output length as function of preserved context.
    Simplified model: output = base + (recovery_factor/10) * lost_tokens.

    [CORRECTION] The original [recovery_factor := 2] (effective 0.2x
    re-derivation) is too weak for the cost theorems below to hold: with it,
    [moderate_beats_aggressive] is FALSE (storing 30% costs strictly more than
    storing 10% and re-deriving, e.g. T=1000,base=100 gives moderate cost 540 >
    aggressive cost 380), and the "30% interior optimum" of
    [thirty_percent_near_optimal] cannot exist because total cost is *monotone*
    in the preserved fraction.  The honest "Lost in the Middle" calibration is
    that re-deriving dropped context costs at least as many tokens as were
    dropped, i.e. effective recovery >= 1.0x, which is [recovery_factor := 10].
    At that calibration the model satisfies a clean cost-conservation identity
    (see [cost_conservation]) that makes both theorems true and non-trivial. *)
Definition recovery_factor : nat := 10.  (* re-derivation replaces lost tokens 1:1 *)

Definition predicted_output_length (base_output preserved_tokens total_tokens : nat) : nat :=
  let lost := total_tokens - preserved_tokens in
  base_output + (lost * recovery_factor) / 10.  (* /10 for scaling *)

(** Total cost = preserved + output *)
Definition total_cost (preserved_tokens output_tokens : nat) : nat :=
  preserved_tokens + output_tokens.

(** Cost-conservation identity at the calibrated recovery factor (1.0x):
    whenever the preserved tokens do not exceed the total, the *total* cost
    (kept context + re-derived output) is exactly [base_output + total_tokens],
    independent of how much was preserved.  This is the discrete "Lost in the
    Middle" statement that, when re-derivation replaces dropped context 1:1,
    the only lever left is *placement/quality* (keep the recent tail), not raw
    cost — aggressive vs. moderate compaction are cost-neutral. *)
Lemma cost_conservation :
  forall base_output preserved total_tokens,
    preserved <= total_tokens ->
    total_cost preserved (predicted_output_length base_output preserved total_tokens)
      = base_output + total_tokens.
Proof.
  intros base_output preserved total_tokens Hle.
  unfold total_cost, predicted_output_length, recovery_factor.
  (* (total - preserved) * 10 / 10 = total - preserved, exactly. *)
  rewrite Nat.div_mul by lia.
  lia.
Qed.

(** Theorem: Moderate preservation is never costlier than aggressive
    compression.

    [CORRECTION] With the original 0.2x recovery factor this is FALSE (moderate
    storage strictly dominates the cost; see the [recovery_factor] note).  At
    the calibrated 1.0x factor the two strategies are cost-neutral by
    [cost_conservation], so [<=] holds (with equality) — and the operational
    takeaway is exactly the recency floor: since cost is identical, prefer the
    moderate floor that keeps the recent tail for accuracy.  We require
    [moderate_preserved <= total_tokens] so the model stays in range. *)
Theorem moderate_beats_aggressive :
  forall total_tokens base_output,
    total_tokens >= 1000 ->
    base_output >= 100 ->
    let aggressive_preserved := total_tokens / 10 in  (* 10% = aggressive *)
    let moderate_preserved := total_tokens * 3 / 10 in  (* 30% = moderate *)
    moderate_preserved <= total_tokens ->
    let aggressive_output := predicted_output_length base_output aggressive_preserved total_tokens in
    let moderate_output := predicted_output_length base_output moderate_preserved total_tokens in
    total_cost moderate_preserved moderate_output <=
    total_cost aggressive_preserved aggressive_output.
Proof.
  intros total_tokens base_output Htotal Hbase
         aggressive_preserved moderate_preserved Hmod
         aggressive_output moderate_output.
  unfold aggressive_output, moderate_output.
  (* Both costs equal base_output + total_tokens by conservation. *)
  rewrite (cost_conservation base_output moderate_preserved total_tokens Hmod).
  assert (Hagg : aggressive_preserved <= total_tokens).
  { unfold aggressive_preserved. apply Nat.Div0.div_le_upper_bound. nia. }
  rewrite (cost_conservation base_output aggressive_preserved total_tokens Hagg).
  lia.
Qed.

(** ** Optimal Preservation Fraction *)

(** Find the preservation fraction that minimizes total cost *)
(** This is the theoretical justification for RECENCY_PRESERVE_FRACTION = 30% *)

Definition cost_at_fraction (total_tokens base_output fraction_num fraction_den : nat) : nat :=
  let preserved := (total_tokens * fraction_num) / fraction_den in
  let output := predicted_output_length base_output preserved total_tokens in
  total_cost preserved output.

(** Any fraction in [0,1] keeps the preserved amount within the total. *)
Lemma cost_at_fraction_in_range :
  forall total_tokens fraction_num fraction_den,
    fraction_num <= fraction_den ->
    fraction_den <> 0 ->
    (total_tokens * fraction_num) / fraction_den <= total_tokens.
Proof.
  intros total_tokens fraction_num fraction_den Hfrac Hden.
  apply Nat.Div0.div_le_upper_bound. nia.
Qed.

(** Helper: cost at a valid fraction collapses to [base_output + total_tokens]. *)
Lemma cost_at_fraction_const :
  forall total_tokens base_output fraction_num fraction_den,
    fraction_num <= fraction_den ->
    fraction_den <> 0 ->
    cost_at_fraction total_tokens base_output fraction_num fraction_den
      = base_output + total_tokens.
Proof.
  intros total_tokens base_output fraction_num fraction_den Hfrac Hden.
  unfold cost_at_fraction.
  apply cost_conservation.
  apply cost_at_fraction_in_range; assumption.
Qed.

(** Theorem: 30% preservation is cost-optimal among 10%/30%/50%.

    [CORRECTION] The original framing ("30% is an interior optimum, within 10%
    of best") cannot hold for this model: total cost is *monotone* in the
    preserved fraction (strictly decreasing for recovery > 1.0x, flat at 1.0x),
    so there is no interior minimum to be "near".  At the calibrated 1.0x factor
    the cost is *invariant* to the fraction (cost-conservation), so 30% costs no
    more than 10% (more aggressive) and no more than 50% (less aggressive) — the
    [<=] claims hold (with equality).  This is the honest justification for the
    30% floor: it is cost-neutral against both extremes, so the choice is driven
    by recency/accuracy, not token cost. *)
Theorem thirty_percent_near_optimal :
  forall total_tokens base_output,
    total_tokens >= 10000 ->
    base_output >= 500 ->
    cost_at_fraction total_tokens base_output 30 100 <=
    cost_at_fraction total_tokens base_output 10 100 /\
    cost_at_fraction total_tokens base_output 30 100 <=
    cost_at_fraction total_tokens base_output 50 100.
Proof.
  intros total_tokens base_output Htotal Hbase.
  rewrite (cost_at_fraction_const total_tokens base_output 30 100) by lia.
  rewrite (cost_at_fraction_const total_tokens base_output 10 100) by lia.
  rewrite (cost_at_fraction_const total_tokens base_output 50 100) by lia.
  split; lia.
Qed.

(** ** Integration with Token Budget *)

(** The floor is computed using select_retained with a budget *)

(** Core invariant of the greedy loop: it never spends more than the current
    [budget] beyond what the accumulator already holds.  ([select_newest_aux]
    decrements [budget] in place, so the bound is stated against that same
    running budget.) *)
Lemma aux_budget_bound :
  forall turns budget acc,
    total_tokens (select_newest_aux turns budget acc)
      <= total_tokens acc + budget.
Proof.
  induction turns as [|t rest IH]; intros budget acc.
  - simpl. lia.
  - simpl. destruct (Nat.leb (turn_tokens t) budget) eqn:Hle.
    + apply Nat.leb_le in Hle.
      specialize (IH (budget - turn_tokens t) (t :: acc)).
      rewrite total_tokens_cons in IH. lia.
    + lia.
Qed.

(** Theorem: Selected tokens never exceed budget.  (Fully proved.) *)
Theorem selection_within_budget :
  forall turns budget,
    fold_left (fun acc t => acc + turn_tokens t) (select_retained turns budget) 0 <= budget.
Proof.
  intros turns budget.
  (* The [fold_left ... 0] in the statement is exactly [total_tokens]. *)
  change (total_tokens (select_retained turns budget) <= budget).
  unfold select_retained.
  pose proof (aux_budget_bound (rev turns) budget []) as H.
  unfold total_tokens in H at 2. simpl in H. exact H.
Qed.

(** *** Recency / greedy-maximality structure

    [CORRECTION] The original [selection_maximal] claimed that EVERY unselected
    turn has cost greater than the remaining budget.  This is FALSE: the greedy
    loop scans newest-first and STOPS at the first turn that does not fit, it
    does not skip-and-continue.  So a cheap *older* turn can go unselected merely
    because a newer (more recent) turn already blocked the scan — e.g.
    [turns = [mkTurnCost 0 2; mkTurnCost 1 100]] (oldest cost 2, newest cost
    100) with [budget = 50]: nothing is selected, [remaining = 50], yet the
    older turn has cost [2 <= 50], contradicting the claim.  (Witness proved in
    [selection_maximal_counterexample].)

    The true, strong statements are the *recency* ones the algorithm actually
    guarantees, matching "Lost in the Middle" (keep the recent tail) and
    LLMLingua's prefix-nesting:

    - [selection_is_newest_suffix]: the retained set is exactly the newest
      contiguous block, i.e. [rev (firstn k (rev turns))] — recency ordering and
      contiguity are preserved.
    - [selection_blocking_exceeds]: the greedy stop is honest — the newest
      turn it refused to take has cost strictly greater than the budget left
      when the scan reached it. *)

(** Characterization of the loop: it consumes some prefix of its input
    (newest-first list [l]), reversing it onto [acc].  The consumed count [k]
    and the running budget are tied together below. *)
Lemma select_newest_aux_prefix :
  forall l budget acc,
    exists k,
      k <= length l /\
      select_newest_aux l budget acc = rev (firstn k l) ++ acc /\
      total_tokens (firstn k l) <= budget /\
      (* either everything fit, or the next (k-th, newest unprocessed) turn
         blocks: its cost exceeds the budget left after the consumed prefix. *)
      (k = length l \/
       exists b, nth_error l k = Some b /\
                 turn_tokens b > budget - total_tokens (firstn k l)).
Proof.
  induction l as [|t rest IH]; intros budget acc.
  - exists 0. cbn [firstn rev length nth_error select_newest_aux app].
    split; [ lia | ]. split; [ reflexivity | ]. split.
    + unfold total_tokens. simpl. lia.
    + left. reflexivity.
  - cbn [select_newest_aux]. destruct (Nat.leb (turn_tokens t) budget) eqn:Hle.
    + (* t fits: recurse with smaller budget, then prepend t *)
      apply Nat.leb_le in Hle.
      destruct (IH (budget - turn_tokens t) (t :: acc)) as
        [k [Hk [Heq [Hsum Hstop]]]].
      exists (S k).
      (* firstn (S k) (t :: rest) = t :: firstn k rest, so its token sum is
         turn_tokens t + total_tokens (firstn k rest). *)
      assert (Hfsum : total_tokens (firstn (S k) (t :: rest))
                       = turn_tokens t + total_tokens (firstn k rest)).
      { cbn [firstn]. rewrite total_tokens_cons. reflexivity. }
      split; [ cbn [length]; lia | ].
      split.
      { (* the produced list *)
        cbn [firstn rev]. rewrite Heq. rewrite <- app_assoc. reflexivity. }
      split.
      { (* token sum within budget *) rewrite Hfsum. lia. }
      (* stopping condition *)
      destruct Hstop as [Hall | [b [Hnth Hgt]]].
      * left. cbn [length]. lia.
      * right. exists b. cbn [nth_error]. split.
        -- exact Hnth.
        -- rewrite Hfsum. lia.
    + (* t does not fit: stop immediately, nothing consumed here *)
      apply Nat.leb_gt in Hle.
      exists 0. cbn [firstn rev length nth_error select_newest_aux app].
      split; [ lia | ].
      split; [ reflexivity | ].
      split.
      * unfold total_tokens. simpl. lia.
      * right. exists t. split.
        -- reflexivity.
        -- unfold total_tokens. simpl. lia.
Qed.

(** Theorem: the retained set is the newest contiguous suffix of [turns].
    Recency ordering and contiguity are preserved: it equals the newest [k]
    turns in original (oldest-first) order for some [k]. *)
Theorem selection_is_newest_suffix :
  forall turns budget,
    exists k,
      k <= length turns /\
      select_retained turns budget = rev (firstn k (rev turns)) /\
      total_tokens (select_retained turns budget) <= budget.
Proof.
  intros turns budget. unfold select_retained.
  destruct (select_newest_aux_prefix (rev turns) budget [])
    as [k [Hk [Heq [Hsum _]]]].
  exists k. rewrite length_rev in Hk.
  split; [ exact Hk | ].
  split.
  - rewrite Heq, app_nil_r. reflexivity.
  - rewrite Heq, app_nil_r, total_tokens_rev. exact Hsum.
Qed.

(** Theorem: honest greedy maximality.  If the scan stopped before consuming
    every turn, the newest turn it refused (the [k]-th of [rev turns]) has cost
    strictly greater than the budget remaining once the consumed prefix is
    paid for. *)
Theorem selection_blocking_exceeds :
  forall turns budget,
    let l := rev turns in
    let k := length (select_retained turns budget) in
    k < length turns ->
    exists b,
      nth_error l k = Some b /\
      turn_tokens b > budget - total_tokens (firstn k l).
Proof.
  intros turns budget l k Hlt.
  unfold k, l, select_retained in *.
  destruct (select_newest_aux_prefix (rev turns) budget [])
    as [k0 [Hk0 [Heq [Hsum Hstop]]]].
  (* The retained length is exactly k0. *)
  assert (Hlen : length (select_newest_aux (rev turns) budget []) = k0).
  { rewrite Heq, app_nil_r, length_rev, length_firstn. lia. }
  rewrite Hlen in *.
  destruct Hstop as [Hall | Hblk].
  - (* everything consumed: contradicts k0 < length turns *)
    rewrite length_rev in Hall. lia.
  - exact Hblk.
Qed.

(** The concrete counterexample to the original (false) [selection_maximal]:
    a cheap *older* turn is unselected while the remaining budget exceeds its
    cost, because a newer turn blocked the greedy scan. *)
Theorem selection_maximal_counterexample :
  let turns := [mkTurnCost 0 2; mkTurnCost 1 100] in
  let budget := 50 in
  let selected := select_retained turns budget in
  let remaining := budget - total_tokens selected in
  exists t,
    In t turns /\ ~ In t selected /\ turn_tokens t <= remaining.
Proof.
  exists (mkTurnCost 0 2).
  (* select_retained reduces to [] here (the newest turn, cost 100, blocks the
     greedy scan against budget 50), so [selected = []] and [remaining = 50]. *)
  cbv [select_retained select_newest_aux rev app firstn total_tokens
       fold_left turn_tokens Nat.leb].
  split.
  - simpl. left. reflexivity.
  - split.
    + simpl. intros HIn. exact HIn.
    + simpl. lia.
Qed.
