(** * KVCacheEviction: Formal Model of KV Cache Management for LLMs

    This module formalizes KV cache eviction strategies based on recent
    research papers. These techniques are directly applicable to JFC's
    context management.

    Primary grounding: heavy-hitter (H2O) eviction, Zhang et al. 2023
    ("H2O: Heavy-Hitter Oracle for Efficient Generative Inference of Large
    Language Models", arXiv:2306.14048).  The cache is bounded by a budget;
    when it overflows we keep the highest accumulated-attention entries (the
    "heavy hitters") and evict the lowest-scoring ones.  Modeled here on a
    cache that is kept sorted by attention score in descending order, so that
    keeping the top [budget] entries (a prefix) is exactly heavy-hitter
    retention.

    The theorems below are real, strong invariants:
      - the cache size never exceeds the budget after eviction;
      - eviction removes a minimum-score element and never a heavy hitter
        ranked above it (every retained entry scores >= every evicted entry);
      - the retained set is exactly the top-scored prefix;
      - eviction is monotone in the budget: raising the budget retains a
        superset of the previously retained entries;
      - inserting one entry into a full cache and then evicting leaves the
        length equal to the budget.

    Every theorem is fully proved (no [admit]/[Admitted]).  Where the original
    statement was a vacuous [True] placeholder or false as written, it has been
    restated to a true and non-trivial theorem and marked with a [CORRECTION]
    note.

    References:
    - Zhang et al. 2023, H2O (arXiv:2306.14048): heavy-hitter eviction.
    - Xiao et al. 2023, StreamingLLM (arXiv:2309.17453): attention sinks.
    - crates/jfc-core/src/retention.rs, crates/jfc-engine/src/compact/engine.rs
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Require Import Coq.Sorting.Sorted.
Import ListNotations.

(** ** KV Cache Entry Model *)

Record KVEntry : Type := mkKVEntry {
  kv_position : nat;           (* Token position *)
  kv_layer : nat;              (* Transformer layer *)
  kv_attention_score : nat;    (* Accumulated attention received, scaled 0-1000 *)
  kv_recent_access : nat;      (* Recency score *)
  kv_size : nat;               (* Memory footprint in bytes *)
}.

Definition KVCache := list KVEntry.

(** ** Attention Score Accumulation *)

(** Pointwise zip of two lists (defined before use). *)
Fixpoint combine_with {A B C : Type} (f : A -> B -> C) (l1 : list A) (l2 : list B) : list C :=
  match l1, l2 with
  | [], _ => []
  | _, [] => []
  | x :: xs, y :: ys => f x y :: combine_with f xs ys
  end.

(** Update attention scores based on latest forward pass (EMA blend). *)
Definition update_attention (cache : KVCache) (new_scores : list nat) : KVCache :=
  combine_with (fun entry score =>
    mkKVEntry (kv_position entry) (kv_layer entry)
              ((kv_attention_score entry + score) / 2)  (* EMA *)
              (kv_recent_access entry)
              (kv_size entry)
  ) cache new_scores.

(** ** Heavy-hitter ordering

    A cache is "heavy-hitter sorted" when entries are in non-increasing
    attention-score order, so the heaviest hitters sit at the front.  This is
    the precondition the H2O eviction strategies assume: with the cache sorted
    by accumulated attention, keeping a [budget]-length prefix retains exactly
    the heavy hitters. *)
Definition score (e : KVEntry) : nat := kv_attention_score e.

Definition hh_sorted (cache : KVCache) : Prop :=
  Sorted (fun a b => score a >= score b) cache.

(** ** Eviction Strategies *)

(** Strategy 1: Evict lowest attention score.  With the cache sorted by
    attention descending, the top [budget] entries are the heavy hitters. *)
Definition evict_by_attention (cache : KVCache) (budget : nat) : KVCache :=
  firstn budget cache.

(** Strategy 2: Evict oldest (FIFO) *)
Definition evict_fifo (cache : KVCache) (budget : nat) : KVCache :=
  skipn (length cache - budget) cache.

(** Strategy 3: LRU - evict least recently used (cache pre-sorted by recency) *)
Definition evict_lru (cache : KVCache) (budget : nat) : KVCache :=
  firstn budget cache.

(** Strategy 4: Combined score (attention + recency) *)
Definition combined_score (entry : KVEntry) : nat :=
  kv_attention_score entry + kv_recent_access entry.

Definition evict_combined (cache : KVCache) (budget : nat) : KVCache :=
  firstn budget cache.  (* Assumes sorted by combined_score *)

(** The set of entries dropped by attention eviction (the complementary
    suffix).  retained ++ evicted = cache. *)
Definition evicted_by_attention (cache : KVCache) (budget : nat) : KVCache :=
  skipn budget cache.

Lemma retained_app_evicted :
  forall cache budget,
    evict_by_attention cache budget ++ evicted_by_attention cache budget = cache.
Proof.
  intros cache budget.
  unfold evict_by_attention, evicted_by_attention.
  apply firstn_skipn.
Qed.

(** ** Eviction Theorems *)

(** Theorem: Any eviction strategy respects the budget. *)
Theorem eviction_respects_budget :
  forall cache budget,
    length (evict_by_attention cache budget) <= budget /\
    length (evict_fifo cache budget) <= budget /\
    length (evict_lru cache budget) <= budget.
Proof.
  intros cache budget.
  split; [|split].
  - unfold evict_by_attention. apply firstn_le_length.
  - unfold evict_fifo. rewrite skipn_length. lia.
  - unfold evict_lru. apply firstn_le_length.
Qed.

(** When the cache is at least as large as the budget, attention eviction
    fills the budget exactly (no slack). *)
Theorem eviction_fills_budget :
  forall cache budget,
    budget <= length cache ->
    length (evict_by_attention cache budget) = budget.
Proof.
  intros cache budget Hle.
  unfold evict_by_attention. rewrite length_firstn. lia.
Qed.

(** *** Heavy hitters are never evicted

    In a heavy-hitter-sorted cache, every entry that survives eviction scores
    at least as high as every entry that gets evicted.  Equivalently: eviction
    removes minimum-score elements and never a heavy hitter ranked above a
    retained one.  This is the core H2O correctness property. *)

(** The heavy-hitter relation is transitive, so [Sorted] strengthens to
    [StronglySorted]: each element dominates the whole tail after it. *)
Lemma hh_score_transitive :
  Relations_1.Transitive (fun a b => score a >= score b).
Proof. intros a b c Hab Hbc. lia. Qed.

Lemma hh_strongly_sorted :
  forall cache, hh_sorted cache ->
    StronglySorted (fun a b => score a >= score b) cache.
Proof.
  intros cache Hs. apply Sorted_StronglySorted.
  - exact hh_score_transitive.
  - exact Hs.
Qed.

(** In a non-increasing-score list, the head dominates every later element. *)
Lemma hh_head_max :
  forall x xs,
    hh_sorted (x :: xs) ->
    forall y, In y xs -> score x >= score y.
Proof.
  intros x xs Hs y Hin.
  apply hh_strongly_sorted in Hs.
  apply StronglySorted_inv in Hs as [_ Hfa].
  rewrite Forall_forall in Hfa.
  apply Hfa. exact Hin.
Qed.

(** Generalization: in a heavy-hitter-sorted list, an earlier element dominates
    a later one.  Phrased on a split: if [pre ++ suf] is sorted then every
    entry in [pre] dominates every entry in [suf]. *)
Lemma hh_prefix_dominates_suffix :
  forall pre suf,
    hh_sorted (pre ++ suf) ->
    forall r e, In r pre -> In e suf -> score r >= score e.
Proof.
  induction pre as [|x xs IH]; intros suf Hs r e Hr He.
  - inversion Hr.
  - simpl in Hs.
    inversion Hr as [Heq | Hr'].
    + subst r.
      (* x dominates everything after it, in particular e in suf *)
      apply (hh_head_max x (xs ++ suf) Hs e).
      apply in_or_app. right. exact He.
    + (* r in xs: recurse on the tail, which is also sorted *)
      unfold hh_sorted in Hs.
      apply Sorted_inv in Hs as [Hs' _].
      apply (IH suf Hs' r e Hr' He).
Qed.

(** Theorem: attention eviction preserves the heavy hitters.

    [CORRECTION] The original [attention_preserves_important] concluded [True]
    (a vacuous "probabilistic guarantee").  Replaced with the real H2O
    invariant: on a heavy-hitter-sorted cache, every retained entry scores at
    least as high as every evicted entry, so no heavy hitter is dropped in
    favor of a lower-scored one. *)
Theorem attention_preserves_important :
  forall cache budget r e,
    hh_sorted cache ->
    In r (evict_by_attention cache budget) ->
    In e (evicted_by_attention cache budget) ->
    score r >= score e.
Proof.
  intros cache budget r e Hs Hr He.
  apply (hh_prefix_dominates_suffix
           (evict_by_attention cache budget)
           (evicted_by_attention cache budget)).
  - rewrite retained_app_evicted. exact Hs.
  - exact Hr.
  - exact He.
Qed.

(** Sharper corollary: any heavy hitter scoring strictly above some retained
    entry is itself retained (never evicted).  This rules out the failure mode
    where a higher-scored entry is dropped while a lower one survives. *)
Theorem no_inversion_eviction :
  forall cache budget hh kept,
    hh_sorted cache ->
    In kept (evict_by_attention cache budget) ->
    In hh (evicted_by_attention cache budget) ->
    score hh > score kept ->
    False.
Proof.
  intros cache budget hh kept Hs Hkept Hhh Hgt.
  pose proof (attention_preserves_important cache budget kept hh Hs Hkept Hhh) as Hle.
  lia.
Qed.

(** *** Retained set is exactly the top-scored prefix

    The retained set is a prefix of the (sorted) cache, hence both a sublist
    (every retained entry is in the cache) and the maximal-score selection. *)

Theorem retained_subset :
  forall cache budget e,
    In e (evict_by_attention cache budget) -> In e cache.
Proof.
  intros cache budget e Hin.
  unfold evict_by_attention in Hin.
  rewrite <- (firstn_skipn budget cache).
  apply in_or_app. left. exact Hin.
Qed.

(** Theorem: the retained set IS the top-scored entries — no entry outside the
    retained prefix outranks one inside it.  (Combines [retained_subset] with
    [attention_preserves_important].) *)
Theorem retained_are_top_scored :
  forall cache budget kept dropped,
    hh_sorted cache ->
    In kept (evict_by_attention cache budget) ->
    In dropped (evicted_by_attention cache budget) ->
    score kept >= score dropped /\ In kept cache.
Proof.
  intros cache budget kept dropped Hs Hk Hd.
  split.
  - apply (attention_preserves_important cache budget kept dropped Hs Hk Hd).
  - apply (retained_subset cache budget kept Hk).
Qed.

(** *** Monotone eviction: raising the budget retains a superset *)

(** [firstn b1 l] is a prefix of [firstn b2 l] when b1 <= b2. *)
Lemma firstn_prefix_mono :
  forall (A : Type) (b1 b2 : nat) (l : list A),
    b1 <= b2 ->
    firstn b1 l = firstn b1 (firstn b2 l).
Proof.
  intros A b1 b2 l Hle.
  rewrite firstn_firstn.
  rewrite Nat.min_l by exact Hle.
  reflexivity.
Qed.

(** Theorem: eviction is monotone in the budget — every entry retained at the
    smaller budget is also retained at the larger budget.

    [CORRECTION] The original file had no monotonicity theorem (and the slot it
    would have replaced, [jfc_preserves_recent], concluded [True]).  This is
    the real superset property the paper grounding asks for. *)
Theorem eviction_monotone :
  forall cache b1 b2,
    b1 <= b2 ->
    incl (evict_by_attention cache b1) (evict_by_attention cache b2).
Proof.
  intros cache b1 b2 Hle x Hx.
  unfold evict_by_attention in *.
  rewrite (firstn_prefix_mono _ b1 b2 cache Hle) in Hx.
  (* x in firstn b1 (firstn b2 cache) ==> x in firstn b2 cache *)
  rewrite <- (firstn_skipn b1 (firstn b2 cache)).
  apply in_or_app. left. exact Hx.
Qed.

(** Monotone in length too: a larger budget never retains fewer entries. *)
Theorem eviction_monotone_length :
  forall cache b1 b2,
    b1 <= b2 ->
    length (evict_by_attention cache b1) <= length (evict_by_attention cache b2).
Proof.
  intros cache b1 b2 Hle.
  unfold evict_by_attention. rewrite !length_firstn. lia.
Qed.

(** *** Insert-then-evict on a full cache holds the length at the budget *)

(** Insert a fresh entry at the heavy-hitter position dictated by its score is
    modeled abstractly: insertion grows the cache by exactly one entry, then
    eviction trims back to the budget.  We only need the length facts. *)
Definition insert_entry (cache : KVCache) (e : KVEntry) : KVCache :=
  e :: cache.

Lemma insert_length :
  forall cache e, length (insert_entry cache e) = S (length cache).
Proof. intros. reflexivity. Qed.

(** Theorem: inserting into a full cache (length = budget) and then evicting
    leaves the cache length exactly the budget — no overflow, no underflow.

    [CORRECTION] The original [joint_better_retention] concluded [True].  This
    is the concrete bounded-cache invariant: a full cache stays full across an
    insert+evict cycle. *)
Theorem full_insert_evict_keeps_budget :
  forall cache budget e,
    length cache = budget ->
    length (evict_by_attention (insert_entry cache e) budget) = budget.
Proof.
  intros cache budget e Hfull.
  unfold evict_by_attention.
  rewrite length_firstn, insert_length, Hfull.
  lia.
Qed.

(** More generally, any insert+evict cycle keeps the length within budget. *)
Theorem insert_evict_respects_budget :
  forall cache budget e,
    length (evict_by_attention (insert_entry cache e) budget) <= budget.
Proof.
  intros cache budget e.
  unfold evict_by_attention. apply firstn_le_length.
Qed.

(** ** PolyKV: Shared Cache Pool Model *)

Record SharedPool : Type := mkSharedPool {
  pool_entries : KVCache;
  pool_compression_ratio : nat;  (* 0-100, lower = more compressed *)
  pool_agent_count : nat;
}.

Definition agent_view (pool : SharedPool) (agent_id : nat) : KVCache :=
  pool_entries pool.

(** Theorem: a shared pool saves memory vs per-agent caches.  We require a
    non-empty cache so the saving is strict (an empty cache saves nothing). *)
Theorem shared_saves_memory :
  forall n_agents cache_size,
    n_agents >= 2 ->
    cache_size >= 1 ->
    cache_size < n_agents * cache_size.
Proof.
  intros n_agents cache_size Hn Hc. nia.
Qed.

(** ** ReST-KV: Layer-wise Output Reconstruction *)

Record LayerReconstruction : Type := mkReconstruction {
  recon_layer : nat;
  recon_error : nat;        (* Reconstruction error, lower is better *)
  recon_compression : nat;  (* Compression ratio achieved *)
}.

(** Sum of compression ratios for layers within the error threshold. *)
Definition optimal_compression (layers : list LayerReconstruction) (error_threshold : nat)
    : nat :=
  fold_left (fun acc l =>
    if Nat.leb (recon_error l) error_threshold then
      acc + recon_compression l
    else
      acc
  ) layers 0.

(** Accumulator-shift lemma for the threshold-gated fold (cf. CompressionBounds'
    [fold_left_add_shift], extended to a conditional add). *)
Lemma optimal_compression_shift :
  forall layers t z,
    fold_left (fun acc l =>
      if Nat.leb (recon_error l) t then acc + recon_compression l else acc) layers z
    = z + fold_left (fun acc l =>
      if Nat.leb (recon_error l) t then acc + recon_compression l else acc) layers 0.
Proof.
  intros layers t. induction layers as [|l ls IH]; intros z.
  - simpl. lia.
  - simpl. destruct (Nat.leb (recon_error l) t) eqn:Hb.
    + rewrite (IH (z + recon_compression l)). rewrite (IH (recon_compression l)). lia.
    + rewrite (IH z). reflexivity.
Qed.

Lemma optimal_compression_cons :
  forall l ls t,
    optimal_compression (l :: ls) t
    = (if Nat.leb (recon_error l) t then recon_compression l else 0)
      + optimal_compression ls t.
Proof.
  intros l ls t. unfold optimal_compression. simpl.
  destruct (Nat.leb (recon_error l) t) eqn:Hb.
  - simpl. rewrite (optimal_compression_shift ls t (recon_compression l)). lia.
  - rewrite (optimal_compression_shift ls t 0). lia.
Qed.

(** Theorem: a lower error threshold admits no more compression than a higher
    one.  (Fully proved; the original was [Admitted].) *)
Theorem error_compression_tradeoff :
  forall layers t1 t2,
    t1 <= t2 ->
    optimal_compression layers t1 <= optimal_compression layers t2.
Proof.
  intros layers t1 t2 Hle.
  induction layers as [|l ls IH].
  - unfold optimal_compression. simpl. lia.
  - rewrite !optimal_compression_cons.
    (* head contribution is monotone: if admitted under t1 it is under t2 *)
    assert (Hhead :
      (if Nat.leb (recon_error l) t1 then recon_compression l else 0)
      <= (if Nat.leb (recon_error l) t2 then recon_compression l else 0)).
    { destruct (Nat.leb (recon_error l) t1) eqn:H1.
      - apply Nat.leb_le in H1.
        assert (recon_error l <= t2) by lia.
        apply Nat.leb_le in H. rewrite H. lia.
      - destruct (Nat.leb (recon_error l) t2); lia. }
    lia.
Qed.

(** ** Spatial-Temporal Smoothing *)

Definition spatial_smooth (scores : list nat) (window : nat) : list nat :=
  map (fun i =>
    let start := i - window / 2 in
    let neighbors := firstn window (skipn start scores) in
    fold_left Nat.add neighbors 0 / max 1 (length neighbors)
  ) (seq 0 (length scores)).

(** Theorem: smoothing preserves the number of score positions.

    [CORRECTION] The original [smoothing_reduces_variance] concluded [True]
    (variance is not modeled here, and "variance reduces" is not provable
    without a variance definition).  The true, checkable structural property of
    the moving-average map is that it is length-preserving: it produces exactly
    one smoothed score per input position. *)
Theorem smoothing_preserves_length :
  forall scores window,
    length (spatial_smooth scores window) = length scores.
Proof.
  intros scores window.
  unfold spatial_smooth.
  rewrite length_map, length_seq. reflexivity.
Qed.

(** ** EVICPRESS: Joint Compression and Eviction *)

Record CompressedEntry : Type := mkCompressed {
  comp_original_size : nat;
  comp_compressed_size : nat;
  comp_quantization_bits : nat;
}.

Definition compress_entry (entry : KVEntry) (bits : nat) : CompressedEntry :=
  mkCompressed
    (kv_size entry)
    (kv_size entry * bits / 32)  (* Quantization savings *)
    bits.

(** Theorem: quantizing to at most 32 bits never grows an entry's footprint. *)
Theorem compress_entry_no_growth :
  forall entry bits,
    bits <= 32 ->
    comp_compressed_size (compress_entry entry bits) <= comp_original_size (compress_entry entry bits).
Proof.
  intros entry bits Hbits.
  unfold compress_entry, comp_compressed_size, comp_original_size.
  apply Nat.Div0.div_le_upper_bound.
  nia.
Qed.

(** Joint optimization: compress all entries, then evict only if still over
    budget.  When compression already fits, the cache is returned unchanged. *)
Definition joint_eviction (cache : KVCache) (memory_budget : nat) : KVCache :=
  let compressed_sizes := map (fun e => kv_size e / 2) cache in
  let total_compressed := fold_left Nat.add compressed_sizes 0 in
  if Nat.leb total_compressed memory_budget then
    cache
  else
    evict_by_attention cache (memory_budget * 2 / (kv_size (hd (mkKVEntry 0 0 0 0 1) cache))).

(** Theorem: when post-compression total fits the budget, joint eviction keeps
    the entire cache (retains strictly more — all of it — than pure eviction
    would when the cache exceeds the budget).

    [CORRECTION] The original [joint_better_retention] concluded [True].  This
    is the concrete retention guarantee: if compression alone brings the cache
    under budget, joint eviction evicts nothing. *)
Theorem joint_retains_all_when_fits :
  forall cache memory_budget,
    fold_left Nat.add (map (fun e => kv_size e / 2) cache) 0 <= memory_budget ->
    joint_eviction cache memory_budget = cache.
Proof.
  intros cache memory_budget Hfit.
  unfold joint_eviction.
  destruct (Nat.leb (fold_left Nat.add (map (fun e => kv_size e / 2) cache) 0) memory_budget) eqn:Hb.
  - reflexivity.
  - apply Nat.leb_gt in Hb. lia.
Qed.

(** ** Application to JFC Context Management *)

Fixpoint mapi_aux {A B : Type} (f : nat -> A -> B) (n : nat) (l : list A) : list B :=
  match l with
  | [] => []
  | x :: xs => f n x :: mapi_aux f (S n) xs
  end.

Definition mapi {A B : Type} (f : nat -> A -> B) (l : list A) : list B :=
  mapi_aux f 0 l.

Lemma mapi_aux_length :
  forall {A B : Type} (f : nat -> A -> B) n l,
    length (mapi_aux f n l) = length l.
Proof.
  intros A B f n l. revert n.
  induction l as [|x xs IH]; intros n.
  - reflexivity.
  - simpl. rewrite IH. reflexivity.
Qed.

Lemma mapi_length :
  forall {A B : Type} (f : nat -> A -> B) l,
    length (mapi f l) = length l.
Proof.
  intros A B f l. unfold mapi. apply mapi_aux_length.
Qed.

Definition context_to_kv (message_tokens : list nat) : KVCache :=
  mapi (fun i t =>
    mkKVEntry i 0 500 (length message_tokens - i) 4
  ) message_tokens.

Lemma context_to_kv_length :
  forall ctx, length (context_to_kv ctx) = length ctx.
Proof.
  intros ctx. unfold context_to_kv. apply mapi_length.
Qed.

(** JFC could use attention-aware eviction for context management. *)
Definition jfc_context_eviction (context : list nat) (budget : nat) : list nat :=
  let kv := context_to_kv context in
  let retained := evict_by_attention kv budget in
  map kv_position retained.

(** Theorem: JFC context eviction respects the budget exactly when the context
    is at least as long as the budget.

    [CORRECTION] The original [jfc_preserves_recent] concluded [True].  This is
    the real end-to-end bound: the retained-position list has length exactly
    [budget] once the context fills it (and never exceeds it in general). *)
Theorem jfc_eviction_respects_budget :
  forall context budget,
    length context >= budget ->
    length (jfc_context_eviction context budget) = budget.
Proof.
  intros context budget Hlen.
  unfold jfc_context_eviction.
  rewrite length_map.
  unfold evict_by_attention.
  rewrite length_firstn, context_to_kv_length.
  lia.
Qed.

Theorem jfc_eviction_bounded :
  forall context budget,
    length (jfc_context_eviction context budget) <= budget.
Proof.
  intros context budget.
  unfold jfc_context_eviction.
  rewrite length_map.
  unfold evict_by_attention.
  rewrite length_firstn. lia.
Qed.

(** ** Memory Efficiency Bounds *)

(** Standard KV cache memory: 2 * num_layers * seq_len * hidden_dim * bytes. *)
Definition kv_memory (num_layers seq_len hidden_dim : nat) : nat :=
  2 * num_layers * seq_len * hidden_dim * 2.  (* FP16 = 2 bytes *)

Definition compressed_kv_memory (num_layers seq_len hidden_dim compression_ratio : nat) : nat :=
  kv_memory num_layers seq_len hidden_dim * compression_ratio / 100.

(** Theorem: compression achieves predictable savings (ratio <= 100 => no growth). *)
Theorem compression_savings :
  forall num_layers seq_len hidden_dim ratio,
    ratio <= 100 ->
    compressed_kv_memory num_layers seq_len hidden_dim ratio <=
    kv_memory num_layers seq_len hidden_dim.
Proof.
  intros num_layers seq_len hidden_dim ratio Hratio.
  unfold compressed_kv_memory.
  apply Nat.div_le_upper_bound.
  - lia.
  - nia.
Qed.
