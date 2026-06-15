(*
  Rocq/Coq formalization of the algorithmic claims behind PAIR-style
  controlled red-team evaluators and theory-grounded variants.

  Scope note:
  - This file proves query bounds and algebraic consequences directly.
  - Deep external results (ergodic Markov-chain convergence, UCB1,
    GP-UCB, Robbins-Monro stochastic approximation, Sinkhorn convergence,
    and HJB verification) are represented as explicit hypotheses before
    deriving the concrete theorem shape used by the implementation.
  - No harmful prompt template, attack content, or operational procedure is
    encoded here. Prompts are abstract elements of a type.
*)

From Stdlib Require Import Arith Classical Lia List Reals Lra.

Import ListNotations.
Open Scope R_scope.

(* -------------------------------------------------------------------------- *)
(* Common finite sums over lists of real numbers.                              *)
(* -------------------------------------------------------------------------- *)

Definition rsum (xs : list R) : R :=
  fold_right Rplus 0 xs.

Lemma rsum_ext :
  forall {A : Type} (f g : A -> R) (xs : list A),
    (forall x, In x xs -> f x = g x) ->
    rsum (map f xs) = rsum (map g xs).
Proof.
  intros A f g xs Hext.
  induction xs as [| x xs IH]; simpl; [reflexivity |].
  assert (Hx : f x = g x) by (apply Hext; left; reflexivity).
  rewrite Hx.
  f_equal.
  apply IH. intros y Hy. apply Hext. right. exact Hy.
Qed.

Lemma rsum_const_mul :
  forall {A : Type} (c : R) (f : A -> R) (xs : list A),
    rsum (map (fun x => c * f x) xs) = c * rsum (map f xs).
Proof.
  intros A c f xs.
  induction xs as [| x xs IH]; simpl; lra.
Qed.

(* -------------------------------------------------------------------------- *)
(* Baseline PAIR / TAP / AutoDAN / Crescendo / GOAT / RoboPAIR / ProAct.       *)
(* -------------------------------------------------------------------------- *)

Module Baseline.

Definition pair_bound (streams iterations : nat) : nat := streams * iterations.
Definition tap_bound (width depth : nat) : nat := width * depth.
Definition full_tree_bound (width branch depth : nat) : nat := width * branch ^ depth.
Definition autodan_bound (population generations : nat) : nat :=
  population * generations.
Definition multiturn_bound (turns : nat) : nat := turns.
Definition robopair_bound (iterations : nat) : nat := iterations.

Theorem pair_query_bound :
  forall streams iterations queries : nat,
    (queries <= streams * iterations)%nat ->
    (queries <= pair_bound streams iterations)%nat.
Proof.
  intros. unfold pair_bound. exact H.
Qed.

Theorem tap_pruned_query_bound :
  forall width depth queries : nat,
    (queries <= width * depth)%nat ->
    (queries <= tap_bound width depth)%nat.
Proof.
  intros. unfold tap_bound. exact H.
Qed.

Theorem tap_tree_query_bound :
  forall width branch depth queries : nat,
    (queries <= width * branch ^ depth)%nat ->
    (queries <= full_tree_bound width branch depth)%nat.
Proof.
  intros. unfold full_tree_bound. exact H.
Qed.

Theorem tap_pruning_never_increases_queries :
  forall generated kept : nat,
    (kept <= generated)%nat ->
    (kept <= generated)%nat.
Proof.
  auto.
Qed.

Theorem autodan_query_bound :
  forall population generations queries : nat,
    (queries <= population * generations)%nat ->
    (queries <= autodan_bound population generations)%nat.
Proof.
  intros. unfold autodan_bound. exact H.
Qed.

Theorem multiturn_query_bound :
  forall turns queries : nat,
    (queries <= turns)%nat ->
    (queries <= multiturn_bound turns)%nat.
Proof.
  intros. unfold multiturn_bound. exact H.
Qed.

Theorem robopair_query_bound :
  forall iterations queries : nat,
    (queries <= iterations)%nat ->
    (queries <= robopair_bound iterations)%nat.
Proof.
  intros. unfold robopair_bound. exact H.
Qed.

Theorem proact_wrapped_tap_bound :
  forall width depth queries : nat,
    (queries <= tap_bound width depth)%nat ->
    (queries <= tap_bound width depth)%nat.
Proof.
  auto.
Qed.

Section ElitistGeneticSearch.
  Variable best : nat -> R.

  Hypothesis elitism :
    forall g, best g <= best (S g).

  Theorem autodan_best_fitness_monotone :
    forall g n, best g <= best (g + n)%nat.
  Proof.
    intros g n.
    induction n as [| n IH].
    - replace (g + 0)%nat with g by lia. lra.
    - replace (g + S n)%nat with (S (g + n))%nat by lia.
      eapply Rle_trans; [exact IH | apply elitism].
  Qed.
End ElitistGeneticSearch.

End Baseline.

(* -------------------------------------------------------------------------- *)
(* PAIR-Markov: finite-state detailed balance implies stationarity.            *)
(* -------------------------------------------------------------------------- *)

Module PairMarkov.

Section FiniteChain.
  Variable Prompt : Type.
  Variable enum : list Prompt.
  Variable pi : Prompt -> R.
  Variable transition : Prompt -> Prompt -> R.

  Definition stationary : Prop :=
    forall y,
      pi y = rsum (map (fun x => pi x * transition x y) enum).

  Definition detailed_balance : Prop :=
    forall x y, pi x * transition x y = pi y * transition y x.

  Hypothesis row_stochastic :
    forall x, rsum (map (fun y => transition x y) enum) = 1.

  Theorem detailed_balance_implies_stationary :
    detailed_balance -> stationary.
  Proof.
    intros Hdb y.
    unfold stationary.
    replace (pi y) with (pi y * 1) by lra.
    rewrite <- (row_stochastic y).
    rewrite <- rsum_const_mul.
    apply rsum_ext.
    intros x _.
    symmetry. apply Hdb.
  Qed.
End FiniteChain.

Section ConvergenceWrapper.
  Variable Prompt : Type.
  Variable chain_law : nat -> Prompt -> R.
  Variable stationary_distribution : Prompt -> R.

  Definition distributional_convergence : Prop :=
    forall prompt eps,
      eps > 0 ->
      exists n0,
        forall n, (n >= n0)%nat ->
          Rabs (chain_law n prompt - stationary_distribution prompt) < eps.

  Hypothesis ergodic_theorem_applies : distributional_convergence.

  Theorem pair_markov_converges_under_ergodicity :
    distributional_convergence.
  Proof.
    exact ergodic_theorem_applies.
  Qed.
End ConvergenceWrapper.

End PairMarkov.

(* -------------------------------------------------------------------------- *)
(* TAP-UCB: regret theorem wrapper and average-regret algebra.                 *)
(* -------------------------------------------------------------------------- *)

Module TapUcb.

Definition average_regret (regret horizon : R) : R := regret / horizon.

Theorem regret_bound_implies_average_bound :
  forall regret bound horizon,
    0 < horizon ->
    regret <= bound ->
    average_regret regret horizon <= bound / horizon.
Proof.
  intros regret bound horizon Hh Hr.
  unfold average_regret, Rdiv.
  apply Rmult_le_compat_r; [left; apply Rinv_0_lt_compat; exact Hh | exact Hr].
Qed.

Section Ucb1Wrapper.
  Variable expected_regret rhs : nat -> R.

  Hypothesis ucb1_regret_bound :
    forall t, expected_regret t <= rhs t.

  Theorem tap_ucb_regret_bound :
    forall t, expected_regret t <= rhs t.
  Proof.
    exact ucb1_regret_bound.
  Qed.
End Ucb1Wrapper.

End TapUcb.

(* -------------------------------------------------------------------------- *)
(* BOJA / GP-UCB: sample-efficiency theorem wrapper and average-regret bound.  *)
(* -------------------------------------------------------------------------- *)

Module Boja.

Definition average_regret (regret horizon : R) : R := regret / horizon.
Definition boja_bound (iterations : nat) : nat := iterations.
Definition boja_proposal_bound (iterations candidates : nat) : nat :=
  iterations * candidates.

Theorem boja_query_bound :
  forall iterations queries : nat,
    (queries <= iterations)%nat ->
    (queries <= boja_bound iterations)%nat.
Proof.
  intros. unfold boja_bound. exact H.
Qed.

Theorem boja_proposal_query_bound :
  forall iterations candidates proposals : nat,
    (proposals <= iterations * candidates)%nat ->
    (proposals <= boja_proposal_bound iterations candidates)%nat.
Proof.
  intros. unfold boja_proposal_bound. exact H.
Qed.

Section GpUcbWrapper.
  Variable cumulative_regret information_bound : nat -> R.

  Hypothesis gp_ucb_regret_bound :
    forall t, cumulative_regret t <= information_bound t.

  Theorem boja_cumulative_regret_bound :
    forall t, cumulative_regret t <= information_bound t.
  Proof.
    exact gp_ucb_regret_bound.
  Qed.

  Theorem boja_average_regret_bound :
    forall t,
      (0 < INR t)%R ->
      average_regret (cumulative_regret t) (INR t)
        <= information_bound t / INR t.
  Proof.
    intros t Ht.
    unfold average_regret, Rdiv.
    apply Rmult_le_compat_r.
    - left. apply Rinv_0_lt_compat. exact Ht.
    - apply gp_ucb_regret_bound.
  Qed.
End GpUcbWrapper.

End Boja.

(* -------------------------------------------------------------------------- *)
(* CASP: random-walk hitting-time algebra.                                     *)
(* -------------------------------------------------------------------------- *)

Module Casp.

Definition casp_bound (turns : nat) : nat := turns.

Theorem casp_query_bound :
  forall turns queries : nat,
    (queries <= turns)%nat ->
    (queries <= casp_bound turns)%nat.
Proof.
  intros. unfold casp_bound. exact H.
Qed.

Theorem hitting_time_algebra :
  forall h0 drift tau : R,
    drift <> 0 ->
    1 - tau * drift = h0 ->
    tau = (1 - h0) / drift.
Proof.
  intros h0 drift tau Hdrift Hmart.
  unfold Rdiv.
  apply Rmult_eq_reg_r with (r := drift); [| exact Hdrift].
  rewrite Rmult_assoc.
  rewrite Rinv_l; lra.
Qed.

Theorem positive_drift_positive_hitting_time :
  forall h0 drift tau : R,
    0 <= h0 < 1 ->
    0 < drift ->
    tau = (1 - h0) / drift ->
    0 < tau.
Proof.
  intros h0 drift tau Hh Hdrift Htau.
  subst tau.
  unfold Rdiv.
  apply Rmult_lt_0_compat; [lra | apply Rinv_0_lt_compat; exact Hdrift].
Qed.

End Casp.

(* -------------------------------------------------------------------------- *)
(* Composed budgets for hybrid evaluators.                                     *)
(* -------------------------------------------------------------------------- *)

Module Hybrid.

Definition sequential_bound (first second : nat) : nat := first + second.

Theorem sequential_budget_bound :
  forall first second total : nat,
    (total <= first + second)%nat ->
    (total <= sequential_bound first second)%nat.
Proof.
  intros. unfold sequential_bound. exact H.
Qed.

Theorem hybrid_boja_casp_total_budget :
  forall bo_iters max_turns total : nat,
    (total <= bo_iters + max_turns)%nat ->
    (total <= Boja.boja_bound bo_iters + Casp.casp_bound max_turns)%nat.
Proof.
  intros bo_iters max_turns total Htotal.
  unfold Boja.boja_bound, Casp.casp_bound.
  exact Htotal.
Qed.

Theorem hybrid_boja_casp_proposal_budget :
  forall bo_iters candidates max_turns total : nat,
    (total <= bo_iters * candidates + max_turns)%nat ->
    (total <= Boja.boja_proposal_bound bo_iters candidates
              + Casp.casp_bound max_turns)%nat.
Proof.
  intros bo_iters candidates max_turns total Htotal.
  unfold Boja.boja_proposal_bound, Casp.casp_bound.
  exact Htotal.
Qed.

End Hybrid.

(* -------------------------------------------------------------------------- *)
(* J-RL: policy-gradient convergence theorem wrapper.                          *)
(* -------------------------------------------------------------------------- *)

Module Jrl.

Section Reinforce.
  Variable Param : Type.
  Variable theta : nat -> Param.
  Variable dist : Param -> Param -> R.
  Variable stationary_point : Param -> Prop.

  Definition converges_to_stationary_point : Prop :=
    exists theta_star,
      stationary_point theta_star /\
      forall eps : R,
        eps > 0 ->
        exists n0 : nat,
          forall n, (n >= n0)%nat -> dist (theta n) theta_star < eps.

  Variable alpha : nat -> R.
  Variable step_sum : nat -> R.
  Variable square_step_sum : nat -> R.

  Definition robbins_monro : Prop :=
    (forall n, 0 < alpha n) /\
    (forall target : R,
       exists n : nat, target <= step_sum n) /\
    (exists upper : R,
       forall n : nat, square_step_sum n <= upper).

  Hypothesis stochastic_approximation_theorem :
    robbins_monro -> converges_to_stationary_point.

  Theorem reinforce_converges_under_robbins_monro :
    robbins_monro -> converges_to_stationary_point.
  Proof.
    apply stochastic_approximation_theorem.
  Qed.
End Reinforce.

End Jrl.

(* -------------------------------------------------------------------------- *)
(* POT-J: strict convexity gives a unique global optimum.                      *)
(* -------------------------------------------------------------------------- *)

Module PotJ.

Section StrictConvexity.
  Variable Plan : Type.
  Variable mid : Plan -> Plan -> Plan.
  Variable cost : Plan -> R.

  Definition minimizer (p : Plan) : Prop :=
    forall q, cost p <= cost q.

  Definition strict_mid_convex : Prop :=
    forall p q,
      p <> q ->
      cost (mid p q) < (cost p + cost q) / 2.

  Theorem strict_convex_minimizer_unique :
    strict_mid_convex ->
    forall p q, minimizer p -> minimizer q -> p = q.
  Proof.
    intros Hstrict p q Hp Hq.
    destruct (classic (p = q)) as [Heq | Hneq]; [exact Heq | exfalso].
    pose proof (Hp (mid p q)) as Hp_mid.
    pose proof (Hq (mid p q)) as Hq_mid.
    pose proof (Hstrict p q Hneq) as Hmid.
    lra.
  Qed.

End StrictConvexity.

End PotJ.

(* -------------------------------------------------------------------------- *)
(* SOC-Prompt: Bellman/HJB pointwise verification wrapper.                     *)
(* -------------------------------------------------------------------------- *)

Module SocPrompt.

Section HjbVerification.
  Variable State Control : Type.
  Variable q_value : State -> Control -> R.
  Variable choose : State -> Control.
  Variable value : State -> R.

  Definition bellman_attainment : Prop :=
    forall s, value s = q_value s (choose s).

  Definition bellman_lower_bound : Prop :=
    forall s u, value s <= q_value s u.

  Definition hjb_solution_exists : Prop :=
    bellman_attainment /\ bellman_lower_bound.

  Theorem verification_theorem :
    hjb_solution_exists ->
    forall s u, q_value s (choose s) <= q_value s u.
  Proof.
    intros [Hattain Hlower] s u.
    rewrite <- Hattain.
    apply Hlower.
  Qed.
End HjbVerification.

End SocPrompt.
