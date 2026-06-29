(** * TokenEstimation: Formal Model of Token Count Estimation

    This module formalizes JFC's token estimation algorithm from
    crates/jfc-engine/src/compact/mod.rs. We prove bounds on the
    estimation error and characterize when the overhead multiplier
    is accurate.

    Key insight from code analysis:
    - CHARS_PER_TOKEN = 4 (rough rule for English)
    - OVERHEAD_MULTIPLIER = 3/2 = 1.5x (accounts for wire format)
    - Empirical measurement: API reports ~1.4-1.5x naive estimate

    Every theorem below is fully proved (no [admit]/[Admitted]).  Where the
    original statement was false as written (an exact equality where integer
    division only gives a one-sided/within-one bound, or an unconditional
    inequality that fails for large inputs), it has been restated to a true
    *and* still non-trivial theorem.  Each such change carries an inline
    [CORRECTION] note explaining the false-vs-true delta.

    References:
    - crates/jfc-engine/src/compact/mod.rs:72 (estimate_tokens)
    - crates/jfc-core/src/paging.rs:51 (estimate_tokens)
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Constants from JFC Code *)

Definition CHARS_PER_TOKEN : nat := 4.
Definition OVERHEAD_MULTIPLIER_NUM : nat := 3.
Definition OVERHEAD_MULTIPLIER_DEN : nat := 2.  (* 3/2 = 1.5x *)

(** ** Base Estimation (without overhead) *)

Definition base_estimate (char_count : nat) : nat :=
  char_count / CHARS_PER_TOKEN.

(** ** Full Estimation (with overhead) *)

Definition estimate_tokens (char_count : nat) : nat :=
  (base_estimate char_count * OVERHEAD_MULTIPLIER_NUM) / OVERHEAD_MULTIPLIER_DEN.

(** Equivalent to: char_count / 4 * 3 / 2 = char_count * 3 / 8 *)

(** ** Estimation Error Model *)

(** Actual token count vs estimated *)
Record TokenEstimate : Type := mkEstimate {
  char_count : nat;
  estimated_tokens : nat;
  actual_tokens : nat;
}.

(** Error as percentage of actual *)
Definition error_pct (e : TokenEstimate) : nat :=
  let diff := max (estimated_tokens e) (actual_tokens e) -
              min (estimated_tokens e) (actual_tokens e) in
  (diff * 100) / max 1 (actual_tokens e).

(** ** Estimation Theorems *)

(** Theorem: Empty input gives zero tokens.  Estimation never invents
    tokens out of nothing. *)
Theorem empty_is_zero :
  estimate_tokens 0 = 0.
Proof.
  unfold estimate_tokens, base_estimate.
  simpl. reflexivity.
Qed.

(** Theorem: Estimation is monotonic in char count.

    [CORRECTION] (proof only, statement unchanged) The original proof applied
    the deprecated [Nat.div_le_mono], whose real signature is
    [c <> 0 -> a <= b -> a/c <= b/c]; the leftover [OVERHEAD_MULTIPLIER_DEN <> 0]
    side goal could not be discharged by [lia] (it does not unfold the
    definition), so the file failed to compile.  We use the unconditional
    [Nat.Div0.div_le_mono : a <= b -> a/c <= b/c] instead. *)
Theorem estimation_monotonic :
  forall c1 c2,
    c1 <= c2 ->
    estimate_tokens c1 <= estimate_tokens c2.
Proof.
  intros c1 c2 Hle.
  unfold estimate_tokens, base_estimate.
  apply Nat.Div0.div_le_mono.
  apply Nat.mul_le_mono_r.
  apply Nat.Div0.div_le_mono.
  exact Hle.
Qed.

(** Theorem: Estimation never overestimates the naive 1.5x-of-base count.

    [CORRECTION] The original [overhead_is_150_percent] claimed the exact
    equality
      c >= CHARS_PER_TOKEN -> estimate_tokens c * 2 = base_estimate c * 3.
    This is FALSE: take c = 4, so base_estimate c = 1, [estimate_tokens 4 =
    (1*3)/2 = 1], and [1*2 = 2 <> 3 = 1*3].  Integer (floor) division loses
    the remainder [base*3 mod 2], so equality holds only when [base*3] is
    even.  The true, still-strong characterization is the two-sided within-one
    bound below, which pins [estimate_tokens c] to exactly [floor(base*3/2)]:
    it is no more than 1.5x of base, and no more than one token short of it.
    The [c >= CHARS_PER_TOKEN] hypothesis is unnecessary and dropped (the
    bound holds for every [c], including the empty input). *)
Theorem overhead_within_one :
  forall c,
    estimate_tokens c * OVERHEAD_MULTIPLIER_DEN <= base_estimate c * OVERHEAD_MULTIPLIER_NUM
    /\ base_estimate c * OVERHEAD_MULTIPLIER_NUM
         <= estimate_tokens c * OVERHEAD_MULTIPLIER_DEN + (OVERHEAD_MULTIPLIER_DEN - 1).
Proof.
  intros c.
  unfold estimate_tokens, OVERHEAD_MULTIPLIER_DEN, OVERHEAD_MULTIPLIER_NUM.
  set (b := base_estimate c).
  pose proof (Nat.div_mod_eq (b * 3) 2) as Hdm.
  pose proof (Nat.mod_upper_bound (b * 3) 2 ltac:(lia)) as Hmod.
  lia.
Qed.

(** Corollary: the estimate never *exceeds* the exact 1.5x naive count,
    i.e. estimation never overcounts relative to the rational target. *)
Theorem overhead_no_overcount :
  forall c,
    estimate_tokens c * OVERHEAD_MULTIPLIER_DEN <= base_estimate c * OVERHEAD_MULTIPLIER_NUM.
Proof.
  intros c. apply (overhead_within_one c).
Qed.

(** ** Concatenation / Additivity *)

(** Theorem: base estimation is super-additive over concatenated inputs.

    Floor division satisfies (a+b)/c >= a/c + b/c, so estimating two chunks
    jointly never *undercounts* relative to estimating them separately and
    adding: joining text can only recover fractional characters that separate
    estimates rounded away.  This is the discrete sub-additivity-of-rounding
    fact that justifies estimating a whole conversation at once. *)
Theorem base_estimate_superadditive :
  forall a b,
    base_estimate (a + b) >= base_estimate a + base_estimate b.
Proof.
  intros a b. unfold base_estimate, CHARS_PER_TOKEN.
  apply Nat.div_le_lower_bound; [ lia | ].
  pose proof (Nat.div_mod_eq a 4) as Ha.
  pose proof (Nat.div_mod_eq b 4) as Hb.
  pose proof (Nat.mod_upper_bound a 4 ltac:(lia)) as Hma.
  pose proof (Nat.mod_upper_bound b 4 ltac:(lia)) as Hmb.
  nia.
Qed.

(** ** Language-Specific Token Density *)

(** Different languages have different chars-per-token ratios *)
Inductive Language : Type :=
  | English
  | Chinese
  | Japanese
  | Code.

Definition chars_per_token (lang : Language) : nat :=
  match lang with
  | English => 4    (* ~4 chars per token *)
  | Chinese => 2    (* ~2 chars per token - more dense *)
  | Japanese => 2   (* Similar to Chinese *)
  | Code => 3       (* Variable names, syntax *)
  end.

Definition language_adjusted_estimate (char_count : nat) (lang : Language) : nat :=
  let base := char_count / chars_per_token lang in
  (base * OVERHEAD_MULTIPLIER_NUM) / OVERHEAD_MULTIPLIER_DEN.

(** Theorem: Chinese text uses at least as many tokens per character as
    English (denser script => fewer chars per token => more tokens). *)
Theorem chinese_more_dense :
  forall char_count,
    char_count >= 4 ->
    language_adjusted_estimate char_count Chinese >=
    language_adjusted_estimate char_count English.
Proof.
  intros char_count Hc.
  unfold language_adjusted_estimate, chars_per_token.
  (* Chinese: char_count / 2, English: char_count / 4 *)
  apply Nat.Div0.div_le_mono.
  apply Nat.mul_le_mono_r.
  (* char_count / 4 <= char_count / 2 *)
  apply Nat.div_le_compat_l.
  lia.
Qed.

(** ** Overhead Components *)

(** What contributes to the 1.5x overhead? *)
Record OverheadComponents : Type := mkOverhead {
  json_framing : nat;       (* {"role":"...", "content":"..."} *)
  tool_definitions : nat;   (* Tool schema overhead *)
  system_prompt : nat;      (* Hidden in API wire format *)
  role_markers : nat;       (* User/Assistant role tokens *)
}.

Definition total_overhead (o : OverheadComponents) : nat :=
  json_framing o + tool_definitions o + system_prompt o + role_markers o.

(** Typical overhead for a tool-heavy conversation *)
Definition typical_tool_conversation_overhead : OverheadComponents :=
  mkOverhead 50 200 100 20.  (* Per message; total = 370 *)

(** Theorem: Tool-heavy conversations need a higher multiplier — when the base
    payload is small relative to the fixed per-message overhead.

    [CORRECTION] The original [tools_increase_overhead] claimed
      (base_tokens + overhead) * 100 / max 1 base_tokens >= 150
    for ALL [base_tokens].  This is FALSE for large base: as base grows the
    ratio tends to 100, e.g. base = 1000 gives (1370*100)/1000 = 137 < 150.
    The fixed 370-token overhead only dominates when the base is small.  The
    true, satisfiable form adds the modeling hypothesis [base_tokens <= 2 *
    overhead] (overhead is at least half the base), which is exactly the
    regime where total cost is >= 1.5x the base; under it the bound holds. *)
Theorem tools_increase_overhead :
  forall base_tokens,
    let overhead := total_overhead typical_tool_conversation_overhead in
    base_tokens <= 2 * overhead ->
    (base_tokens + overhead) * 100 / max 1 base_tokens >= 150.
Proof.
  intros base_tokens.
  unfold total_overhead, typical_tool_conversation_overhead. cbn [json_framing
    tool_definitions system_prompt role_markers].
  intros Hb.
  destruct base_tokens as [|b].
  - (* base = 0: (0+370)*100/1 = 37000 >= 150 *)
    assert (Hm : max 1 0 = 1) by lia. rewrite Hm.
    apply Nat.div_le_lower_bound; lia.
  - (* base = S b > 0 *)
    assert (Hm : max 1 (S b) = S b) by lia. rewrite Hm.
    apply Nat.div_le_lower_bound; lia.
Qed.

(** ** PageStore Token Estimation *)

(** From paging.rs: simpler estimate without overhead multiplier *)
Definition paging_estimate (s_len : nat) : nat :=
  (s_len + 3) / 4.  (* Ceiling division *)

(** Theorem: Paging estimate is ceiling of chars/4 *)
Theorem paging_is_ceiling :
  forall s_len,
    paging_estimate s_len = (s_len + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN.
Proof.
  intros s_len.
  unfold paging_estimate, CHARS_PER_TOKEN.
  (* RHS (s_len + 4 - 1)/4 reduces to (s_len + 3)/4 *)
  f_equal. lia.
Qed.

(** Theorem: Paging (ceiling) estimate is at least the floor estimate, and
    never overshoots it by more than one token.  Ceiling rounding never
    *undercounts* the true chars/4 token count. *)
Theorem paging_ge_floor :
  forall s_len,
    paging_estimate s_len >= s_len / CHARS_PER_TOKEN.
Proof.
  intros s_len.
  unfold paging_estimate, CHARS_PER_TOKEN.
  apply Nat.Div0.div_le_mono.
  lia.
Qed.

(** Theorem: ceiling estimate exceeds the floor estimate by at most one. *)
Theorem paging_le_floor_plus_one :
  forall s_len,
    paging_estimate s_len <= s_len / CHARS_PER_TOKEN + 1.
Proof.
  intros s_len.
  unfold paging_estimate, CHARS_PER_TOKEN.
  (* relate quotients of (s_len + 3) and s_len directly via their div_mod forms *)
  pose proof (Nat.div_mod_eq (s_len + 3) 4) as Hc.
  pose proof (Nat.div_mod_eq s_len 4) as Hf.
  pose proof (Nat.mod_upper_bound (s_len + 3) 4 ltac:(lia)) as Mc.
  pose proof (Nat.mod_upper_bound s_len 4 ltac:(lia)) as Mf.
  nia.
Qed.

(** Theorem: ceiling estimate is exact on multiples of CHARS_PER_TOKEN. *)
Theorem paging_exact_on_multiples :
  forall k,
    paging_estimate (k * CHARS_PER_TOKEN) = k.
Proof.
  intros k. unfold paging_estimate, CHARS_PER_TOKEN.
  (* (4k + 3)/4 = k + 3/4 = k *)
  replace (k * 4 + 3) with (k * 4 + 3) by lia.
  rewrite Nat.div_add_l by lia.
  rewrite (Nat.div_small 3 4) by lia.
  lia.
Qed.

(** ** Estimation Accuracy Analysis *)

(** Under what conditions is the estimate accurate? *)

(** Accuracy: 100 - relative error (%), clamped into [0, 100]. *)
Definition accuracy (estimated actual : nat) : nat :=
  let diff := max estimated actual - min estimated actual in
  100 - min 100 ((diff * 100) / max 1 actual).

(** Theorem: when the estimate equals the actual count, accuracy is exactly
    100% (zero relative error). *)
Theorem english_prose_accurate :
  forall char_count actual,
    actual = estimate_tokens char_count ->
    accuracy (estimate_tokens char_count) actual = 100.
Proof.
  intros char_count actual Heq.
  unfold accuracy.
  rewrite Heq.
  rewrite Nat.max_id.
  rewrite Nat.min_id.
  (* diff = 0, so 0*100/max 1 actual = 0, min 100 0 = 0, 100 - 0 = 100 *)
  rewrite Nat.sub_diag.
  rewrite Nat.mul_0_l. rewrite Nat.Div0.div_0_l. rewrite Nat.min_0_r. lia.
Qed.

(** Theorem: accuracy is always a valid percentage (never exceeds 100). *)
Theorem accuracy_bounded :
  forall estimated actual,
    accuracy estimated actual <= 100.
Proof.
  intros estimated actual. unfold accuracy. lia.
Qed.

(** ** Novel: Adaptive Estimation *)

(** Generic fold_left accumulator-shift lemma, used to peel the accumulator
    out of the sum-of-actuals fold so we can reason by [cons] equations. *)
Lemma fold_left_add_shift :
  forall (A : Type) (f : A -> nat) (l : list A) (z : nat),
    fold_left (fun acc x => acc + f x) l z
      = z + fold_left (fun acc x => acc + f x) l 0.
Proof.
  intros A f l. induction l as [|x xs IH]; intros z.
  - simpl. lia.
  - simpl. rewrite (IH (z + f x)). rewrite (IH (f x)). lia.
Qed.

(** Track estimation error and adjust multiplier *)
Record EstimationHistory : Type := mkHistory {
  samples : list (nat * nat);  (* (estimated, actual) pairs *)
  current_multiplier_num : nat;
  current_multiplier_den : nat;
}.

(** Compute optimal multiplier from samples: numerator is the total observed
    actual tokens, denominator the total predicted-base tokens (made positive
    so the learned ratio is always applicable). *)
Definition optimal_multiplier (h : EstimationHistory) : (nat * nat) :=
  let sum_actual := fold_left (fun acc p => acc + snd p) (samples h) 0 in
  let sum_base := fold_left (fun acc p =>
    acc + (fst p * current_multiplier_den h / max 1 (current_multiplier_num h)))
    (samples h) 0 in
  (* optimal = sum_actual / sum_base *)
  (sum_actual, max 1 sum_base).

(** Total observed actual tokens across a sample set. *)
Definition total_actual (s : list (nat * nat)) : nat :=
  fold_left (fun acc p => acc + snd p) s 0.

Lemma total_actual_cons :
  forall p s, total_actual (p :: s) = snd p + total_actual s.
Proof.
  intros p s. unfold total_actual. simpl.
  rewrite (fold_left_add_shift (nat * nat) (fun q => snd q) s (snd p)). lia.
Qed.

(** Theorem: the learned multiplier is always well-defined — its denominator
    is positive, so applying it can never divide by zero. *)
Theorem adaptive_denominator_positive :
  forall h,
    snd (optimal_multiplier h) >= 1.
Proof.
  intros h. unfold optimal_multiplier. cbn [snd]. apply Nat.le_max_l.
Qed.

(** Theorem: the learned numerator is exactly the empirical total of observed
    actual token counts — the optimizer is grounded in the real data, not a
    free parameter. *)
Theorem adaptive_numerator_is_evidence :
  forall h,
    fst (optimal_multiplier h) = total_actual (samples h).
Proof.
  intros h. unfold optimal_multiplier, total_actual. cbn [fst]. reflexivity.
Qed.

(** Theorem: Adaptive multiplier accumulates evidence monotonically — adding
    a sample never decreases the learned numerator (the empirical actual-token
    total).  This is the honest "more data only sharpens the estimate"
    content; the original [adaptive_converges] concluded a vacuous [True]
    under an unused [length >= 10] hypothesis, which carried no information.

    [CORRECTION] Original conclusion was [True] (a placeholder).  Replaced with
    the real monotone-accumulation guarantee over the sample history. *)
Theorem adaptive_converges :
  forall p h,
    fst (optimal_multiplier (mkHistory (p :: samples h)
                                       (current_multiplier_num h)
                                       (current_multiplier_den h)))
      >= fst (optimal_multiplier h).
Proof.
  intros p h.
  rewrite !adaptive_numerator_is_evidence. cbn [samples].
  rewrite total_actual_cons. lia.
Qed.

(** ** JFC-Specific Estimation *)

(** JFC's estimate_tokens from compact/mod.rs *)
Definition jfc_estimate_tokens (messages : list nat) : nat :=
  (* Sum of per-message estimates with overhead *)
  let base := fold_left Nat.add messages 0 / CHARS_PER_TOKEN in
  base * OVERHEAD_MULTIPLIER_NUM / OVERHEAD_MULTIPLIER_DEN.

(** Theorem: JFC estimation matches expected formula *)
Theorem jfc_matches_formula :
  forall char_counts,
    jfc_estimate_tokens char_counts =
    estimate_tokens (fold_left Nat.add char_counts 0).
Proof.
  intros char_counts.
  unfold jfc_estimate_tokens, estimate_tokens, base_estimate.
  reflexivity.
Qed.

(** ** Pressure Thresholds *)

(** From paging.rs: Pressure levels based on window utilization *)
Inductive Pressure : Type :=
  | Ok      (* Below warning threshold *)
  | Warn    (* At/above warn_frac *)
  | Flush.  (* At/above flush_frac *)

Definition compute_pressure (used window warn_frac flush_frac : nat) : Pressure :=
  (* frac = used * 100 / window *)
  let frac := used * 100 / max 1 window in
  if Nat.leb flush_frac frac then Flush
  else if Nat.leb warn_frac frac then Warn
  else Ok.

(** Theorem: Pressure is monotonic in used tokens — once enough usage triggers
    a Flush, no smaller drop in usage can un-trigger it. *)
Theorem pressure_monotonic :
  forall u1 u2 window warn flush,
    u1 <= u2 ->
    compute_pressure u1 window warn flush = Flush ->
    compute_pressure u2 window warn flush = Flush.
Proof.
  intros u1 u2 window warn flush Hle Hp1.
  unfold compute_pressure in *.
  destruct (Nat.leb flush (u1 * 100 / max 1 window)) eqn:Hf1.
  - (* u1 >= flush threshold *)
    assert (u2 * 100 / max 1 window >= u1 * 100 / max 1 window) as Hge.
    { apply Nat.Div0.div_le_mono. apply Nat.mul_le_mono_r. exact Hle. }
    destruct (Nat.leb flush (u2 * 100 / max 1 window)) eqn:Hf2.
    + reflexivity.
    + apply Nat.leb_le in Hf1. apply Nat.leb_nle in Hf2. lia.
  - (* u1 below flush threshold: Hp1 reduces to an inner if that is Warn/Ok,
       never Flush, so the hypothesis is contradictory regardless of warn. *)
    destruct (Nat.leb warn (u1 * 100 / max 1 window)); discriminate.
Qed.
