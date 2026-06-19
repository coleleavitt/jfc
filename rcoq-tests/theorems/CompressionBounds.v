(** * CompressionBounds: Information-Theoretic Limits on Context Compression

    This module formalizes bounds on how much LLM context can be compressed
    while preserving semantic information, and proves correctness/optimality
    properties of JFC's greedy retention and token-gap-step algorithms.

    Every theorem below is fully proved (no [admit]/[Admitted]).  Where the
    original statement was false as written, it has been restated to a true
    *and* non-trivial theorem; each such change is documented inline with a
    [CORRECTION] note explaining why the weaker/false form does not hold and
    what modeling hypothesis makes the real bound provable.

    References (note: the local extract files are MISLABELED; attributions
    below use the actual papers):
    - Shannon, "A Mathematical Theory of Communication" (1948)
    - Cover & Thomas, "Elements of Information Theory"
    - LLMLingua, Jiang et al. 2023 (arXiv:2310.05736): budget-conservation
      identity tau*L = tau_ins*L_ins + tau_dems*L_dems + tau_que*L_que, and the
      greedy demonstration cap L~_D <= k*tau_dems*L_dems.
    - LLMLingua-2, Pan et al. 2024 (arXiv:2403.12968): extractive budget
      N~ = tau*N <= N (compression never grows the prompt).
    - crates/jfc-core/src/retention.rs (select_retained)
    - crates/jfc-engine/src/compact/engine.rs (token_gap_step)
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Generic fold_left accumulator-shift lemma

    All the "sum a numeric field over a list" definitions below are
    [fold_left (fun acc x => acc + f x) l 0].  This lemma lets us peel the
    accumulator out so we can reason by [cons] equations. *)
Lemma fold_left_add_shift :
  forall (A : Type) (f : A -> nat) (l : list A) (z : nat),
    fold_left (fun acc x => acc + f x) l z
      = z + fold_left (fun acc x => acc + f x) l 0.
Proof.
  intros A f l. induction l as [|x xs IH]; intros z.
  - simpl. lia.
  - simpl. rewrite (IH (z + f x)). rewrite (IH (f x)). lia.
Qed.

(** ** Basic Definitions *)

(** A message with token count and semantic weight *)
Record Message : Type := mkMessage {
  msg_tokens : nat;
  msg_semantic_weight : nat;  (* Approximates information content *)
}.

(** Compression ratio: output_tokens / input_tokens, as a percentage *)
Definition compression_ratio (input output : nat) : nat :=
  if Nat.eqb input 0 then 0 else (output * 100) / input.

(** ** Compression Ratio Bounds *)

(** Theorem: Compression ratio <= 100% when the summary is no larger.

    This is LLMLingua-2's invariant N~ = tau*N <= N expressed as a percentage:
    an extractive/abstractive summary that is not longer than its input has
    ratio at most 100. *)
Theorem compression_ratio_bounded :
  forall input output : nat,
    output <= input ->
    compression_ratio input output <= 100.
Proof.
  intros input output Hle.
  unfold compression_ratio.
  destruct (Nat.eqb input 0) eqn:Hinput.
  - (* input = 0 *) lia.
  - (* input > 0 *)
    apply Nat.Div0.div_le_upper_bound.
    (* goal: output * 100 <= input * 100 *)
    nia.
Qed.

(** ** Retention Selection Model

    Models the greedy retention selection algorithm from jfc-core.
    Given a budget and list of (id, token_cost) pairs, select the
    newest items that fit within budget. *)

Record TurnCost : Type := mkTurnCost {
  turn_id : nat;
  turn_tokens : nat;
}.

(** Greedy selection: take from end until budget exhausted *)
Fixpoint select_retained_aux (turns : list TurnCost) (budget remaining : nat)
    (acc : list TurnCost) : list TurnCost :=
  match turns with
  | [] => acc
  | t :: rest =>
      if Nat.leb (turn_tokens t) remaining then
        select_retained_aux rest budget (remaining - turn_tokens t) (t :: acc)
      else
        acc
  end.

Definition select_retained (turns : list TurnCost) (budget : nat) : list TurnCost :=
  select_retained_aux (rev turns) budget budget [].

(** Total tokens in a selection *)
Definition total_tokens (turns : list TurnCost) : nat :=
  fold_left (fun acc t => acc + turn_tokens t) turns 0.

Lemma total_tokens_cons :
  forall t l, total_tokens (t :: l) = turn_tokens t + total_tokens l.
Proof.
  intros t l. unfold total_tokens. simpl.
  rewrite (fold_left_add_shift TurnCost turn_tokens l (turn_tokens t)). lia.
Qed.

(** ** Retention Selection Theorems *)

(** Core invariant: the aux loop never spends more than [remaining] tokens
    beyond what is already in the accumulator. *)
Lemma aux_bound :
  forall turns budget remaining acc,
    total_tokens (select_retained_aux turns budget remaining acc)
      <= total_tokens acc + remaining.
Proof.
  induction turns as [|t rest IH]; intros budget remaining acc.
  - simpl. lia.
  - simpl. destruct (Nat.leb (turn_tokens t) remaining) eqn:Hle.
    + apply Nat.leb_le in Hle.
      specialize (IH budget (remaining - turn_tokens t) (t :: acc)).
      rewrite total_tokens_cons in IH. lia.
    + lia.
Qed.

(** Theorem: Selected tokens never exceed budget.  (Fully proved.) *)
Theorem budget_satisfied :
  forall turns budget,
    total_tokens (select_retained turns budget) <= budget.
Proof.
  intros turns budget. unfold select_retained.
  pose proof (aux_bound (rev turns) budget budget []) as H.
  assert (total_tokens (@nil TurnCost) = 0) as H0 by reflexivity.
  rewrite H0 in H. lia.
Qed.

(** The aux loop only ever grows the accumulator. *)
Lemma aux_length_grows :
  forall turns budget remaining acc,
    length acc <= length (select_retained_aux turns budget remaining acc).
Proof.
  induction turns as [|t rest IH]; intros budget remaining acc.
  - simpl. lia.
  - simpl. destruct (Nat.leb (turn_tokens t) remaining).
    + specialize (IH budget (remaining - turn_tokens t) (t :: acc)).
      simpl in IH. lia.
    + lia.
Qed.

(** Monotonicity of the aux loop in (remaining, accumulator-length).  The
    [budget] argument is threaded but never read, so it may differ on the two
    sides (as it does in [retention_monotonic], where it equals each budget). *)
Lemma aux_mono :
  forall turns b1 b2 r1 r2 acc1 acc2,
    r1 <= r2 ->
    length acc1 <= length acc2 ->
    length (select_retained_aux turns b1 r1 acc1)
      <= length (select_retained_aux turns b2 r2 acc2).
Proof.
  induction turns as [|t rest IH]; intros b1 b2 r1 r2 acc1 acc2 Hr Hacc.
  - simpl. exact Hacc.
  - simpl.
    destruct (Nat.leb (turn_tokens t) r1) eqn:H1;
      destruct (Nat.leb (turn_tokens t) r2) eqn:H2.
    + apply Nat.leb_le in H1. apply Nat.leb_le in H2.
      apply IH; simpl; lia.
    + apply Nat.leb_le in H1. apply Nat.leb_gt in H2. lia.
    + apply Nat.leb_gt in H1. apply Nat.leb_le in H2.
      pose proof (aux_length_grows rest b2 (r2 - turn_tokens t) (t :: acc2)) as G.
      simpl in G. lia.
    + exact Hacc.
Qed.

(** Theorem: Retention is monotonic in budget.  (Fully proved.) *)
Theorem retention_monotonic :
  forall turns b1 b2,
    b1 <= b2 ->
    length (select_retained turns b1) <= length (select_retained turns b2).
Proof.
  intros turns b1 b2 Hle. unfold select_retained.
  apply aux_mono.
  - exact Hle.
  - simpl. lia.
Qed.

(** ** Greedy Optimality for Uniform Costs

    When all items have the same cost c, greedy selection retains exactly
    min(#items, budget/c) of them. *)

Definition all_same_cost (turns : list TurnCost) (c : nat) : Prop :=
  forall t, In t turns -> turn_tokens t = c.

(** Exact count under uniform cost: the loop fills as many slots as the
    budget allows, capped by the number of items available. *)
Lemma aux_uniform :
  forall gs budget c,
    c > 0 ->
    (forall t, In t gs -> turn_tokens t = c) ->
    forall remaining acc,
      length (select_retained_aux gs budget remaining acc)
        = length acc + Nat.min (length gs) (remaining / c).
Proof.
  intros gs budget c Hc. induction gs as [|t rest IH]; intros Hin remaining acc.
  - cbn [select_retained_aux length]. lia.
  - assert (Htt : turn_tokens t = c) by (apply Hin; left; reflexivity).
    assert (Hin_rest : forall t0, In t0 rest -> turn_tokens t0 = c)
      by (intros t0 Ht0; apply Hin; right; exact Ht0).
    specialize (IH Hin_rest).
    cbn [select_retained_aux length].
    rewrite Htt.
    destruct (Nat.leb c remaining) eqn:Hcmp.
    + apply Nat.leb_le in Hcmp.
      rewrite IH.
      cbn [length].
      assert (Hrem : remaining / c = S ((remaining - c) / c)).
      { replace remaining with (1 * c + (remaining - c)) at 1 by lia.
        rewrite Nat.div_add_l by lia. lia. }
      rewrite Hrem. lia.
    + apply Nat.leb_gt in Hcmp.
      assert (Hd0 : remaining / c = 0) by (apply Nat.div_small; lia).
      rewrite Hd0. lia.
Qed.

(** Theorem: Greedy is optimal for uniform costs.

    [CORRECTION] The original statement claimed
      length (select_retained turns budget) = budget / c
    which is FALSE whenever there are fewer than budget/c items available
    (e.g. turns = [] gives length 0, not budget/c).  The true, strictly more
    general theorem is the min with the number of available items. *)
Theorem greedy_optimal_uniform :
  forall turns budget c,
    c > 0 ->
    all_same_cost turns c ->
    length (select_retained turns budget) = Nat.min (length turns) (budget / c).
Proof.
  intros turns budget c Hc Hall. unfold select_retained.
  assert (Hin' : forall t, In t (rev turns) -> turn_tokens t = c).
  { intros t Ht. rewrite <- in_rev in Ht. exact (Hall t Ht). }
  rewrite (aux_uniform (rev turns) budget c Hc Hin' budget []).
  cbn [length]. rewrite length_rev. reflexivity.
Qed.

(** ** Information-Theoretic Lower Bound

    Formalization of the semantic compression limit.  The compressed
    representation cannot be smaller than the information content of the
    semantic signal it must carry. *)

(** Semantic content approximation (sum of semantic weights) *)
Definition semantic_content (msgs : list Message) : nat :=
  fold_left (fun acc m => acc + msg_semantic_weight m) msgs 0.

(** Total tokens in messages *)
Definition message_tokens (msgs : list Message) : nat :=
  fold_left (fun acc m => acc + msg_tokens m) msgs 0.

Lemma semantic_content_cons :
  forall m l, semantic_content (m :: l) = msg_semantic_weight m + semantic_content l.
Proof.
  intros m l. unfold semantic_content. simpl.
  rewrite (fold_left_add_shift Message msg_semantic_weight l (msg_semantic_weight m)).
  lia.
Qed.

Lemma message_tokens_cons :
  forall m l, message_tokens (m :: l) = msg_tokens m + message_tokens l.
Proof.
  intros m l. unfold message_tokens. simpl.
  rewrite (fold_left_add_shift Message msg_tokens l (msg_tokens m)). lia.
Qed.

(** A summary is valid if it preserves semantic content.  This is the
    discrete analog of LLMLingua's KL fidelity objective: the compressed
    context must carry at least the original's semantic signal. *)
Definition valid_summary (original summary : list Message) : Prop :=
  semantic_content summary >= semantic_content original.

(** max_semantic_density = max semantic units a single token can carry.
    Heuristic ~10 tokens per semantic unit -> 1 token <= 10 semantic units. *)
Definition max_semantic_density : nat := 10.

(** Per-message density bound: each message's semantic weight is at most its
    token count times the density.  This is the Shannon-style channel
    constraint (a token has bounded carrying capacity), here a hypothesis. *)
Definition density_bound (msgs : list Message) (D : nat) : Prop :=
  forall m, In m msgs -> msg_semantic_weight m <= msg_tokens m * D.

(** Corpus-level consequence of the per-message density bound. *)
Lemma semantic_content_le_tokens :
  forall msgs D,
    density_bound msgs D ->
    semantic_content msgs <= message_tokens msgs * D.
Proof.
  induction msgs as [|m rest IH]; intros D Hd.
  - cbn [semantic_content message_tokens fold_left]. lia.
  - rewrite semantic_content_cons, message_tokens_cons.
    assert (Hm : msg_semantic_weight m <= msg_tokens m * D)
      by (apply Hd; left; reflexivity).
    assert (Hrest : semantic_content rest <= message_tokens rest * D)
      by (apply IH; intros m' Hm'; apply Hd; right; exact Hm').
    rewrite Nat.mul_add_distr_r. lia.
Qed.

(** Theorem: Semantic content bounds minimum summary size.

    [CORRECTION] The original statement had no density hypothesis, so it did
    not follow (nothing related token counts to semantic content).  Adding the
    per-token carrying-capacity bound [density_bound] (the actual Shannon
    assumption) makes it a real lower bound: to preserve the original's
    semantic content the summary needs >= content / density tokens. *)
Theorem semantic_lower_bound :
  forall original summary,
    density_bound summary max_semantic_density ->
    valid_summary original summary ->
    message_tokens summary * max_semantic_density >= semantic_content original.
Proof.
  intros original summary Hd Hv. unfold valid_summary in Hv.
  pose proof (semantic_content_le_tokens summary max_semantic_density Hd) as Hle.
  lia.
Qed.

(** ** Compression Cannot Beat Entropy *)

(** Entropy approximation (number of distinct semantic states) *)
Definition approx_entropy (msgs : list Message) : nat :=
  length msgs.

(** Bits per token, assuming log2(vocab_size) bits.  ~65k vocab = 16 bits. *)
Definition vocab_bits : nat := 16.

(** Minimum tokens to represent [entropy_bits] bits (ceiling division). *)
Definition min_tokens_for_entropy (entropy_bits : nat) : nat :=
  (entropy_bits + vocab_bits - 1) / vocab_bits.

(** A summary has the channel capacity to encode [entropy_bits] bits iff its
    tokens * bits-per-token cover them.  This is the channel-capacity premise
    of the source-coding bound. *)
Definition encodes_entropy (summary : list Message) (entropy_bits : nat) : Prop :=
  entropy_bits <= message_tokens summary * vocab_bits.

(** Theorem: Compression cannot beat entropy.

    [CORRECTION] The original statement related [message_tokens summary] to
    [valid_summary] (a semantic-content predicate) with no bridge between the
    two quantities, so it was not provable.  The honest information-theoretic
    statement is: if the summary has the channel capacity to encode the
    original's entropy, then it needs at least ceil(entropy / bits-per-token)
    tokens.  That is exactly the source-coding (Shannon) lower bound. *)
Theorem compression_entropy_bound :
  forall original summary,
    encodes_entropy summary (approx_entropy original) ->
    message_tokens summary >= min_tokens_for_entropy (approx_entropy original).
Proof.
  intros original summary Hcap.
  unfold encodes_entropy in Hcap. unfold min_tokens_for_entropy.
  set (e := approx_entropy original) in *.
  set (t := message_tokens summary) in *.
  pose proof (Nat.div_mod_eq (e + vocab_bits - 1) vocab_bits) as Hdm.
  pose proof (Nat.mod_upper_bound (e + vocab_bits - 1) vocab_bits
                ltac:(unfold vocab_bits; lia)) as Hmod.
  unfold vocab_bits in *. nia.
Qed.

(** ** Practical Compression Ratio for JFC

    Empirically JFC's summarization achieves ~3-5x compression.  At 3-5x the
    output/input ratio sits in [20%, 33%]. *)

Definition practical_compression_min : nat := 20.  (* 20% = 5x compression *)
Definition practical_compression_max : nat := 33.  (* 33% = 3x compression *)

(** Theorem: Achievable compression is within practical bounds.

    [CORRECTION] The original hypotheses were [output * 5 <= input] and
    [input <= output * 3], i.e. 5*output <= input <= 3*output, which forces
    output = 0 — they are mutually contradictory for any real summary.  The
    intended meaning ("between 3x and 5x compression") is
      output * 3 <= input   (at least 3x: output <= input/3)
      input <= output * 5   (at most 5x:  output >= input/5)
    under which the ratio provably lands in [20, 33]. *)
Theorem practical_compression_achievable :
  forall input output,
    output > 0 ->
    output * 3 <= input ->
    input <= output * 5 ->
    practical_compression_min <= compression_ratio input output /\
    compression_ratio input output <= practical_compression_max.
Proof.
  intros input output Hout H3 H5.
  unfold compression_ratio, practical_compression_min, practical_compression_max.
  assert (Hin : input > 0) by lia.
  destruct (Nat.eqb input 0) eqn:Heq.
  - apply Nat.eqb_eq in Heq. lia.
  - split.
    + (* 20 <= output * 100 / input *)
      apply Nat.div_le_lower_bound; [ lia | nia ].
    + (* output * 100 / input <= 33 *)
      set (q := (output * 100) / input).
      pose proof (Nat.div_mod_eq (output * 100) input) as Hdm.
      pose proof (Nat.mod_upper_bound (output * 100) input ltac:(lia)) as Hmod.
      assert (input * q <= output * 100) by lia.
      nia.
Qed.

(** ** Token Gap Step Correctness

    The token_gap_step function computes how many group-of-turns to preserve
    so that the preserved groups' tokens cover the token gap the API reported.
    This mirrors crates/jfc-engine/src/compact/engine.rs. *)

(** Sum of a list of token counts. *)
Fixpoint sumn (l : list nat) : nat :=
  match l with
  | [] => 0
  | x :: r => x + sumn r
  end.

(** Number of groups (taken from the front of [gs]) whose tokens cover
    [remaining].  Stops as soon as the running gap reaches 0. *)
Fixpoint groups_to_cover (gs : list nat) (remaining : nat) : nat :=
  match gs with
  | [] => 0
  | t :: rest =>
      if Nat.leb remaining 0 then 0
      else S (groups_to_cover rest (remaining - t))
  end.

(** Step function model.  With a gap [g], preserve enough of the newest
    [split] groups to cover [g]; otherwise fall back to halving. *)
Definition token_gap_step (gap : option nat) (group_tokens : list nat) (split : nat) : nat :=
  match gap with
  | None => Nat.max 1 (split / 2)
  | Some g => Nat.max 1 (groups_to_cover (rev (firstn split group_tokens)) g)
  end.

(** Theorem: Step is always at least 1.  (Fully proved.) *)
Theorem step_at_least_one :
  forall gap group_tokens split,
    token_gap_step gap group_tokens split >= 1.
Proof.
  intros gap group_tokens split.
  unfold token_gap_step.
  destruct gap; apply Nat.le_max_l.
Qed.

(** Coverage: the groups counted by [groups_to_cover] really do sum to at
    least the gap, whenever the available groups can cover it at all. *)
Lemma groups_to_cover_covers :
  forall gs g,
    0 < g ->
    g <= sumn gs ->
    g <= sumn (firstn (groups_to_cover gs g) gs).
Proof.
  induction gs as [|t rest IH]; intros g Hg Hsum.
  - cbn [sumn] in Hsum. lia.
  - cbn [groups_to_cover].
    destruct (Nat.leb g 0) eqn:Hle0.
    + apply Nat.leb_le in Hle0. lia.
    + apply Nat.leb_gt in Hle0.
      cbn [firstn sumn] in *.
      destruct (Nat.leb g t) eqn:Hgt.
      * apply Nat.leb_le in Hgt. lia.
      * apply Nat.leb_gt in Hgt.
        specialize (IH (g - t) ltac:(lia) ltac:(lia)).
        lia.
Qed.

(** Theorem: the token-gap step preserves enough groups to cover the gap.

    [CORRECTION] The original [step_covers_gap] concluded [True] (a vacuous
    placeholder).  This is the real coverage guarantee: when the newest
    [split] groups have enough tokens to cover a positive gap [g], the number
    of groups the step preserves does sum to at least [g]. *)
Theorem token_gap_step_covers :
  forall g gts split,
    0 < g ->
    g <= sumn (rev (firstn split gts)) ->
    g <= sumn (firstn (token_gap_step (Some g) gts split)
                      (rev (firstn split gts))).
Proof.
  intros g gts split Hg Hsum.
  unfold token_gap_step.
  set (L := rev (firstn split gts)) in *.
  assert (Hpos : groups_to_cover L g >= 1).
  { destruct L as [|x xs].
    - cbn [sumn] in Hsum. lia.
    - cbn [groups_to_cover].
      assert (Nat.leb g 0 = false) as Hf by (apply Nat.leb_gt; lia).
      rewrite Hf. lia. }
  assert (Hmax : Nat.max 1 (groups_to_cover L g) = groups_to_cover L g) by lia.
  rewrite Hmax.
  apply groups_to_cover_covers; assumption.
Qed.
