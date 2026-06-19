(** * StreamEvents: Formal Model of Streaming Event Processing
    
    This module formalizes the streaming event handling in JFC.
    We model:
    
    1. Event categorization (commits_output)
    2. Event ordering guarantees
    3. Stream state machine
    
    References:
    - crates/jfc-provider/src/lib.rs (StreamEvent, FrameCategory)
    - crates/jfc-engine/src/runtime/events.rs
*)

Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.Arith.Arith.
Import ListNotations.

(** ** Frame Category
    
    Mirrors FrameCategory from jfc-provider.
    Determines how a frame affects output state.
*)
Inductive FrameCategory : Type :=
  | CommitsOutput     (* Finalizes content - text_delta, tool_use complete *)
  | Advisory          (* Metadata only - input_json_delta, thinking *)
  | Terminal          (* Stream ends - message_stop, error *)
  | Structural.       (* Protocol frames - message_start, content_block_start *)

(** ** Stream Event Model *)

Inductive StreamEvent : Type :=
  | TextDelta : nat -> StreamEvent       (* token count *)
  | ToolUseStart : nat -> StreamEvent    (* tool id *)
  | ToolInputDelta : nat -> StreamEvent  (* tool id *)
  | ToolUseComplete : nat -> StreamEvent (* tool id *)
  | MessageStart : StreamEvent
  | MessageStop : StreamEvent
  | ContentBlockStart : StreamEvent
  | ContentBlockStop : StreamEvent
  | Error : StreamEvent
  | Ping : StreamEvent.

(** Category assignment *)
Definition category (e : StreamEvent) : FrameCategory :=
  match e with
  | TextDelta _ => CommitsOutput
  | ToolUseComplete _ => CommitsOutput
  | ToolUseStart _ => Structural
  | ToolInputDelta _ => Advisory
  | MessageStart => Structural
  | MessageStop => Terminal
  | ContentBlockStart => Structural
  | ContentBlockStop => Structural
  | Error => Terminal
  | Ping => Advisory
  end.

(** ** Output Commitment Predicate *)

Definition commits_output (e : StreamEvent) : bool :=
  match category e with
  | CommitsOutput => true
  | _ => false
  end.

(** ** Stream State *)

Inductive StreamState : Type :=
  | Idle
  | MessageInProgress
  | ContentBlockActive
  | ToolUseActive : nat -> StreamState  (* active tool id *)
  | Terminated.

(** ** Stream Transitions *)

Definition stream_transition (st : StreamState) (e : StreamEvent) : option StreamState :=
  match st, e with
  (* Idle transitions *)
  | Idle, MessageStart => Some MessageInProgress
  | Idle, _ => None  (* Invalid: no message started *)
  
  (* Message in progress *)
  | MessageInProgress, ContentBlockStart => Some ContentBlockActive
  | MessageInProgress, MessageStop => Some Terminated
  | MessageInProgress, Error => Some Terminated
  | MessageInProgress, Ping => Some MessageInProgress
  
  (* Content block active *)
  | ContentBlockActive, TextDelta _ => Some ContentBlockActive
  | ContentBlockActive, ToolUseStart id => Some (ToolUseActive id)
  | ContentBlockActive, ContentBlockStop => Some MessageInProgress
  
  (* Tool use active *)
  | ToolUseActive id, ToolInputDelta id' =>
      if Nat.eqb id id' then Some (ToolUseActive id) else None
  | ToolUseActive id, ToolUseComplete id' =>
      if Nat.eqb id id' then Some ContentBlockActive else None
  
  (* Terminated is absorbing *)
  | Terminated, _ => None
  
  (* Default: invalid transition *)
  | _, _ => None
  end.

(** ** Well-Formed Stream *)

Fixpoint process_stream (st : StreamState) (events : list StreamEvent) : option StreamState :=
  match events with
  | [] => Some st
  | e :: rest =>
      match stream_transition st e with
      | Some st' => process_stream st' rest
      | None => None
      end
  end.

Definition well_formed_stream (events : list StreamEvent) : Prop :=
  process_stream Idle events <> None.

(** ** Stream Theorems *)

(** Theorem: Empty stream is well-formed *)
Theorem empty_stream_wellformed :
  process_stream Idle [] = Some Idle.
Proof.
  reflexivity.
Qed.

(** Theorem: Terminated state is absorbing *)
Theorem terminated_absorbing :
  forall e,
    stream_transition Terminated e = None.
Proof.
  intros e. destruct e; reflexivity.
Qed.

(** Theorem: Processing after termination fails *)
Theorem no_events_after_terminal :
  forall events e,
    process_stream Terminated (e :: events) = None.
Proof.
  intros events e.
  destruct e; reflexivity.
Qed.

(** Theorem: Message must start before content *)
Theorem message_before_content :
  forall events,
    process_stream Idle (ContentBlockStart :: events) = None.
Proof.
  intros events. simpl. reflexivity.
Qed.

(** Theorem: A proper stream starts with MessageStart *)
Lemma valid_stream_starts_with_message :
  forall e events,
    process_stream Idle (e :: events) <> None ->
    e = MessageStart.
Proof.
  intros e events H.
  destruct e; simpl in H; try contradiction.
  reflexivity.
Qed.

(** ** Output Counting *)

Fixpoint count_output_events (events : list StreamEvent) : nat :=
  match events with
  | [] => 0
  | e :: rest =>
      (if commits_output e then 1 else 0) + count_output_events rest
  end.

(** Theorem: Only CommitsOutput category events are counted *)
Theorem output_count_matches_category :
  forall events,
    count_output_events events =
    length (filter commits_output events).
Proof.
  intros events.
  induction events as [|e rest IH].
  - reflexivity.
  - simpl. destruct (commits_output e) eqn:Hco.
    + simpl. rewrite IH. reflexivity.
    + simpl. exact IH.
Qed.

(** ** Tool Use Ordering
    
    A tool use block must follow: ToolUseStart -> ToolInputDelta* -> ToolUseComplete
*)

Inductive ToolUseSequence : nat -> list StreamEvent -> Prop :=
  | ToolSeqStart : forall id,
      ToolUseSequence id [ToolUseStart id]
  | ToolSeqDelta : forall id events,
      ToolUseSequence id events ->
      ToolUseSequence id (events ++ [ToolInputDelta id])
  | ToolSeqComplete : forall id events,
      ToolUseSequence id events ->
      ToolUseSequence id (events ++ [ToolUseComplete id]).

(** Theorem: Valid tool sequence starts with ToolUseStart *)
Theorem tool_sequence_starts_with_start :
  forall id events,
    ToolUseSequence id events ->
    exists rest, events = ToolUseStart id :: rest.
Proof.
  intros id events H.
  induction H.
  - exists []. reflexivity.
  - destruct IHToolUseSequence as [rest' Heq].
    exists (rest' ++ [ToolInputDelta id]).
    rewrite Heq. reflexivity.
  - destruct IHToolUseSequence as [rest' Heq].
    exists (rest' ++ [ToolUseComplete id]).
    rewrite Heq. reflexivity.
Qed.
