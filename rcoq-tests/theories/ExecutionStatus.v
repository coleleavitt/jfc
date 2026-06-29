(** * ExecutionStatus: Formal Model of Tool/Task Lifecycle
    
    This module formalizes the ExecutionStatus state machine from
    jfc-core/src/execution.rs. We prove:
    
    1. Terminal states are absorbing (no outgoing transitions)
    2. Alive and terminal states partition the state space
    3. State transitions respect the allows_transition_to predicate
    
    Reference: crates/jfc-core/src/execution.rs:1-83
*)

Require Import Coq.Bool.Bool.
Require Import Coq.Lists.List.
Import ListNotations.

(** ** State Definition
    
    Mirrors ExecutionStatus enum from Rust:
    {Pending, Running, Idle, Completed, Failed, Cancelled}
*)
Inductive ExecutionStatus : Type :=
  | Pending
  | Running
  | Idle
  | Completed
  | Failed
  | Cancelled.

(** ** State Predicates *)

Definition is_terminal (s : ExecutionStatus) : bool :=
  match s with
  | Completed | Failed | Cancelled => true
  | _ => false
  end.

Definition is_alive (s : ExecutionStatus) : bool :=
  match s with
  | Pending | Running | Idle => true
  | _ => false
  end.

(** Boolean equality for ExecutionStatus *)
Definition status_beq (s1 s2 : ExecutionStatus) : bool :=
  match s1, s2 with
  | Pending, Pending => true
  | Running, Running => true
  | Idle, Idle => true
  | Completed, Completed => true
  | Failed, Failed => true
  | Cancelled, Cancelled => true
  | _, _ => false
  end.

(** ** Transition Function
    
    Mirrors allows_transition_to from Rust.
    A transition from s to t is valid iff:
    - s = t (reflexive), OR
    - s is not terminal
*)
Definition allows_transition (s t : ExecutionStatus) : bool :=
  if status_beq s t then true
  else negb (is_terminal s).

(** ** Theorems *)

(** Theorem 1: Terminal and alive states partition the space *)
Theorem terminal_alive_partition :
  forall s : ExecutionStatus,
    xorb (is_terminal s) (is_alive s) = true.
Proof.
  intros s.
  destruct s; simpl; reflexivity.
Qed.

(** Theorem 2: Terminal states are absorbing (self-loops only) *)
Theorem terminal_absorbing :
  forall s t : ExecutionStatus,
    is_terminal s = true ->
    allows_transition s t = true ->
    status_beq s t = true.
Proof.
  intros s t Hterm Htrans.
  unfold allows_transition in Htrans.
  destruct (status_beq s t) eqn:Heq.
  - reflexivity.
  - (* If status_beq s t = false, then allows_transition requires negb (is_terminal s) = true *)
    simpl in Htrans.
    rewrite Hterm in Htrans.
    simpl in Htrans.
    discriminate.
Qed.

(** Theorem 3: Alive states can transition to any state *)
Theorem alive_can_transition :
  forall s t : ExecutionStatus,
    is_alive s = true ->
    allows_transition s t = true.
Proof.
  intros s t Halive.
  unfold allows_transition.
  destruct (status_beq s t) eqn:Heq.
  - reflexivity.
  - (* s is alive, so is_terminal s = false *)
    assert (is_terminal s = false) as Hnoterm.
    { destruct s; simpl in Halive; try discriminate; reflexivity. }
    rewrite Hnoterm. simpl. reflexivity.
Qed.

(** Theorem 4: Transition relation is reflexive *)
Theorem transition_reflexive :
  forall s : ExecutionStatus,
    allows_transition s s = true.
Proof.
  intros s.
  unfold allows_transition.
  destruct s; simpl; reflexivity.
Qed.

(** ** All States List (for exhaustive checks) *)
Definition all_states : list ExecutionStatus :=
  [Pending; Running; Idle; Completed; Failed; Cancelled].

(** Theorem 5: All states are either alive or terminal (constructive) *)
Theorem all_states_classified :
  forall s : ExecutionStatus,
    In s all_states ->
    (is_alive s = true \/ is_terminal s = true).
Proof.
  intros s Hin.
  destruct s; [left | left | left | right | right | right]; reflexivity.
Qed.
