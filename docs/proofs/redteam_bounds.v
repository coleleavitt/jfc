(*
  Small Rocq/Coq formalization for the controlled red-team search metadata.

  This file proves only the arithmetic/query-bound facts that are actually
  formal in the implementation. It deliberately does not claim to prove LLM
  convergence assumptions such as attacker full support, stationary bandit arms,
  or GP/RKHS model correctness.
*)

From Stdlib Require Import Arith Lia Reals Lra.

Definition pair_bound (n_streams iterations : nat) : nat :=
  n_streams * iterations.

Definition tap_ucb_bound (iterations prune_width : nat) : nat :=
  iterations * prune_width.

Definition autodan_bound (population generations : nat) : nat :=
  population * generations.

Definition multiturn_bound (max_turns : nat) : nat := max_turns.

Definition potj_target_bound (iterations : nat) : nat := iterations.

Definition potj_proposal_bound (iterations candidates : nat) : nat :=
  iterations * candidates.

Definition jrl_bound (max_turns : nat) : nat := max_turns.

Definition soc_prompt_bound (max_turns : nat) : nat := max_turns.

Theorem pair_queries_within_bound :
  forall n_streams iterations queries,
    queries <= n_streams * iterations ->
    queries <= pair_bound n_streams iterations.
Proof.
  intros. unfold pair_bound. exact H.
Qed.

Theorem tap_ucb_queries_within_bound :
  forall iterations prune_width queries,
    queries <= iterations * prune_width ->
    queries <= tap_ucb_bound iterations prune_width.
Proof.
  intros. unfold tap_ucb_bound. exact H.
Qed.

Theorem autodan_queries_within_bound :
  forall population generations queries,
    queries <= population * generations ->
    queries <= autodan_bound population generations.
Proof.
  intros. unfold autodan_bound. exact H.
Qed.

Theorem multiturn_queries_within_bound :
  forall max_turns queries,
    queries <= max_turns ->
    queries <= multiturn_bound max_turns.
Proof.
  intros. unfold multiturn_bound. exact H.
Qed.

Theorem potj_target_queries_within_bound :
  forall iterations queries,
    queries <= iterations ->
    queries <= potj_target_bound iterations.
Proof.
  intros. unfold potj_target_bound. exact H.
Qed.

Theorem potj_proposals_within_bound :
  forall iterations candidates proposals,
    proposals <= iterations * candidates ->
    proposals <= potj_proposal_bound iterations candidates.
Proof.
  intros. unfold potj_proposal_bound. exact H.
Qed.

Theorem jrl_queries_within_bound :
  forall max_turns queries,
    queries <= max_turns ->
    queries <= jrl_bound max_turns.
Proof.
  intros. unfold jrl_bound. exact H.
Qed.

Theorem soc_prompt_queries_within_bound :
  forall max_turns queries,
    queries <= max_turns ->
    queries <= soc_prompt_bound max_turns.
Proof.
  intros. unfold soc_prompt_bound. exact H.
Qed.

Open Scope R_scope.

Theorem casp_hitting_time_algebra :
  forall h0 mu tau : R,
    mu <> 0 ->
    1 - tau * mu = h0 ->
    tau = (1 - h0) / mu.
Proof.
  intros h0 mu tau Hmu Hmart.
  unfold Rdiv.
  apply Rmult_eq_reg_r with (r := mu); [| exact Hmu].
  rewrite Rmult_assoc.
  rewrite Rinv_l; lra.
Qed.
