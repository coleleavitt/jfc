(** * SessionState: Formal Model of Session Lifecycle
    
    This module formalizes session state management in JFC.
    We model:
    
    1. Session state transitions
    2. Persistence invariants
    3. Resume/replay correctness
    
    References:
    - crates/jfc-anthropic-sdk/src/sessions.rs
    - crates/jfc-session/src/lib.rs
*)

Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.Arith.Arith.
Require Import Coq.micromega.Lia.
Import ListNotations.

Require Import JFC.ExecutionStatus.

(** ** Session State
    
    Mirrors SessionState from jfc-anthropic-sdk.
*)
Inductive SessionState : Type :=
  | Initializing
  | Active
  | Suspended
  | Resuming
  | Closed
  | Failed.

(** ** State Predicates *)

Definition is_session_terminal (s : SessionState) : bool :=
  match s with
  | Closed | Failed => true
  | _ => false
  end.

Definition is_session_active (s : SessionState) : bool :=
  match s with
  | Active => true
  | _ => false
  end.

Definition can_accept_input (s : SessionState) : bool :=
  match s with
  | Active => true
  | _ => false
  end.

(** ** State Transitions *)

Definition session_transition (s : SessionState) (action : nat) : option SessionState :=
  match s, action with
  (* Initializing transitions *)
  | Initializing, 0 (* init_complete *) => Some Active
  | Initializing, 1 (* init_failed *) => Some Failed
  
  (* Active transitions *)
  | Active, 2 (* suspend *) => Some Suspended
  | Active, 3 (* close *) => Some Closed
  | Active, 4 (* error *) => Some Failed
  
  (* Suspended transitions *)
  | Suspended, 5 (* resume *) => Some Resuming
  | Suspended, 3 (* close *) => Some Closed
  
  (* Resuming transitions *)
  | Resuming, 6 (* resume_complete *) => Some Active
  | Resuming, 1 (* resume_failed *) => Some Failed
  
  (* Terminal states *)
  | Closed, _ => None
  | Failed, _ => None
  
  | _, _ => None
  end.

(** ** Session History *)

Record SessionEvent : Type := mkSessionEvent {
  event_timestamp : nat;
  event_action : nat;
  event_from_state : SessionState;
  event_to_state : SessionState;
}.

Definition SessionHistory := list SessionEvent.

(** Valid history: each transition is valid *)
Fixpoint valid_history (h : SessionHistory) (initial : SessionState) : Prop :=
  match h with
  | [] => True
  | e :: rest =>
      event_from_state e = initial /\
      session_transition (event_from_state e) (event_action e) = Some (event_to_state e) /\
      valid_history rest (event_to_state e)
  end.

(** ** Session Theorems *)

(** Theorem: Terminal states are absorbing *)
Theorem session_terminal_absorbing :
  forall s action,
    is_session_terminal s = true ->
    session_transition s action = None.
Proof.
  intros s action H.
  destruct s; simpl in H; try discriminate.
  - (* Closed *) destruct action; reflexivity.
  - (* Failed *) destruct action; reflexivity.
Qed.

(** Theorem: Only Active state can accept input *)
Theorem only_active_accepts_input :
  forall s,
    can_accept_input s = true <-> s = Active.
Proof.
  intros s. split.
  - destruct s; simpl; intro H; try discriminate. reflexivity.
  - intro H. subst. reflexivity.
Qed.

(** Theorem: Initialization leads to Active or Failed *)
Theorem init_binary_outcome :
  forall action s',
    session_transition Initializing action = Some s' ->
    s' = Active \/ s' = Failed.
Proof.
  intros action s' H.
  destruct action as [|[|action]]; simpl in H; try discriminate;
    inversion H; subst; auto.
Qed.

(** ** Persistence Model *)

Record PersistentSession : Type := mkPersistentSession {
  session_id : nat;
  persist_state : SessionState;
  message_count : nat;
  tool_call_count : nat;
  checksum : nat;
}.

(** Checksum invariant: should match computed value *)
Definition compute_checksum (ps : PersistentSession) : nat :=
  session_id ps + message_count ps + tool_call_count ps.

Definition checksum_valid (ps : PersistentSession) : Prop :=
  checksum ps = compute_checksum ps.

(** ** Resume Correctness *)

(** Resume from persisted state should restore session *)
Definition resume_preserves_state (before after : PersistentSession) : Prop :=
  session_id before = session_id after /\
  message_count before = message_count after /\
  (* After resume, state should be Active if it was Suspended *)
  (persist_state before = Suspended -> persist_state after = Active).

(** Theorem: Resume transitions through Resuming *)
Theorem resume_goes_through_resuming :
  forall s s' action,
    s = Suspended ->
    session_transition s action = Some s' ->
    s' = Resuming \/ s' = Closed.
Proof.
  intros s s' action Hs H.
  subst s.
  destruct action as [|[|[|[|[|[|action]]]]]]; simpl in H; try discriminate;
    inversion H; subst; auto.
Qed.

(** ** Replay Model
    
    Replaying a session history should arrive at the same state.
*)

Fixpoint replay_history (h : SessionHistory) (s : SessionState) : option SessionState :=
  match h with
  | [] => Some s
  | e :: rest =>
      match session_transition s (event_action e) with
      | Some s' => replay_history rest s'
      | None => None
      end
  end.

(** Theorem: Valid history replays successfully *)
Theorem valid_history_replays :
  forall h initial,
    valid_history h initial ->
    replay_history h initial <> None.
Proof.
  intros h.
  induction h as [|e rest IH].
  - intros initial _. simpl. discriminate.
  - intros initial [Hfrom [Htrans Hrest]].
    simpl.
    rewrite Hfrom in Htrans.
    rewrite Htrans.
    apply IH. exact Hrest.
Qed.

(** ** Message Ordering Invariant *)

Record Message' : Type := mkMessage' {
  msg_index : nat;
  msg_timestamp : nat;
}.

Definition messages_ordered (msgs : list Message') : Prop :=
  forall i j m_i m_j,
    nth_error msgs i = Some m_i ->
    nth_error msgs j = Some m_j ->
    i < j ->
    msg_index m_i < msg_index m_j /\
    msg_timestamp m_i <= msg_timestamp m_j.

(** Theorem: Appending preserves order *)
Theorem append_preserves_order :
  forall msgs m,
    messages_ordered msgs ->
    (forall m', In m' msgs -> msg_index m' < msg_index m) ->
    (forall m', In m' msgs -> msg_timestamp m' <= msg_timestamp m) ->
    messages_ordered (msgs ++ [m]).
Proof.
  intros msgs m Hord Hidx Hts.
  unfold messages_ordered.
  unfold messages_ordered in Hord.
  intros i j m_i m_j Hi Hj Hij.
  destruct (Nat.lt_ge_cases j (length msgs)) as [Hjlt|Hjge].
  - (* Both in original list *)
    rewrite nth_error_app1 in Hi by lia.
    rewrite nth_error_app1 in Hj by lia.
    eapply Hord; eauto.
  - (* j points to appended element *)
    assert (j = length msgs) as Hjeq.
    { assert (j < length (msgs ++ [m])) as Hjbound.
      { apply nth_error_Some. rewrite Hj. discriminate. }
      rewrite app_length in Hjbound. simpl in Hjbound.
      lia. }
    subst j.
    rewrite nth_error_app2 in Hj by lia.
    replace (length msgs - length msgs) with 0 in Hj by lia.
    simpl in Hj. injection Hj. intro Hmj. subst m_j.
    (* m_i is in original list *)
    assert (i < length msgs) as Hilt by lia.
    rewrite nth_error_app1 in Hi by exact Hilt.
    split.
    + apply Hidx. apply nth_error_In with i. exact Hi.
    + apply Hts. apply nth_error_In with i. exact Hi.
Qed.
