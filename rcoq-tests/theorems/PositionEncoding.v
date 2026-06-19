(** * PositionEncoding: Formal Model of Position Encoding and Length Extrapolation

    This module formalizes position encoding schemes (RoPE, ALiBi) and proves
    bounds on context-length extrapolation, plus the structural properties a
    position-encoding map must have to be usable: it is injective (distinct
    positions get distinct codes), order-preserving/monotone, relative-position
    codes are shift-invariant (RoPE-style: the code depends only on the offset
    i-j), and a uniform position shift preserves relative ordering.  We also
    formalize the "Lost in the Middle" U-shaped positional-salience model.

    Every theorem below is fully proved (no [admit]/[Admitted]).  Where the
    original statement was false as written, it has been restated to a true and
    still non-trivial theorem; each such change is documented inline with a
    [CORRECTION] note explaining why the weaker/false form does not hold and what
    modeling hypothesis (or tightened bound) makes the real statement provable.
    The rigor bar, the [CORRECTION] convention, and the [cbn]-targeted /
    [div_mod_eq + nia] patterns follow theorems/CompressionBounds.v.

    References:
    - Su et al., "RoFormer: Enhanced Transformer with Rotary Position Embedding"
      (RoPE: attention depends only on the relative offset i - j).
    - Press et al., "ALiBi: Train Short, Test Long" (linear-bias slopes per head).
    - Liu et al. 2023, "Lost in the Middle: How Language Models Use Long Contexts"
      (arXiv:2307.03172): U-shaped positional salience -- accuracy/attention is
      highest at the start and end of the context and lowest in the middle.
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Division helper

    [a * k / d <= k] when [a <= d] and [d > 0].  Used to bound the linear
    quality-degradation terms (which are floor divisions) by their limit. *)
Lemma mul_div_le_k :
  forall a k d, 0 < d -> a <= d -> a * k / d <= k.
Proof.
  intros a k d Hd Ha. apply Nat.Div0.div_le_upper_bound. nia.
Qed.

(** ** Position Encoding Schemes *)

Inductive PositionScheme : Type :=
  | Absolute    (* Learned absolute positions *)
  | RoPE        (* Rotary Position Embedding *)
  | ALiBi       (* Attention with Linear Biases *)
  | NoPE.       (* No Position Encoding *)

(** Training context length *)
Definition trained_length (scheme : PositionScheme) : nat :=
  match scheme with
  | Absolute => 512   (* Limited by learned embeddings *)
  | RoPE => 4096      (* Standard RoPE training length *)
  | ALiBi => 2048     (* ALiBi is length-agnostic but trained on this *)
  | NoPE => 0         (* N/A *)
  end.

(** ** Extrapolation Capability *)

(** How well does the model perform at position p given training length L? *)
(** Quality degrades as p exceeds L *)

Definition extrapolation_quality (scheme : PositionScheme) (train_len test_pos : nat) : nat :=
  match scheme with
  | Absolute =>
      (* Absolute fails completely beyond training length *)
      if Nat.leb test_pos train_len then 100 else 0
  | RoPE =>
      (* RoPE degrades gradually, ~50% quality at 2x *)
      if Nat.leb test_pos train_len then 100
      else 100 - min 100 ((test_pos - train_len) * 50 / max 1 train_len)
  | ALiBi =>
      (* ALiBi extrapolates well, ~80% quality at 2x *)
      if Nat.leb test_pos train_len then 100
      else 100 - min 100 ((test_pos - train_len) * 20 / max 1 train_len)
  | NoPE => 0
  end.

(** ** Extrapolation Theorems *)

(** Theorem: Absolute encoding fails beyond training length *)
Theorem absolute_fails_beyond_training :
  forall train_len test_pos,
    test_pos > train_len ->
    extrapolation_quality Absolute train_len test_pos = 0.
Proof.
  intros train_len test_pos Hgt.
  unfold extrapolation_quality.
  destruct (Nat.leb test_pos train_len) eqn:Hleb.
  - apply Nat.leb_le in Hleb. lia.
  - reflexivity.
Qed.

(** Theorem: ALiBi extrapolates better than RoPE *)
Theorem alibi_better_extrapolation :
  forall train_len test_pos,
    test_pos > train_len ->
    extrapolation_quality ALiBi train_len test_pos >=
    extrapolation_quality RoPE train_len test_pos.
Proof.
  intros train_len test_pos Hgt.
  unfold extrapolation_quality.
  destruct (Nat.leb test_pos train_len) eqn:Hleb.
  - apply Nat.leb_le in Hleb. lia.
  - (* ALiBi degrades 20% per train_len, RoPE degrades 50% *)
    (* So ALiBi quality >= RoPE quality *)
    (* 100 - min 100 ((p-L)*20/L) >= 100 - min 100 ((p-L)*50/L) *)
    assert ((test_pos - train_len) * 20 / max 1 train_len <=
            (test_pos - train_len) * 50 / max 1 train_len) as Hdeg.
    { apply Nat.Div0.div_le_mono.
      apply Nat.mul_le_mono_l. lia. }
    pose proof (Nat.le_min_r 100 ((test_pos - train_len) * 20 / max 1 train_len)).
    pose proof (Nat.le_min_r 100 ((test_pos - train_len) * 50 / max 1 train_len)).
    lia.
Qed.

(** ** RoPE Frequency Scaling *)

(** RoPE uses frequencies theta_i = 10000^(-2i/d) *)
(** Position interpolation scales these for longer contexts *)

Definition base_frequency : nat := 10000.

(** Scaled frequency for position interpolation *)
Definition scaled_frequency (original_freq scale_factor : nat) : nat :=
  original_freq * 100 / max 1 scale_factor.

(** Position after interpolation *)
Definition interpolated_position (original_pos scale_factor : nat) : nat :=
  original_pos * 100 / max 1 scale_factor.

(** Theorem: Interpolation maps longer context back into the training range.

    [CORRECTION] The original statement omitted [train_len > 0].  When
    [train_len = 0] the bound becomes [interp <= 0], i.e. [interp = 0], but
    with [target_len > 0] and [pos = target_len = 1] one gets [scale = 100]
    and [interp = 1*100/100 = 1 > 0], so the theorem is FALSE there.  A model
    always has a positive training length, so adding [train_len > 0] is a
    genuine, satisfiable modeling hypothesis.  Under it the scale factor is at
    least 100, which is exactly what forces interpolation back into the (very
    loose but real) training range bound [train_len * 100]. *)
Theorem interpolation_fits_training :
  forall pos train_len target_len,
    train_len > 0 ->
    target_len > train_len ->
    pos <= target_len ->
    let scale := (target_len * 100) / max 1 train_len in
    interpolated_position pos scale <= train_len * 100.
Proof.
  intros pos train_len target_len Htrainpos Htarget Hpos. cbv zeta.
  unfold interpolated_position.
  assert (Hmt : Nat.max 1 train_len = train_len) by lia.
  rewrite Hmt.
  set (scale := target_len * 100 / train_len).
  (* The interpolation scale is at least 100 because target_len > train_len. *)
  assert (Hscale100 : scale >= 100).
  { unfold scale. apply Nat.div_le_lower_bound; [lia | nia]. }
  assert (Hms : Nat.max 1 scale = scale) by lia.
  rewrite Hms.
  apply Nat.Div0.div_le_upper_bound.
  (* pos * 100 <= scale * (train_len * 100); close with the div_mod identity. *)
  pose proof (Nat.div_mod_eq (target_len * 100) train_len) as Hdm.
  pose proof (Nat.mod_upper_bound (target_len * 100) train_len ltac:(lia)) as Hmod.
  fold scale in Hdm.
  nia.
Qed.

(** ** ALiBi Slope Model *)

(** ALiBi adds linear bias: bias(i,j) = -m * |i - j| *)
(** where m is a head-specific slope *)

Definition alibi_bias (slope query_pos key_pos : nat) : nat :=
  slope * (max query_pos key_pos - min query_pos key_pos).

(** Optimal slope for head h of H total heads *)
Definition optimal_slope (head_idx total_heads : nat) : nat :=
  (* Slopes are 2^(-8/H), 2^(-16/H), ... *)
  (* Approximated as 100 / 2^(head_idx * 8 / total_heads) *)
  100 / max 1 (Nat.pow 2 (head_idx * 8 / max 1 total_heads)).

(** Theorem: Heads with larger index attend to longer distances (smaller slope).

    [CORRECTION] The original statement used strict [>], which is FALSE: the
    exponent [head_idx * 8 / total_heads] is a floor division, so neighbouring
    head indices can collapse to the same exponent and hence the same slope.
    Concretely [optimal_slope 0 16 = optimal_slope 1 16 = 100], so [100 > 100]
    fails.  The true ALiBi property is that slopes are *monotone non-increasing*
    in the head index: a larger head index gives a slope no larger than a
    smaller one (so it reaches at least as far).  This is still a strong,
    content-bearing ordering -- it is the geometric-decay structure of ALiBi. *)
Theorem heads_cover_distances :
  forall h1 h2 total_heads,
    h1 < h2 ->
    h2 < total_heads ->
    optimal_slope h1 total_heads >= optimal_slope h2 total_heads.
Proof.
  intros h1 h2 total_heads Hlt Hlt2.
  unfold optimal_slope.
  (* Smaller head index -> smaller exponent -> smaller divisor -> larger slope. *)
  assert (He : h1 * 8 / max 1 total_heads <= h2 * 8 / max 1 total_heads).
  { apply Nat.Div0.div_le_mono. nia. }
  assert (Hpow : Nat.pow 2 (h1 * 8 / max 1 total_heads)
                 <= Nat.pow 2 (h2 * 8 / max 1 total_heads)).
  { apply Nat.pow_le_mono_r; lia. }
  apply Nat.div_le_compat_l. split; lia.
Qed.

(** ** Position Encoding Utilization *)

(** How efficiently does the model use its context window? *)

Record ContextUtilization : Type := mkUtilization {
  window_size : nat;
  effective_attention_span : nat;  (* How far back attention actually reaches *)
  position_scheme : PositionScheme;
}.

(** Utilization ratio (effective span as a percentage of the window). *)
Definition utilization_ratio (u : ContextUtilization) : nat :=
  (effective_attention_span u * 100) / max 1 (window_size u).

(** Theorem: a longer effective attention span yields a higher utilization
    ratio, at fixed window size.

    [CORRECTION] The original [better_encoding_better_utilization] concluded
    [True] (a vacuous placeholder with an unused [window] binder that did not
    even typecheck cleanly).  The real content is the monotonicity of the
    utilization ratio in the effective span: a position scheme that lets
    attention reach further (larger [effective_attention_span], e.g. ALiBi over
    Absolute) cannot have a smaller utilization ratio for the same window. *)
Theorem better_encoding_better_utilization :
  forall u1 u2,
    window_size u1 = window_size u2 ->
    effective_attention_span u1 <= effective_attention_span u2 ->
    utilization_ratio u1 <= utilization_ratio u2.
Proof.
  intros u1 u2 Hwin Hspan. unfold utilization_ratio.
  rewrite Hwin.
  apply Nat.Div0.div_le_mono.
  apply Nat.mul_le_mono_r. exact Hspan.
Qed.

(** ** RoPE Relative-Position Encoding (shift invariance)

    The defining property of RoPE is that the attention interaction between
    query position [i] and key position [j] depends only on the relative offset
    [i - j], not on the absolute positions.  We model the relative code as the
    offset magnitude |i - j| and prove the shift invariance directly. *)

Definition rope_relative (i j : nat) : nat :=
  Nat.max i j - Nat.min i j.

(** Theorem (RoPE shift invariance): translating both query and key positions
    by the same amount [s] leaves the relative code unchanged.  This is the
    core relative-position property that lets RoPE extrapolate. *)
Theorem rope_shift_invariant :
  forall i j s, rope_relative (i + s) (j + s) = rope_relative i j.
Proof.
  intros i j s. unfold rope_relative. lia.
Qed.

(** The relative code is symmetric in its two arguments (|i-j| = |j-i|). *)
Theorem rope_relative_sym :
  forall i j, rope_relative i j = rope_relative j i.
Proof.
  intros i j. unfold rope_relative. lia.
Qed.

(** Distinct query positions (at or beyond a fixed key) give distinct relative
    codes: the offset map is injective in the query.  Equal codes force equal
    positions. *)
Theorem rope_relative_injective_offset :
  forall i1 i2 j,
    j <= i1 -> j <= i2 ->
    rope_relative i1 j = rope_relative i2 j ->
    i1 = i2.
Proof.
  intros i1 i2 j H1 H2 He. unfold rope_relative in He. lia.
Qed.

(** ** Absolute Position Encoding (injective + order preserving)

    An absolute code must distinguish positions (injective) and respect their
    order (monotone).  A stride-scaled code [stride * pos] with [stride > 0]
    is the minimal model with both properties. *)

Definition abs_code (stride pos : nat) : nat := stride * pos.

(** Theorem: distinct positions get distinct codes (injectivity). *)
Theorem abs_code_injective :
  forall stride p1 p2,
    stride > 0 ->
    abs_code stride p1 = abs_code stride p2 ->
    p1 = p2.
Proof.
  intros stride p1 p2 Hs He. unfold abs_code in He. nia.
Qed.

(** Theorem: the code is strictly monotone (strictly order-preserving). *)
Theorem abs_code_monotone :
  forall stride p1 p2,
    stride > 0 ->
    p1 < p2 ->
    abs_code stride p1 < abs_code stride p2.
Proof.
  intros stride p1 p2 Hs Hlt. unfold abs_code. nia.
Qed.

(** Theorem: the code is order-preserving in the non-strict sense as well. *)
Theorem abs_code_order_preserving :
  forall stride p1 p2,
    p1 <= p2 ->
    abs_code stride p1 <= abs_code stride p2.
Proof.
  intros stride p1 p2 Hle. unfold abs_code. nia.
Qed.

(** ** Uniform Shift Preserves Relative Ordering

    A position shift (adding a constant offset to every position, as happens
    when a prefix is prepended) preserves the relative order of positions, both
    at the raw-position level and after a strictly-monotone absolute encoding. *)

(** Theorem: shifting all positions by [s] preserves pairwise ordering. *)
Theorem shift_preserves_order :
  forall p1 p2 s, p1 < p2 <-> p1 + s < p2 + s.
Proof.
  intros p1 p2 s. lia.
Qed.

(** Theorem: the encoded order is preserved under a uniform shift, for any
    positive stride.  (The biconditional shows the shift neither creates nor
    destroys an ordering between the two codes.) *)
Theorem shift_preserves_code_order :
  forall stride p1 p2 s,
    stride > 0 ->
    (abs_code stride p1 < abs_code stride p2 <->
     abs_code stride (p1 + s) < abs_code stride (p2 + s)).
Proof.
  intros stride p1 p2 s Hs. unfold abs_code. nia.
Qed.

(** ** "Lost in the Middle": U-shaped positional salience (Liu et al. 2023)

    In a context spanning token positions [0 .. n], the model uses information
    near the two ends (positions [0] and [n]) far better than information in the
    middle; accuracy/attention as a function of position is U-shaped.  We model
    salience as decreasing with the distance to the *nearer* edge, so a position
    close to either edge scores high and the center scores lowest. *)

(** Distance from [pos] to the nearer edge of the interval [0 .. n]. *)
Definition edge_distance (pos n : nat) : nat := Nat.min pos (n - pos).

(** U-shaped salience: higher = more salient.  Subtracting the edge distance
    from [n] makes the two edges maximal and the center minimal. *)
Definition salience (pos n : nat) : nat := n - edge_distance pos n.

(** The left edge is maximally salient. *)
Theorem salience_left_edge_max :
  forall n, salience 0 n = n.
Proof.
  intros n. unfold salience, edge_distance. lia.
Qed.

(** The right edge is maximally salient. *)
Theorem salience_right_edge_max :
  forall n, salience n n = n.
Proof.
  intros n. unfold salience, edge_distance. lia.
Qed.

(** Theorem (U-shape): every interior position is no more salient than either
    edge.  This is the precise "edges dominate the middle" statement. *)
Theorem salience_edges_dominate_middle :
  forall pos n,
    pos <= n ->
    salience pos n <= salience 0 n /\ salience pos n <= salience n n.
Proof.
  intros pos n Hpos. unfold salience, edge_distance.
  pose proof (Nat.le_min_l pos (n - pos)).
  pose proof (Nat.le_min_r pos (n - pos)).
  split; lia.
Qed.

(** Theorem: salience is monotone toward the edges -- a position at least as
    close to its nearer edge is at least as salient. *)
Theorem salience_monotone_toward_edge :
  forall p1 p2 n,
    edge_distance p1 n <= edge_distance p2 n ->
    salience p1 n >= salience p2 n.
Proof.
  intros p1 p2 n H. unfold salience.
  assert (edge_distance p2 n <= n).
  { unfold edge_distance. pose proof (Nat.le_min_l p2 (n - p2)). lia. }
  lia.
Qed.

(** Theorem (strict U-dip): in a nontrivial context of even length [2k] the
    exact center [k] is strictly less salient than the edge.  This rules out a
    flat salience profile -- the U genuinely dips. *)
Theorem salience_center_strict_dip :
  forall k, k > 0 -> salience k (2 * k) < salience 0 (2 * k).
Proof.
  intros k Hk. unfold salience, edge_distance.
  replace (2 * k - k) with k by lia.
  rewrite Nat.min_id.
  replace (2 * k - 0) with (2 * k) by lia.
  rewrite Nat.min_0_l.
  lia.
Qed.

(** ** Position Encoding for Conversation *)

(** Conversations have special structure: turn boundaries, tool outputs, etc. *)

Record ConversationPosition : Type := mkConvPos {
  absolute_position : nat;    (* Token position in sequence *)
  turn_number : nat;          (* Which conversational turn *)
  within_turn_position : nat; (* Position within turn *)
  is_system : bool;           (* Is this system/assistant text? *)
}.

(** Relative importance based on conversation structure *)
Definition conversation_importance (p : ConversationPosition) : nat :=
  let turn_recency := 100 - min 100 (turn_number p * 10) in
  let system_boost := if is_system p then 20 else 0 in
  turn_recency + system_boost.

(** Theorem: Recent turns more important than old turns *)
Theorem recent_turns_important :
  forall p1 p2,
    turn_number p1 < turn_number p2 ->
    is_system p1 = is_system p2 ->
    conversation_importance p1 >= conversation_importance p2.
Proof.
  intros p1 p2 Hturn Hsys.
  unfold conversation_importance.
  rewrite Hsys.
  assert (100 - min 100 (turn_number p1 * 10) >=
          100 - min 100 (turn_number p2 * 10)) as Hrec.
  { (* Larger turn_number -> larger min -> smaller result *)
    assert (turn_number p1 * 10 <= turn_number p2 * 10) as Hmul.
    { apply Nat.mul_le_mono_r. lia. }
    pose proof (Nat.le_min_r 100 (turn_number p1 * 10)).
    pose proof (Nat.le_min_r 100 (turn_number p2 * 10)).
    lia. }
  lia.
Qed.

(** ** Hierarchical Position Encoding *)

(** For very long contexts, use hierarchical positions:
    (block_id, position_within_block) *)

Record HierarchicalPosition : Type := mkHierPos {
  block_id : nat;
  block_position : nat;
  block_size : nat;
}.

(** Flatten to absolute position *)
Definition flatten_position (h : HierarchicalPosition) : nat :=
  block_id h * block_size h + block_position h.

(** Theorem: Hierarchical positions are unique.

    [CORRECTION] The original statement lacked the within-block bound
    [block_position < block_size], so it was FALSE: with a common block size 64,
    [(id=0, pos=64)] and [(id=1, pos=0)] both flatten to 64 yet have different
    block ids.  Uniqueness of a mixed-radix / Euclidean-division decomposition
    requires each digit to be below the radix, i.e. [block_position h < block_size h]
    for both positions.  This is the well-formedness invariant of a hierarchical
    position and is exactly satisfiable; under it the decomposition is unique. *)
Theorem hierarchical_unique :
  forall h1 h2,
    block_size h1 = block_size h2 ->
    block_size h1 > 0 ->
    block_position h1 < block_size h1 ->
    block_position h2 < block_size h2 ->
    flatten_position h1 = flatten_position h2 ->
    block_id h1 = block_id h2 /\ block_position h1 = block_position h2.
Proof.
  intros h1 h2 Hsize Hpos Hwf1 Hwf2 Hflat.
  unfold flatten_position in Hflat.
  rewrite <- Hsize in *.
  set (s := block_size h1) in *.
  set (i1 := block_id h1) in *. set (i2 := block_id h2) in *.
  set (p1 := block_position h1) in *. set (p2 := block_position h2) in *.
  (* i1*s + p1 = i2*s + p2, with p1<s, p2<s, s>0, forces i1=i2 and p1=p2. *)
  assert (i1 = i2) by nia.
  split; nia.
Qed.

(** Benefit: Block-local attention is O(block_size^2) instead of O(seq_len^2) *)
Definition hierarchical_attention_cost (seq_len block_size : nat) : nat :=
  let n_blocks := seq_len / max 1 block_size in
  let local_cost := block_size * block_size in
  let cross_block_cost := n_blocks * n_blocks in
  n_blocks * local_cost + cross_block_cost.

(** Theorem: Hierarchical attention is strictly cheaper for long sequences.
    (Fully proved; the original was [admit].)

    For [seq_len >= 4 * block_size >= 256], the block-local cost
    [n_blocks * block_size^2 = (n_blocks * block_size) * block_size
    <= seq_len * block_size <= seq_len^2 / 4], and the cross-block cost
    [n_blocks^2 <= (seq_len / 64)^2] is tiny, so the total is below [seq_len^2]. *)
Theorem hierarchical_more_efficient :
  forall seq_len block_size,
    block_size >= 64 ->
    seq_len >= block_size * 4 ->
    hierarchical_attention_cost seq_len block_size < seq_len * seq_len.
Proof.
  intros seq_len block_size Hblock Hseq.
  unfold hierarchical_attention_cost. cbv zeta.
  assert (Hmb : Nat.max 1 block_size = block_size) by lia.
  rewrite Hmb.
  set (n := seq_len / block_size).
  assert (Hnb : block_size * n <= seq_len).
  { unfold n. apply Nat.Div0.mul_div_le. }
  (* block_size >= 64, so n * 64 <= block_size * n <= seq_len. *)
  assert (Hn64 : n * 64 <= seq_len) by nia.
  nia.
Qed.

(** ** Application to JFC *)

(** JFC should be aware of model position-encoding limits *)

(** Safe context length: the largest position at which quality stays above the
    threshold.

    [CORRECTION] The original ALiBi entry used [train_len * 5], which is FALSE
    against the degradation model: at [pos = 5 * 2048 = 10240] the ALiBi quality
    is [100 - (10240-2048)*20/2048 = 100 - 80 = 20 < 50].  The honest safe length
    for ALiBi under this model is [train_len * 2] (quality 80 there).  RoPE keeps
    its [train_len * 2] (quality exactly 50 at the boundary), and Absolute keeps
    [train_len] (quality 100).  NoPE has no position encoding, so a "safe context
    length" is meaningless and is excluded by hypothesis in the theorem below. *)
Definition jfc_safe_context_length (scheme : PositionScheme) (quality_threshold : nat) : nat :=
  let train_len := trained_length scheme in
  match scheme with
  | Absolute => train_len
  | RoPE => train_len * 2  (* ~50% quality at 2x *)
  | ALiBi => train_len * 2 (* ~80% quality at 2x *)
  | NoPE => 0
  end.

(** Theorem: within the safe context length, extrapolation quality stays at or
    above the threshold.

    [CORRECTION] Two issues with the original.  (1) With [quality_threshold >= 50]
    the conclusion fails at the RoPE boundary, where quality is exactly 50: the
    real, satisfiable constraint is [quality_threshold <= 50] (you may demand up
    to 50% quality).  (2) NoPE has quality identically 0, so no positive
    threshold is ever met; it has no position encoding and is excluded by the
    [scheme <> NoPE] hypothesis.  Under these honest hypotheses the guarantee is
    true and non-trivial: it certifies, per scheme, that the chosen safe length
    really does keep quality above the demanded threshold. *)
Theorem jfc_respects_safe_length :
  forall scheme quality_threshold pos,
    scheme <> NoPE ->
    quality_threshold <= 50 ->
    pos <= jfc_safe_context_length scheme quality_threshold ->
    extrapolation_quality scheme (trained_length scheme) pos >= quality_threshold.
Proof.
  intros scheme quality_threshold pos Hnope Hthresh Hsafe.
  destruct scheme;
    unfold jfc_safe_context_length, extrapolation_quality, trained_length in *.
  - (* Absolute: pos <= 512 => quality 100 *)
    destruct (Nat.leb pos 512) eqn:E; [lia |].
    apply Nat.leb_gt in E. lia.
  - (* RoPE: pos <= 8192 => degradation term <= 50 => quality >= 50 *)
    destruct (Nat.leb pos 4096) eqn:E; [lia |].
    apply Nat.leb_gt in E.
    assert (H : (pos - 4096) * 50 / max 1 4096 <= 50) by (apply mul_div_le_k; lia).
    pose proof (Nat.le_min_r 100 ((pos - 4096) * 50 / max 1 4096)). lia.
  - (* ALiBi: pos <= 4096 => degradation term <= 20 => quality >= 80 >= threshold *)
    destruct (Nat.leb pos 2048) eqn:E; [lia |].
    apply Nat.leb_gt in E.
    assert (H : (pos - 2048) * 20 / max 1 2048 <= 20) by (apply mul_div_le_k; lia).
    pose proof (Nat.le_min_r 100 ((pos - 2048) * 20 / max 1 2048)). lia.
  - (* NoPE: excluded by hypothesis *)
    exfalso. apply Hnope. reflexivity.
Qed.
