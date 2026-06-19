(** * AttentionPruning: Formal Model of Attention-Aware Context Pruning

    This module formalizes attention-/score-based token selection for context
    compression, following the score-threshold pruning rule of LLMLingua
    (Jiang et al. 2023, arXiv:2310.05736) and LongLLMLingua
    (arXiv:2310.06839).  In those works a token x_i is *kept* iff its
    importance score p(x_i) exceeds a threshold gamma (Eq. 7:
      s~ = { x_i | p(x_i) > gamma }),
    and low-score (low-perplexity / low-MI) tokens, which contribute
    negligibly to entropy, are pruned first.

    We model the score with a token's [attention_weight] (a nat, scaled
    0-1000) and prove the real guarantees that rule must satisfy:

    1. Pruning never increases length (prune is a sublist of the input).
    2. Retention is monotone in the threshold: lowering gamma keeps a
       superset (and only ever keeps at least as many tokens).
    3. The kept set is upward-closed in score: if a kept token has score <=
       some other input token's score, that other token is kept too.
    4. Pruning is idempotent.
    5. Every pruned token has score <= gamma; every kept token has score
       >= gamma (soundness / completeness of the threshold cut).
    6. Top-k selection from a score-sorted list preserves the score ordering
       and the sample's average score is at least the whole set's average.

    Every theorem is fully proved (no [admit]/[Admitted]).  Where the
    original statement was false or vacuous, it has been restated to a true
    and still-strong theorem with an inline [CORRECTION] note.

    References:
    - Vaswani et al., "Attention Is All You Need" (2017)
    - LLMLingua, Jiang et al. 2023 (arXiv:2310.05736)
    - LongLLMLingua, Jiang et al. 2023 (arXiv:2310.06839)
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.Sorting.Permutation.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Token with Attention / Importance Weight *)

Record Token : Type := mkToken {
  token_id : nat;
  token_position : nat;
  attention_weight : nat;  (* importance score p(x_i), scaled 0-1000 *)
}.

(** ** Attention Score Computation

    Simplified model: attention[i,j] = softmax(Q_i . K_j / sqrt(d)).
    We abstract this to a per-token importance score [attention_weight].
*)

Definition AttentionScores := list Token.

(** Sort by attention weight descending *)
Definition weight_ge (t1 t2 : Token) : bool :=
  Nat.leb (attention_weight t2) (attention_weight t1).

(** Check if list is sorted by weight descending *)
Fixpoint sorted_by_weight (ts : list Token) : Prop :=
  match ts with
  | [] => True
  | [_] => True
  | t1 :: (t2 :: _) as rest =>
      attention_weight t1 >= attention_weight t2 /\ sorted_by_weight rest
  end.

(** *** Structural facts about [sorted_by_weight] *)

(** The tail of a sorted list is sorted. *)
Lemma sorted_tail :
  forall a l, sorted_by_weight (a :: l) -> sorted_by_weight l.
Proof.
  intros a l H. destruct l as [|b r].
  - simpl. exact I.
  - simpl in H. destruct H as [_ Hs]. exact Hs.
Qed.

(** The head of a sorted list dominates every later element. *)
Lemma sorted_head_ge :
  forall a l,
    sorted_by_weight (a :: l) ->
    forall x, In x l -> attention_weight a >= attention_weight x.
Proof.
  intros a l. revert a. induction l as [|b r IH]; intros a Hs x Hin.
  - simpl in Hin. contradiction.
  - simpl in Hs. destruct Hs as [Hab Hbr].
    simpl in Hin. destruct Hin as [Hxb | Hxr].
    + subst x. exact Hab.
    + (* x in r : a >= b >= x *)
      assert (attention_weight b >= attention_weight x) as Hbx
        by (apply (IH b Hbr x Hxr)).
      lia.
Qed.

(** Descending order is realized pointwise: earlier index has weight >= later
    index.  This is the core ordering invariant of a score-sorted list. *)
Lemma sorted_nth_desc :
  forall l,
    sorted_by_weight l ->
    forall i j a b,
      nth_error l i = Some a ->
      nth_error l j = Some b ->
      i <= j ->
      attention_weight a >= attention_weight b.
Proof.
  induction l as [|h t IH]; intros Hs i j a b Hi Hj Hij.
  - destruct i; simpl in Hi; discriminate.
  - destruct i as [|i'].
    + (* a = h *)
      simpl in Hi. injection Hi as Hi. subst a.
      destruct j as [|j'].
      * simpl in Hj. injection Hj as Hj. subst b. lia.
      * (* b is in t *)
        simpl in Hj.
        assert (In b t) as Hinb by (eapply nth_error_In; exact Hj).
        apply (sorted_head_ge h t Hs b Hinb).
    + (* i = S i', so j = S j' since j >= i > 0 *)
      destruct j as [|j']; [ lia | ].
      simpl in Hi, Hj.
      apply (IH (sorted_tail h t Hs) i' j' a b Hi Hj). lia.
Qed.

(** Any prefix of a sorted list is sorted. *)
Lemma sorted_firstn :
  forall k l, sorted_by_weight l -> sorted_by_weight (firstn k l).
Proof.
  induction k as [|k IHk]; intros l Hs.
  - simpl. exact I.
  - destruct l as [|h t].
    + simpl. exact I.
    + (* firstn (S k) (h::t) = h :: firstn k t *)
      assert (Hst : sorted_by_weight (firstn k t)) by (apply IHk; eapply sorted_tail; exact Hs).
      simpl.
      destruct (firstn k t) as [|b r] eqn:Hf.
      * simpl. exact I.
      * (* need attention_weight h >= attention_weight b /\ sorted (b::r) *)
        split; [ | exact Hst ].
        (* b is the first element of firstn k t, hence b is in t *)
        assert (In b t) as Hbt.
        { assert (In b (firstn k t)) as Hbf by (rewrite Hf; left; reflexivity).
          rewrite <- (firstn_skipn k t). apply in_or_app. left. exact Hbf. }
        apply (sorted_head_ge h t Hs b Hbt).
Qed.

(** ** Top-K Selection *)

(** Select top k tokens by attention weight (assumes pre-sorted by weight). *)
Definition top_k (tokens : list Token) (k : nat) : list Token :=
  firstn k tokens.

(** Theorem: Top-k preserves the score ordering within the selection: a token
    at an earlier index has weight at least that of a token at a later index.
    (Fully proved; the original was [admit]/[Admitted].) *)
Theorem top_k_preserves_position_order :
  forall tokens k t1 t2 i j,
    sorted_by_weight tokens ->
    In t1 (top_k tokens k) ->
    In t2 (top_k tokens k) ->
    nth_error (top_k tokens k) i = Some t1 ->
    nth_error (top_k tokens k) j = Some t2 ->
    i < j ->
    attention_weight t1 >= attention_weight t2.
Proof.
  intros tokens k t1 t2 i j Hsorted _ _ Hi Hj Hij.
  unfold top_k in *.
  assert (Hpref : sorted_by_weight (firstn k tokens)) by (apply sorted_firstn; exact Hsorted).
  apply (sorted_nth_desc (firstn k tokens) Hpref i j t1 t2 Hi Hj). lia.
Qed.

(** Theorem: Top-k output is never larger than input.

    [CORRECTION] The original proof used [firstn_le_length], which in this
    Stdlib bounds [length (firstn k l) <= k], not [<= length l]; it did not
    close this goal.  The size bound [length (firstn k l) <= length l] follows
    from [length_firstn] (= min k (length l)) and [Nat.le_min_r].  The
    statement is unchanged. *)
Theorem top_k_size_bound :
  forall tokens k,
    length (top_k tokens k) <= length tokens.
Proof.
  intros tokens k.
  unfold top_k.
  rewrite length_firstn.
  apply Nat.le_min_r.
Qed.

(** Theorem: Top-k output is at most k.

    [CORRECTION] The original proof used [firstn_length_le] (an *equality*
    needing [length tokens <= k]) followed by [lia]; that does not discharge
    [length (firstn k l) <= k] in general.  [firstn_le_length] gives exactly
    this bound in one step.  The statement is unchanged. *)
Theorem top_k_at_most_k :
  forall tokens k,
    length (top_k tokens k) <= k.
Proof.
  intros tokens k.
  unfold top_k.
  apply firstn_le_length.
Qed.

(** ** Threshold Pruning (LLMLingua Eq. 7)

    Keep a token iff its score is at least the threshold [gamma].  (We use
    [>=] rather than the paper's strict [>]; the analogous strict-threshold
    variant is proved below as [strict_threshold_*].) *)
Definition threshold_prune (tokens : list Token) (gamma : nat) : list Token :=
  filter (fun t => Nat.leb gamma (attention_weight t)) tokens.

(** Theorem: Threshold pruning is sound - every token at or above the
    threshold is kept.  (Fully proved.) *)
Theorem threshold_prune_sound :
  forall tokens gamma t,
    In t tokens ->
    attention_weight t >= gamma ->
    In t (threshold_prune tokens gamma).
Proof.
  intros tokens gamma t Hin Hge.
  unfold threshold_prune.
  apply filter_In.
  split.
  - exact Hin.
  - apply Nat.leb_le. exact Hge.
Qed.

(** Theorem: Threshold pruning is complete - every kept token is at or above
    the threshold.  (Fully proved.) *)
Theorem threshold_prune_complete :
  forall tokens gamma t,
    In t (threshold_prune tokens gamma) ->
    attention_weight t >= gamma.
Proof.
  intros tokens gamma t Hin.
  unfold threshold_prune in Hin.
  apply filter_In in Hin.
  destruct Hin as [_ Hfilter].
  apply Nat.leb_le. exact Hfilter.
Qed.

(** Theorem: Every pruned (dropped) token has score <= gamma.  This is the
    dual of completeness and is one of the requested guarantees: a token that
    fails to survive pruning had a sub-threshold score.

    Because the keep test is [gamma <= weight], a dropped token has
    [weight < gamma], hence in particular [weight <= gamma]. *)
Theorem pruned_tokens_below_gamma :
  forall tokens gamma t,
    In t tokens ->
    ~ In t (threshold_prune tokens gamma) ->
    attention_weight t <= gamma.
Proof.
  intros tokens gamma t Hin Hnotin.
  unfold threshold_prune in Hnotin.
  destruct (Nat.leb gamma (attention_weight t)) eqn:Hb.
  - (* would be kept - contradiction *)
    exfalso. apply Hnotin. apply filter_In. split; [ exact Hin | exact Hb ].
  - apply Nat.leb_gt in Hb. lia.
Qed.

(** Theorem: Pruning never increases length (prune is a sub-multiset of the
    input: every kept token was in the input, and the count cannot grow).
    (Fully proved.) *)
Theorem prune_reduces_size :
  forall tokens gamma,
    length (threshold_prune tokens gamma) <= length tokens.
Proof.
  intros tokens gamma.
  unfold threshold_prune.
  apply filter_length_le.
Qed.

(** Theorem: prune is a subset of the input - every kept token came from the
    input.  (Companion to [prune_reduces_size]: the "prune subset of input"
    guarantee at the membership level.) *)
Theorem prune_subset_input :
  forall tokens gamma t,
    In t (threshold_prune tokens gamma) ->
    In t tokens.
Proof.
  intros tokens gamma t Hin.
  unfold threshold_prune in Hin.
  apply filter_In in Hin. destruct Hin as [Hin _]. exact Hin.
Qed.

(** Theorem: Retention is monotone in the threshold.  Lowering gamma keeps a
    superset: if gamma1 <= gamma2, every token kept at the higher threshold
    gamma2 is also kept at the lower threshold gamma1.

    This is the threshold-monotonicity the LLMLingua cut must satisfy: a
    smaller gamma is a strictly more permissive keep rule. *)
Theorem retention_monotone_in_threshold :
  forall tokens gamma1 gamma2 t,
    gamma1 <= gamma2 ->
    In t (threshold_prune tokens gamma2) ->
    In t (threshold_prune tokens gamma1).
Proof.
  intros tokens gamma1 gamma2 t Hle Hin.
  apply threshold_prune_sound.
  - apply (prune_subset_input tokens gamma2 t Hin).
  - assert (attention_weight t >= gamma2) by (apply (threshold_prune_complete tokens gamma2 t Hin)).
    lia.
Qed.

(** Length-level corollary: lowering gamma keeps at least as many tokens. *)
Theorem retention_count_monotone_in_threshold :
  forall tokens gamma1 gamma2,
    gamma1 <= gamma2 ->
    length (threshold_prune tokens gamma2) <= length (threshold_prune tokens gamma1).
Proof.
  intros tokens gamma1 gamma2 Hle.
  unfold threshold_prune.
  (* both are filters of the same list; the gamma2 keep-test implies the
     gamma1 keep-test pointwise, so use NoDup-free sublist counting via
     a direct induction. *)
  induction tokens as [|h r IH].
  - simpl. lia.
  - simpl.
    destruct (Nat.leb gamma2 (attention_weight h)) eqn:H2;
    destruct (Nat.leb gamma1 (attention_weight h)) eqn:H1.
    + simpl. lia.
    + (* kept at gamma2 but not gamma1: impossible since gamma1 <= gamma2 *)
      apply Nat.leb_le in H2. apply Nat.leb_gt in H1. lia.
    + simpl. lia.
    + lia.
Qed.

(** Theorem: The kept set is upward-closed in score.  If a kept token [t] has
    score at most that of some other input token [u], then [u] is kept too.
    Intuitively: pruning never drops a higher-scoring token while retaining a
    lower-scoring one.  (One of the requested guarantees.) *)
Theorem kept_set_upward_closed :
  forall tokens gamma t u,
    In t (threshold_prune tokens gamma) ->
    In u tokens ->
    attention_weight t <= attention_weight u ->
    In u (threshold_prune tokens gamma).
Proof.
  intros tokens gamma t u Ht Hu Hle.
  assert (attention_weight t >= gamma) by (apply (threshold_prune_complete tokens gamma t Ht)).
  apply threshold_prune_sound.
  - exact Hu.
  - lia.
Qed.

(** Theorem: Pruning is idempotent - pruning an already-pruned set at the same
    threshold changes nothing.  (One of the requested guarantees.) *)
Theorem threshold_prune_idempotent :
  forall tokens gamma,
    threshold_prune (threshold_prune tokens gamma) gamma = threshold_prune tokens gamma.
Proof.
  intros tokens gamma.
  unfold threshold_prune.
  induction tokens as [|h r IH].
  - reflexivity.
  - simpl.
    destruct (Nat.leb gamma (attention_weight h)) eqn:Hb.
    + (* kept: outer filter re-tests and keeps it *)
      simpl. rewrite Hb. rewrite IH. reflexivity.
    + exact IH.
Qed.

(** *** Strict-threshold variant (matches the paper's [p(x_i) > gamma]).

    The paper keeps x_i iff p(x_i) > gamma (strict).  We prove the same
    soundness/completeness for the strict cut to show the modeling choice of
    [>=] above is not load-bearing. *)
Definition strict_threshold_prune (tokens : list Token) (gamma : nat) : list Token :=
  filter (fun t => Nat.ltb gamma (attention_weight t)) tokens.

Theorem strict_threshold_prune_complete :
  forall tokens gamma t,
    In t (strict_threshold_prune tokens gamma) ->
    attention_weight t > gamma.
Proof.
  intros tokens gamma t Hin.
  unfold strict_threshold_prune in Hin.
  apply filter_In in Hin. destruct Hin as [_ Hf].
  apply Nat.ltb_lt. exact Hf.
Qed.

Theorem strict_pruned_tokens_at_most_gamma :
  forall tokens gamma t,
    In t tokens ->
    ~ In t (strict_threshold_prune tokens gamma) ->
    attention_weight t <= gamma.
Proof.
  intros tokens gamma t Hin Hnot.
  unfold strict_threshold_prune in Hnot.
  destruct (Nat.ltb gamma (attention_weight t)) eqn:Hb.
  - exfalso. apply Hnot. apply filter_In. split; [ exact Hin | exact Hb ].
  - apply Nat.ltb_ge in Hb. lia.
Qed.

(** ** Attention-Weighted Sampling

    Sample k tokens proportional to attention weights.  We model the
    deterministic high-weight selection (top-k from a score-sorted list). *)
Definition attention_sample (tokens : list Token) (k : nat) : list Token :=
  top_k tokens k.

(** The total attention weight of a token set. *)
Definition total_attention (tokens : list Token) : nat :=
  fold_left (fun acc t => acc + attention_weight t) tokens 0.

(** Accumulator-shift for [total_attention] (cf. CompressionBounds.v). *)
Lemma total_attention_shift :
  forall l z,
    fold_left (fun acc t => acc + attention_weight t) l z
      = z + fold_left (fun acc t => acc + attention_weight t) l 0.
Proof.
  induction l as [|x xs IH]; intros z.
  - simpl. lia.
  - simpl. rewrite (IH (z + attention_weight x)). rewrite (IH (attention_weight x)). lia.
Qed.

Lemma total_attention_cons :
  forall t l, total_attention (t :: l) = attention_weight t + total_attention l.
Proof.
  intros t l. unfold total_attention. simpl.
  rewrite (total_attention_shift l (attention_weight t)). lia.
Qed.

(** If every element's score is >= c, the total is >= length * c. *)
Lemma total_attention_ge :
  forall l c,
    (forall t, In t l -> attention_weight t >= c) ->
    total_attention l >= length l * c.
Proof.
  induction l as [|h r IH]; intros c Hall.
  - simpl. lia.
  - rewrite total_attention_cons. simpl.
    assert (attention_weight h >= c) by (apply Hall; left; reflexivity).
    assert (total_attention r >= length r * c)
      by (apply IH; intros t Ht; apply Hall; right; exact Ht).
    lia.
Qed.

(** If every element's score is <= c, the total is <= length * c. *)
Lemma total_attention_le :
  forall l c,
    (forall t, In t l -> attention_weight t <= c) ->
    total_attention l <= length l * c.
Proof.
  induction l as [|h r IH]; intros c Hall.
  - cbn [length Nat.mul]. unfold total_attention. simpl. lia.
  - rewrite total_attention_cons. simpl.
    assert (attention_weight h <= c) by (apply Hall; left; reflexivity).
    assert (total_attention r <= length r * c)
      by (apply IH; intros t Ht; apply Hall; right; exact Ht).
    lia.
Qed.

(** Pivot lemma: in a score-sorted list, splitting at index k yields a
    threshold value c such that every prefix element scores >= c and every
    suffix element scores <= c.  (Take c = the score of the element at index
    k, the boundary; if the suffix is empty, any c works.) *)
Lemma sorted_split_pivot :
  forall l k,
    sorted_by_weight l ->
    exists c,
      (forall t, In t (firstn k l) -> attention_weight t >= c) /\
      (forall t, In t (skipn k l) -> attention_weight t <= c).
Proof.
  intros l k Hs.
  destruct (skipn k l) as [|p suf] eqn:Hsk.
  - (* suffix empty: pick c = 0; prefix >= 0, suffix vacuous *)
    exists 0. split.
    + intros t _. lia.
    + intros t Hin. simpl in Hin. contradiction.
  - (* suffix nonempty: pivot c = attention_weight p, the element at index k *)
    exists (attention_weight p).
    (* p is at index k of l *)
    assert (Hpk : nth_error l k = Some p).
    { rewrite <- (firstn_skipn k l). rewrite Hsk.
      rewrite nth_error_app2.
      - rewrite length_firstn.
        (* index = k - min k (length l).  When suffix nonempty, k <= length l,
           so min k (length l) = k and we read index 0 of (p::suf). *)
        assert (Hkle : k <= length l).
        { destruct (Nat.le_gt_cases k (length l)) as [Hle | Hgt]; [ exact Hle | ].
          rewrite (skipn_all2 l) in Hsk by lia. discriminate. }
        replace (Nat.min k (length l)) with k by lia.
        rewrite Nat.sub_diag. reflexivity.
      - rewrite length_firstn. lia. }
    split.
    + (* prefix elements: index i < k, so weight >= weight p by sorted_nth_desc *)
      intros t Hin.
      apply In_nth_error in Hin. destruct Hin as [i Hi].
      rewrite nth_error_firstn in Hi.
      destruct (Nat.ltb i k) eqn:Hik; [ | discriminate ].
      apply Nat.ltb_lt in Hik.
      apply (sorted_nth_desc l Hs i k t p Hi Hpk). lia.
    + (* suffix elements: index k + i >= k, so weight <= weight p *)
      intros t Hin.
      assert (Hin' : In t (skipn k l)) by (rewrite Hsk; exact Hin).
      apply In_nth_error in Hin'. destruct Hin' as [i Hi].
      rewrite nth_error_skipn in Hi.
      apply (sorted_nth_desc l Hs k (k + i) p t Hpk Hi). lia.
Qed.

(** Theorem: Sampling preserves high-attention tokens: the average score of
    the top-k sample is at least the average score of the whole set, i.e.
      total(sample) * length(set) >= total(set) * k.
    (Fully proved; the original was [admit]/[Admitted].) *)
Theorem sampling_preserves_attention :
  forall tokens k,
    k > 0 ->
    k <= length tokens ->
    sorted_by_weight tokens ->
    total_attention (attention_sample tokens k) * length tokens >=
    total_attention tokens * k.
Proof.
  intros tokens k Hk Hlen Hsorted.
  unfold attention_sample, top_k.
  (* split tokens = firstn k ++ skipn k *)
  set (P := firstn k tokens).
  set (S := skipn k tokens).
  assert (Hsplit : tokens = P ++ S) by (symmetry; apply firstn_skipn).
  (* lengths *)
  assert (HlenP : length P = k).
  { unfold P. rewrite length_firstn. lia. }
  assert (HlenS : length S = length tokens - k).
  { unfold S. apply length_skipn. }
  assert (Hntot : length tokens = k + length S) by lia.
  (* total over the split *)
  assert (Htot : total_attention tokens = total_attention P + total_attention S).
  { rewrite Hsplit. clear. induction P as [|h r IH].
    - simpl. lia.
    - simpl. rewrite !total_attention_cons. rewrite IH. lia. }
  (* pivot *)
  destruct (sorted_split_pivot tokens k Hsorted) as [c [HPge HSle]].
  fold P in HPge. fold S in HSle.
  assert (HPlow : total_attention P >= length P * c) by (apply total_attention_ge; exact HPge).
  assert (HShi : total_attention S <= length S * c) by (apply total_attention_le; exact HSle).
  (* Goal: total(P) * length tokens >= total(tokens) * k
     = (total(P) + total(S)) * k.
     Reduces to total(P) * length(S) >= total(S) * k, which follows from
     total(P) >= k*c (length P = k) and total(S) <= length(S)*c. *)
  rewrite Htot, Hntot, HlenP in *.
  nia.
Qed.

(** ** Position-Aware Attention

    Combine attention weight with position importance (recency bias). *)
Definition position_boost (max_pos pos : nat) : nat :=
  (pos * 100) / (max_pos + 1).  (* 0-99 scale, higher for later positions *)

Definition combined_score (t : Token) (max_pos : nat) : nat :=
  attention_weight t + position_boost max_pos (token_position t).

(** Select by combined score (assumes already sorted by combined score). *)
Definition position_aware_top_k (tokens : list Token) (k max_pos : nat) : list Token :=
  firstn k tokens.

(** Theorem: Position-aware scoring never lowers a token's selection score
    below its raw attention weight: the recency-boosted combined score
    dominates the bare attention weight.

    [CORRECTION] The original theorem concluded [True] (a vacuous placeholder
    with no content).  The real, provable guarantee that captures its intent
    ("recent high-attention tokens are favored") is that the combined score is
    monotone above the raw attention weight - so a token never *loses* ranking
    by being scored position-aware.  Hence a recent token's combined score is
    at least its attention weight, and in particular a high-attention token
    keeps a high combined score (>= its [attention_weight] threshold). *)
Theorem position_boost_never_lowers_score :
  forall t max_pos,
    combined_score t max_pos >= attention_weight t.
Proof.
  intros t max_pos. unfold combined_score. lia.
Qed.

(** Corollary: a high-attention token (>= 500) retains a combined score that
    still clears the same high-attention bar - position scoring cannot demote
    it below threshold.  This is the substantive content the placeholder
    [True] was standing in for. *)
Theorem position_aware_keeps_high_attention :
  forall t max_pos,
    attention_weight t >= 500 ->
    combined_score t max_pos >= 500.
Proof.
  intros t max_pos Hhi.
  pose proof (position_boost_never_lowers_score t max_pos) as Hge.
  lia.
Qed.

(** ** KV Cache Compression Model

    Model for key-value cache compression in transformers. *)
Record KVEntry : Type := mkKVEntry {
  kv_layer : nat;
  kv_position : nat;
  kv_importance : nat;  (* Derived from attention patterns *)
}.

Definition KVCache := list KVEntry.

(** Evict lowest-importance entries to stay within budget (assumes sorted by
    importance descending; take the top [budget] entries). *)
Definition evict_kv (cache : KVCache) (budget : nat) : KVCache :=
  firstn budget cache.

(** Theorem: KV eviction maintains budget.

    [CORRECTION] The original proof used [firstn_length_le] (an equality
    needing [length cache <= budget]) then [lia], which does not close the
    bound for caches longer than the budget.  [firstn_le_length] gives
    [length (firstn budget cache) <= budget] directly.  Statement unchanged. *)
Theorem kv_eviction_maintains_budget :
  forall cache budget,
    length (evict_kv cache budget) <= budget.
Proof.
  intros cache budget.
  unfold evict_kv.
  apply firstn_le_length.
Qed.

(** Theorem: KV eviction never grows the cache. *)
Theorem kv_eviction_reduces_size :
  forall cache budget,
    length (evict_kv cache budget) <= length cache.
Proof.
  intros cache budget. unfold evict_kv.
  rewrite length_firstn. apply Nat.le_min_r.
Qed.

(** ** Attention Pattern Analysis *)

(** Detect "attention sinks" - tokens that always receive high attention. *)
Definition is_attention_sink (t : Token) (threshold : nat) : bool :=
  Nat.leb threshold (attention_weight t).

(** [CORRECTION] The original [attention_sinks] used the SSReflect section
    notation [is_attention_sink^~ threshold], which does not parse without
    importing ssreflect (this file imports only Stdlib).  Replaced with an
    explicit lambda of the same meaning. *)
Definition attention_sinks (tokens : list Token) (threshold : nat) : list Token :=
  filter (fun t => is_attention_sink t threshold) tokens.

(** Theorem: Attention sinks are never pruned - every sink survives the
    threshold cut at the same threshold.  (Fully proved.) *)
Theorem never_prune_sinks :
  forall tokens threshold t,
    In t (attention_sinks tokens threshold) ->
    In t (threshold_prune tokens threshold).
Proof.
  intros tokens threshold t Hin.
  unfold attention_sinks in Hin.
  unfold threshold_prune.
  apply filter_In in Hin.
  destruct Hin as [Hintoks Hissink].
  apply filter_In.
  split.
  - exact Hintoks.
  - unfold is_attention_sink in Hissink. exact Hissink.
Qed.

(** ** Sliding Window Attention *)

Definition in_attention_window (pos window_start window_size : nat) : bool :=
  Nat.leb window_start pos && Nat.ltb pos (window_start + window_size).

Definition windowed_tokens (tokens : list Token) (window_start window_size : nat) : list Token :=
  filter (fun t => in_attention_window (token_position t) window_start window_size) tokens.

(** Helper: filtering preserves the "distinct under a projection" property. *)
Lemma NoDup_map_filter :
  forall (A B : Type) (f : A -> B) (p : A -> bool) (l : list A),
    NoDup (map f l) -> NoDup (map f (filter p l)).
Proof.
  intros A B f p l. induction l as [|h r IH]; intros Hnd.
  - simpl. constructor.
  - simpl in Hnd. inversion Hnd as [|x xs Hnotin Hnd' Heq]; subst.
    simpl. destruct (p h) eqn:Hp.
    + simpl. constructor.
      * (* f h not in map f (filter p r), since not in map f r and filter shrinks *)
        intro Hc. apply Hnotin.
        apply in_map_iff in Hc. destruct Hc as [x [Hfx Hxin]].
        apply in_map_iff. exists x. split; [ exact Hfx | ].
        apply filter_In in Hxin. destruct Hxin as [Hxr _]. exact Hxr.
      * apply IH. exact Hnd'.
    + apply IH. exact Hnd'.
Qed.

(** Theorem: Windowed attention bounds the context to the window size.

    [CORRECTION] The original statement
      length (windowed_tokens tokens ws wsz) <= wsz
    is FALSE without a distinctness assumption: several tokens may share the
    same position, so an arbitrarily long list whose positions all fall inside
    one window has [length > wsz].  The intended guarantee ("a window of size
    wsz holds at most wsz tokens") holds exactly when positions are distinct.
    We add the genuine, satisfiable modeling hypothesis
      NoDup (map token_position tokens)
    (one token per position) under which the bound is true: the window spans
    [wsz] distinct positions, and distinct tokens occupy distinct positions in
    it, so at most [wsz] tokens survive. *)
Theorem window_reduces_context :
  forall tokens window_start window_size,
    NoDup (map token_position tokens) ->
    length (windowed_tokens tokens window_start window_size) <= window_size.
Proof.
  intros tokens ws wsz Hnd.
  unfold windowed_tokens.
  set (p := fun t => in_attention_window (token_position t) ws wsz).
  set (W := filter p tokens).
  (* length W = length (map token_position W) *)
  rewrite <- (length_map token_position W).
  (* map token_position W is NoDup *)
  assert (HndW : NoDup (map token_position W)) by (apply NoDup_map_filter; exact Hnd).
  (* every position in W lies in seq ws wsz *)
  assert (Hincl : incl (map token_position W) (seq ws wsz)).
  { intros x Hx.
    apply in_map_iff in Hx. destruct Hx as [t [Hpos Htin]].
    subst x. unfold W in Htin. apply filter_In in Htin.
    destruct Htin as [_ Hp]. unfold p, in_attention_window in Hp.
    apply andb_true_iff in Hp. destruct Hp as [Hge Hlt].
    apply Nat.leb_le in Hge. apply Nat.ltb_lt in Hlt.
    apply in_seq. lia. }
  (* NoDup + incl into seq ws wsz, whose length is wsz *)
  pose proof (NoDup_incl_length HndW Hincl) as Hbound.
  rewrite length_seq in Hbound.
  exact Hbound.
Qed.
