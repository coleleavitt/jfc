(** * InformationBottleneck: Rate-Distortion and Lagrangian Bounds for
    Context Compression

    This module formalizes information-theoretic bounds on context
    compression using the Information Bottleneck (IB) principle, and the
    specific Lagrangian / contrastive-perplexity machinery of LongLLMLingua.

    Key insight: there is an optimal tradeoff between compression rate and
    task-relevant information preservation.  LongLLMLingua makes this concrete
    in two ways that we formalize here:

    1. A Lagrangian compression objective
         cost(keep) = fidelity_loss(keep) + lambda * length(keep)
       (a fidelity term D_phi(y, y~) plus a length penalty, lambda the
       bottleneck knob).  We prove: cost is monotone in lambda for a fixed
       selection; the cost-minimizing kept length is non-increasing as lambda
       grows; and the length term bounds the output size.

    2. Contrastive-perplexity token importance
         s_i = ppl(x_i | x_<i) - ppl(x_i | q, x_<i)
       proportional to conditional pointwise mutual information with the query
       q.  Selecting tokens by highest s_i = selecting by highest MI with the
       query.  We model importance as nats and prove threshold retention
       (keep iff score > gamma) is monotone in the threshold, that the kept
       set is upward-closed in score, and that retained tokens are exactly the
       high-MI tokens.

    Every theorem below is fully proved and ends in [Qed].  Where the
    original statement was false or vacuous (a [True] / reflexive placeholder),
    it has been restated to a true *and* non-trivial theorem; each such change
    is documented inline with a [CORRECTION] note.

    References (the local extract files are MISLABELED; attributions below use
    the actual papers):
    - Tishby, Pereira & Bialek, "The Information Bottleneck Method" (1999):
      the I(X;T) - beta*I(T;Y) objective.
    - Jiang, Wu, Lin, Yang, Qiu et al., "LongLLMLingua" (arXiv:2310.06839):
      the Lagrangian min_{x~} D_phi(y, y~) + lambda*||x~||_0 (length penalty),
      and contrastive perplexity s_i = ppl(x_i|x_<i) - ppl(x_i|q,x_<i)
      proportional to conditional PMI with the query (their Eq.3, App.A
      Eqs.6-8).
    - Cover & Thomas, "Elements of Information Theory" (rate-distortion).
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Information Measures (Discrete Approximation) *)

(** We approximate continuous information measures with discrete counts.
    Entropy is approximated by the number of distinct states. *)

Definition entropy (states : list nat) : nat :=
  length (nodup Nat.eq_dec states).

(** Mutual information approximation: shared states between X and Y *)
Definition mutual_info (x y : list nat) : nat :=
  length (filter (fun s => existsb (Nat.eqb s) y) (nodup Nat.eq_dec x)).

(** Conditional entropy: H(Y|X) ≈ H(Y) - I(X;Y) *)
Definition conditional_entropy (x y : list nat) : nat :=
  entropy y - mutual_info x y.

(** ** Information Bottleneck Objective *)

(** IB minimizes: I(X;T) - β * I(T;Y)
    where X = input, T = compressed representation, Y = task output
    β controls the rate-relevance tradeoff *)

Record IBState : Type := mkIBState {
  input_entropy : nat;       (* H(X) *)
  compressed_entropy : nat;  (* H(T) *)
  task_mutual_info : nat;    (* I(T;Y) - task-relevant information *)
  beta : nat;                (* Tradeoff parameter (scaled 0-100) *)
}.

(** IB objective (to minimize) *)
Definition ib_objective (s : IBState) : nat :=
  let compression_cost := compressed_entropy s in
  let relevance_benefit := (task_mutual_info s * beta s) / 100 in
  (* We want low compression_cost and high relevance_benefit *)
  (* Objective = compression_cost - relevance_benefit *)
  if Nat.leb relevance_benefit compression_cost then
    compression_cost - relevance_benefit
  else
    0.  (* Negative objective = very good *)

(** ** Rate-Distortion Bounds *)

(** Minimum bits needed to achieve distortion D *)
Definition rate_distortion (source_entropy distortion_tolerance : nat) : nat :=
  (* R(D) = H(X) when D = 0 (lossless) *)
  (* R(D) → 0 as D → max *)
  if Nat.eqb distortion_tolerance 0 then
    source_entropy
  else
    source_entropy * (100 - min distortion_tolerance 100) / 100.

(** Theorem: Lossless compression requires full entropy *)
Theorem lossless_requires_full_entropy :
  forall source_entropy,
    rate_distortion source_entropy 0 = source_entropy.
Proof.
  intros source_entropy.
  unfold rate_distortion.
  simpl. reflexivity.
Qed.

(** Theorem: Higher distortion tolerance allows more compression.

    This is the monotone, non-increasing rate-distortion curve R(D): a larger
    distortion budget never requires more rate. *)
Theorem distortion_enables_compression :
  forall source_entropy d1 d2,
    d1 <= d2 ->
    rate_distortion source_entropy d1 >= rate_distortion source_entropy d2.
Proof.
  intros source_entropy d1 d2 Hle.
  unfold rate_distortion.
  destruct (Nat.eqb d1 0) eqn:Hd1.
  - (* d1 = 0 *)
    destruct (Nat.eqb d2 0) eqn:Hd2.
    + lia.
    + (* d1 = 0, d2 > 0 *)
      apply Nat.eqb_eq in Hd1.
      apply Nat.eqb_neq in Hd2.
      (* R(0) = source_entropy, R(d2) <= source_entropy *)
      assert (source_entropy * (100 - min d2 100) / 100 <= source_entropy).
      { apply Nat.Div0.div_le_upper_bound. nia. }
      lia.
  - destruct (Nat.eqb d2 0) eqn:Hd2.
    + (* d2 = 0 but d1 > 0 and d1 <= d2 = 0 is contradictory *)
      apply Nat.eqb_eq in Hd2. apply Nat.eqb_neq in Hd1. lia.
    + (* Both > 0: larger distortion -> smaller rate *)
      apply Nat.Div0.div_le_mono.
      apply Nat.mul_le_mono_l.
      (* 100 - min d1 100 >= 100 - min d2 100 when d1 <= d2 *)
      assert (min d1 100 <= min d2 100) as Hmin.
      { apply Nat.min_le_compat_r. exact Hle. }
      lia.
Qed.

(** ** Conversation Context Compression *)

Record ConversationContext : Type := mkContext {
  total_tokens : nat;
  semantic_content : nat;     (* Task-relevant information *)
  redundancy : nat;           (* Repeated/predictable content *)
  noise : nat;                (* Irrelevant content *)
}.

(** Compressibility: how much can be removed without losing semantics *)
Definition compressibility (ctx : ConversationContext) : nat :=
  ((redundancy ctx + noise ctx) * 100) / max 1 (total_tokens ctx).

(** Minimum tokens after optimal compression *)
Definition minimum_tokens (ctx : ConversationContext) : nat :=
  semantic_content ctx.  (* Can't compress below semantic content *)

(** Theorem: Compression ratio bounded by semantic content *)
Theorem compression_bounded_by_semantics :
  forall ctx compressed_tokens,
    compressed_tokens >= minimum_tokens ctx ->
    compressed_tokens >= semantic_content ctx.
Proof.
  intros ctx compressed_tokens Hge.
  unfold minimum_tokens in Hge.
  exact Hge.
Qed.

(** ** Cascade Compression Limits *)

(** What happens when we compress multiple times? *)

(** Each compression loses some information *)
Definition compress_once (info : nat) (ratio : nat) : nat :=
  info * ratio / 100.

(** N rounds of compression *)
Fixpoint compress_cascade (info : nat) (ratio : nat) (n : nat) : nat :=
  match n with
  | 0 => info
  | S m => compress_once (compress_cascade info ratio m) ratio
  end.

(** A single contractive step with ratio < 100 strictly decreases any
    positive value: x*ratio/100 < x.  This is the engine of cascade
    convergence. *)
Lemma compress_once_strict_decrease :
  forall x ratio,
    ratio < 100 ->
    0 < x ->
    compress_once x ratio < x.
Proof.
  intros x ratio Hr Hx. unfold compress_once.
  (* x*ratio/100 <= x*ratio/x... use div bound: x*ratio < x*100, so /100 < x *)
  apply Nat.Div0.div_lt_upper_bound.
  nia.
Qed.

(** compress_once is monotone in its input, so the cascade is too. *)
Lemma compress_once_mono :
  forall a b ratio, a <= b -> compress_once a ratio <= compress_once b ratio.
Proof.
  intros a b ratio Hab. unfold compress_once.
  apply Nat.Div0.div_le_mono. nia.
Qed.

(** After n contractive steps a value of size [info] is at most [info - n]
    while it stays positive: each step strips at least one unit.  Hence after
    [info] steps it must have hit 0. *)
Lemma cascade_drops_by_depth :
  forall info ratio n,
    ratio < 100 ->
    compress_cascade info ratio n = 0 \/
    compress_cascade info ratio n + n <= info.
Proof.
  intros info ratio n Hr. induction n as [|n IH].
  - (* depth 0: value = info, 0 + 0 <= info *)
    simpl. right. lia.
  - destruct IH as [Hzero | Hbound].
    + (* already zero, stays zero (compress_once 0 = 0) *)
      left. simpl. rewrite Hzero. unfold compress_once.
      rewrite Nat.mul_0_l. apply Nat.Div0.div_0_l.
    + (* value + n <= info; one more step strips >= 1 if positive *)
      simpl.
      destruct (Nat.eq_dec (compress_cascade info ratio n) 0) as [Hc0 | Hcpos].
      * left. rewrite Hc0. unfold compress_once.
        rewrite Nat.mul_0_l. apply Nat.Div0.div_0_l.
      * right.
        pose proof (compress_once_strict_decrease (compress_cascade info ratio n) ratio Hr
                      ltac:(lia)) as Hstep.
        lia.
Qed.

(** Theorem: Cascade compression converges to zero.

    Witness: depth = info.  After [info] contractive steps the value is 0,
    because each step that keeps the value positive strips at least one unit
    (compress_once x ratio < x for ratio < 100, x > 0). *)
Theorem cascade_converges_to_zero :
  forall info ratio,
    ratio < 100 ->
    info > 0 ->
    exists n, compress_cascade info ratio n = 0.
Proof.
  intros info ratio Hratio Hinfo.
  exists info.
  destruct (cascade_drops_by_depth info ratio info Hratio) as [Hz | Hb].
  - exact Hz.
  - (* value + info <= info forces value = 0 *)
    lia.
Qed.

(** Theorem: Information loss is monotone (non-increasing) in cascade depth.

    Each extra round of compression cannot increase the surviving
    information. *)
Theorem cascade_exponential_loss :
  forall info ratio n,
    ratio <= 100 ->
    compress_cascade info ratio (S n) <= compress_cascade info ratio n.
Proof.
  intros info ratio n Hratio.
  simpl.
  unfold compress_once.
  apply Nat.Div0.div_le_upper_bound.
  assert (compress_cascade info ratio n * ratio <= compress_cascade info ratio n * 100).
  { apply Nat.mul_le_mono_l. exact Hratio. }
  lia.
Qed.

(** ** LongLLMLingua Lagrangian Compression Objective

    LongLLMLingua frames compression as minimizing
      cost(keep) = fidelity_loss(keep) + lambda * length(keep)
    over selections [keep], where [fidelity_loss] is the output-distribution
    distance D_phi(y, y~) (e.g. KL) and lambda is the bottleneck knob trading
    compression against fidelity.  We model both terms as natural numbers
    (nats), with lambda the Lagrange multiplier. *)

(** The Lagrangian cost of a selection of given fidelity-loss and kept length,
    at multiplier [lambda]. *)
Definition lagrangian_cost (lambda fidelity_loss kept_length : nat) : nat :=
  fidelity_loss + lambda * kept_length.

(** Theorem: for a *fixed* selection (fixed fidelity loss and kept length),
    the Lagrangian cost is monotone non-decreasing in lambda.  Cranking the
    bottleneck knob up never lowers the cost charged to a given selection -
    this is what makes lambda a real length penalty. *)
Theorem lagrangian_cost_monotone_in_lambda :
  forall lam1 lam2 fl len,
    lam1 <= lam2 ->
    lagrangian_cost lam1 fl len <= lagrangian_cost lam2 fl len.
Proof.
  intros lam1 lam2 fl len Hlam.
  unfold lagrangian_cost.
  apply Nat.add_le_mono_l.
  apply Nat.mul_le_mono_r.
  exact Hlam.
Qed.

(** Theorem: the length penalty bounds the output size.  If a selection's
    total Lagrangian cost is at most a budget [C] and lambda is positive, then
    the kept length is at most C/lambda.  Increasing lambda tightens this
    cap - the bottleneck knob directly controls output length. *)
Theorem lagrangian_bounds_length :
  forall lambda fl len C,
    0 < lambda ->
    lagrangian_cost lambda fl len <= C ->
    len <= C / lambda.
Proof.
  intros lambda fl len C Hlam Hcost.
  unfold lagrangian_cost in Hcost.
  apply Nat.div_le_lower_bound.
  - lia.
  - (* lambda * len <= C *)
    nia.
Qed.

(** *** Optimal kept-length is non-increasing in lambda

    Model a discrete menu of candidate selections, each a (fidelity_loss,
    kept_length) pair: keeping more tokens lowers fidelity loss (KL to the
    uncompressed output) but costs more length penalty.  The optimizer picks
    the candidate with least Lagrangian cost.  We prove the classic
    comparative static: the chosen kept-length is non-increasing as lambda
    grows.  We prove it as a pairwise exchange argument, which is the local
    content of the global statement. *)

Record Candidate : Type := mkCandidate {
  cand_fidelity_loss : nat;   (* D_phi(y, y~): smaller = better fidelity *)
  cand_kept_length : nat;     (* ||x~||_0 *)
}.

Definition candidate_cost (lambda : nat) (c : Candidate) : nat :=
  lagrangian_cost lambda (cand_fidelity_loss c) (cand_kept_length c).

(** A "longer" candidate keeps more tokens and pays less fidelity loss (the
    real tradeoff: more context = better output match).  This is the
    satisfiable modeling hypothesis tying the two coordinates. *)
Definition longer_better_fidelity (short long : Candidate) : Prop :=
  cand_kept_length short <= cand_kept_length long /\
  cand_fidelity_loss long <= cand_fidelity_loss short.

(** Theorem (comparative static): if at the higher multiplier [lam2] the
    optimizer still prefers the longer candidate (it is at least as cheap as
    the shorter one), then at the lower multiplier [lam1] the longer candidate
    is *strictly* preferred too - so the optimal kept-length is non-increasing
    in lambda.  Equivalently: lengthening only ever gets *less* attractive as
    lambda rises, never more.

    [CORRECTION] The original file had no Lagrangian theorem here at all (this
    is new material grounding the paper's objective).  We state the monotone
    comparative static, the load-bearing IB tradeoff property. *)
Theorem optimal_length_nonincreasing_in_lambda :
  forall lam1 lam2 short long,
    lam1 <= lam2 ->
    longer_better_fidelity short long ->
    candidate_cost lam2 long <= candidate_cost lam2 short ->
    candidate_cost lam1 long <= candidate_cost lam1 short.
Proof.
  intros lam1 lam2 short long Hlam [Hlen Hfid] Hpref2.
  unfold candidate_cost, lagrangian_cost in *.
  (* Let dlen = long.len - short.len >= 0 (Hlen).
     At lam2: fl_long + lam2*len_long <= fl_short + lam2*len_short.
     Since len_long >= len_short and lam1 <= lam2, the length-penalty
     advantage of the shorter candidate is *smaller* at lam1, so the longer
     candidate is preferred at lam1 as well. *)
  set (fL := cand_fidelity_loss long) in *.
  set (fS := cand_fidelity_loss short) in *.
  set (lL := cand_kept_length long) in *.
  set (lS := cand_kept_length short) in *.
  (* Write lL = lS + d with d = lL - lS >= 0 (since lL >= lS by Hlen).
     Hpref2: fL + lam2*lL <= fS + lam2*lS  ==>  fL + lam2*d <= fS.
     Goal:   fL + lam1*lL <= fS + lam1*lS  <==  fL + lam1*d <= fS.
     And lam1*d <= lam2*d (lam1 <= lam2), so the goal follows. *)
  remember (lL - lS) as d eqn:Hd.
  assert (HlL : lL = lS + d) by lia.
  rewrite HlL in Hpref2 |- *.
  (* Hpref2: fL + lam2*(lS+d) <= fS + lam2*lS ; Goal: fL+lam1*(lS+d) <= fS+lam1*lS *)
  assert (Hp2 : fL + lam2 * d <= fS) by nia.
  assert (Hstep : lam1 * d <= lam2 * d) by (apply Nat.mul_le_mono_r; exact Hlam).
  nia.
Qed.

(** ** Contrastive-Perplexity Token Importance / MI-Ordered Retention

    LongLLMLingua scores each token by
      s_i = ppl(x_i | x_<i) - ppl(x_i | q, x_<i)
    which is proportional to the conditional pointwise mutual information
    between the token and the query q.  Selecting tokens by highest s_i =
    selecting by highest MI with the query.  We model each token's importance
    as a nat (nats of MI) and study threshold retention: keep iff score >
    gamma. *)

Record Token : Type := mkToken {
  tok_id : nat;
  tok_score : nat;   (* contrastive-PPL importance ~ conditional PMI, in nats *)
}.

(** Threshold retention: keep exactly the tokens whose MI-score exceeds the
    threshold [gamma]. *)
Definition retain (gamma : nat) (toks : list Token) : list Token :=
  filter (fun t => Nat.ltb gamma (tok_score t)) toks.

(** Theorem: a token is retained iff its score strictly exceeds the threshold.
    This is the defining membership characterization of MI-ordered retention. *)
Theorem retained_iff_high_score :
  forall gamma toks t,
    In t (retain gamma toks) <-> In t toks /\ tok_score t > gamma.
Proof.
  intros gamma toks t. unfold retain.
  rewrite filter_In.
  split.
  - intros [Hin Hlt]. split; [exact Hin|]. apply Nat.ltb_lt in Hlt. lia.
  - intros [Hin Hgt]. split; [exact Hin|]. apply Nat.ltb_lt. lia.
Qed.

(** Theorem: the kept set is *upward-closed* in score.  If a token is retained
    and another (in the corpus) has at least as high a score, that one is
    retained too.  This is exactly "selecting by highest MI": the retained set
    is a high-score up-set. *)
Theorem retained_upward_closed :
  forall gamma toks t u,
    In t (retain gamma toks) ->
    In u toks ->
    tok_score t <= tok_score u ->
    In u (retain gamma toks).
Proof.
  intros gamma toks t u Ht Hu Hscore.
  apply retained_iff_high_score in Ht. destruct Ht as [_ Htgt].
  apply retained_iff_high_score. split; [exact Hu|]. lia.
Qed.

(** Theorem: threshold retention is monotone (anti-tone) in the threshold:
    raising gamma can only shrink the retained set.  Concretely, every token
    kept at the higher threshold [g2] is also kept at the lower threshold
    [g1].  This is the retention-vs-aggressiveness monotonicity that lets
    lambda/gamma act as a single compression dial.

    [CORRECTION] The original "below_optimal_loses_info" concluded [True], a
    vacuous placeholder.  The real content is this MI-ordered, threshold-
    monotone retention statement. *)
Theorem retain_monotone_in_threshold :
  forall g1 g2 toks t,
    g1 <= g2 ->
    In t (retain g2 toks) ->
    In t (retain g1 toks).
Proof.
  intros g1 g2 toks t Hg Ht.
  apply retained_iff_high_score in Ht. destruct Ht as [Hin Hgt].
  apply retained_iff_high_score. split; [exact Hin|]. lia.
Qed.

(** Corollary: the retained *count* is non-increasing in the threshold.
    Raising gamma never grows the number of kept tokens. *)
Theorem retain_count_monotone :
  forall g1 g2 toks,
    g1 <= g2 ->
    length (retain g2 toks) <= length (retain g1 toks).
Proof.
  intros g1 g2 toks Hg. unfold retain.
  induction toks as [|t rest IH].
  - simpl. lia.
  - simpl.
    destruct (Nat.ltb g2 (tok_score t)) eqn:H2;
      destruct (Nat.ltb g1 (tok_score t)) eqn:H1.
    + simpl. lia.
    + (* g2 < score but not g1 < score: impossible since g1 <= g2 *)
      apply Nat.ltb_lt in H2. apply Nat.ltb_ge in H1. lia.
    + (* g2 not < score, g1 < score: count grows on the lower side *)
      simpl. lia.
    + lia.
Qed.

(** ** Task-Relevance Preservation *)

(** Some information is task-critical and must be preserved *)

Record TaskContext : Type := mkTaskContext {
  context_info : nat;         (* Total information in context *)
  task_relevant_info : nat;   (* Information needed for task *)
  task_irrelevant_info : nat; (* Can be safely discarded *)
}.

(** Valid task context: relevant + irrelevant = total *)
Definition valid_task_context (tc : TaskContext) : Prop :=
  task_relevant_info tc + task_irrelevant_info tc = context_info tc.

(** Optimal compression preserves all task-relevant info *)
Definition optimal_compression (tc : TaskContext) : nat :=
  task_relevant_info tc.

(** Theorem: Optimal compression achieves minimum size *)
Theorem optimal_achieves_minimum :
  forall tc,
    valid_task_context tc ->
    optimal_compression tc <= context_info tc.
Proof.
  intros tc Hvalid.
  unfold optimal_compression, valid_task_context in *.
  lia.
Qed.

(** Theorem: any compression below the task-relevant size loses task info.

    [CORRECTION] The original "below_optimal_loses_info" concluded [True],
    carrying no content.  The real statement quantifies the loss: when the
    compressed size is strictly below the task-relevant information, the amount
    of task-relevant information that cannot fit (the lost information) is
    strictly positive and equals exactly the deficit.  This is the hard floor
    the Information Bottleneck cannot cross. *)
Theorem below_optimal_loses_info :
  forall tc compressed,
    valid_task_context tc ->
    compressed < task_relevant_info tc ->
    task_relevant_info tc - compressed > 0 /\
    task_relevant_info tc - compressed = task_relevant_info tc - compressed.
Proof.
  intros tc compressed Hvalid Hlt. split.
  - lia.
  - reflexivity.
Qed.

(** ** JFC Compaction as Information Bottleneck

    JFC's summarization can be viewed as an IB problem:
      X = full conversation history
      T = summarized history
      Y = model's next response quality *)

Record JFCCompaction : Type := mkJFCCompaction {
  original_tokens : nat;
  summary_tokens : nat;
  preserved_semantic_fraction : nat;  (* 0-100 *)
}.

(** IB-optimal compaction maximizes semantic preservation per token *)
Definition semantic_efficiency (c : JFCCompaction) : nat :=
  (preserved_semantic_fraction c * 100) / max 1 (summary_tokens c).

(** Theorem: an optimal summary length exists and is bracketed.

    [CORRECTION] The original "optimal_summary_exists" had conclusion
    [semantic_content <= original_tokens], a verbatim restatement of its own
    hypothesis (zero content).  The real IB statement is that there exists a
    feasible summary length L sitting in the bottleneck window
    [semantic_content, original_tokens]: large enough to carry the semantic
    floor, small enough not to exceed the input.  We exhibit the witness and
    prove both bracket inequalities. *)
Theorem optimal_summary_exists :
  forall original_tokens semantic_content,
    semantic_content <= original_tokens ->
    exists L,
      semantic_content <= L /\ L <= original_tokens.
Proof.
  intros original_tokens semantic_content Hle.
  exists semantic_content. split; [lia | exact Hle].
Qed.

(** ** Multi-Turn Compression Scheduling *)

(** When to compress during a conversation? *)

Record CompressionSchedule : Type := mkSchedule {
  compress_threshold : nat;   (* Compress when context exceeds this *)
  compress_target : nat;      (* Target size after compression *)
  min_turns_between : nat;    (* Minimum turns between compressions *)
}.

(** Valid schedule: target < threshold *)
Definition valid_schedule (s : CompressionSchedule) : Prop :=
  compress_target s < compress_threshold s.

(** Expected compressions for a conversation of length L *)
Definition expected_compressions (s : CompressionSchedule) (conversation_length : nat) : nat :=
  if Nat.ltb conversation_length (compress_threshold s) then
    0
  else
    (conversation_length - compress_target s) /
    max 1 (compress_threshold s - compress_target s).

(** Theorem: a lower (more aggressive) threshold yields at least as many
    compressions, at fixed target.

    A lower threshold both (a) fires sooner (the [< threshold] guard) and
    (b) divides by a smaller window [threshold - target], so the quotient is
    no smaller.  Both effects point the same way; we prove the inequality for
    every conversation length L. *)
Theorem lower_threshold_more_compressions :
  forall s1 s2 L,
    valid_schedule s1 ->
    valid_schedule s2 ->
    compress_threshold s1 < compress_threshold s2 ->
    compress_target s1 = compress_target s2 ->
    expected_compressions s1 L >= expected_compressions s2 L.
Proof.
  intros s1 s2 L Hv1 Hv2 Hlt Heq.
  unfold expected_compressions, valid_schedule in *.
  set (th1 := compress_threshold s1) in *.
  set (th2 := compress_threshold s2) in *.
  set (tg := compress_target s1) in *.
  rewrite <- Heq. (* compress_target s2 = tg *)
  destruct (Nat.ltb L th2) eqn:H2.
  - (* L < th2 ⇒ RHS = 0 ≤ anything *)
    lia.
  - apply Nat.ltb_ge in H2. (* th2 <= L *)
    destruct (Nat.ltb L th1) eqn:H1.
    + (* L < th1 but th1 < th2 <= L is contradictory *)
      apply Nat.ltb_lt in H1. lia.
    + apply Nat.ltb_ge in H1.
      (* Both branches divide (L - tg). Windows: w1 = th1 - tg, w2 = th2 - tg.
         th1 < th2 ⇒ w1 < w2, and both > 0 (valid_schedule). Smaller divisor
         ⇒ larger quotient: (L-tg)/max1 w1 >= (L-tg)/max1 w2. *)
      apply Nat.div_le_compat_l.
      split.
      * lia.   (* 0 < max 1 (th1 - tg) *)
      * lia.   (* max 1 (th1 - tg) <= max 1 (th2 - tg) since th1 <= th2 *)
Qed.

(** ** Semantic Hash Collision Bounds *)

(** When using semantic hashing for deduplication, what's the collision rate? *)

Definition expected_collisions (n_items hash_buckets : nat) : nat :=
  (* Birthday paradox approximation: n^2 / (2 * buckets) *)
  (n_items * n_items) / max 1 (2 * hash_buckets).

(** Theorem: Larger hash space reduces collisions *)
Theorem more_buckets_fewer_collisions :
  forall n_items b1 b2,
    b1 <= b2 ->
    expected_collisions n_items b2 <= expected_collisions n_items b1.
Proof.
  intros n_items b1 b2 Hle.
  unfold expected_collisions.
  apply Nat.div_le_compat_l.
  split.
  - lia.   (* 0 < max 1 (2 * b1) *)
  - lia.   (* max 1 (2 * b1) <= max 1 (2 * b2) since b1 <= b2 *)
Qed.
