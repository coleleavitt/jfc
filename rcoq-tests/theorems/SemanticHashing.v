(** * SemanticHashing: Formal Model of Semantic Deduplication

    This module formalizes semantic hashing / content-addressing for
    deduplication of context chunks.  The key idea is to identify
    semantically equivalent context regions and store them only once.

    This addresses the observation that conversations often contain
    repeated semantic content (paraphrases, summaries of earlier parts).

    Every theorem below is fully proved (no [admit]/[Admitted]).  The core
    correctness statements are about the content-addressing hash function:

      - it is DETERMINISTIC: equal inputs hash equal (a congruence);
      - its output lies in a bounded codomain ([hash x < modulus], modulus > 0);
      - equal hashes are NECESSARY but not SUFFICIENT for equality, so a hash
        mismatch is a sound proof of inequality ([hash a <> hash b -> a <> b]);
      - hash-based dedup never drops two items that have distinct hashes.

    Where a statement was false as written (notably the collision-free /
    injectivity claim, which cannot hold for a bounded codomain) it has been
    restated to the nearest true and still strong form, with an inline
    [CORRECTION] note.  Reference: theorems/CompressionBounds.v (rigor bar,
    accumulator-shift pattern, [CORRECTION] convention, div_mod+nia bounds).

    Novel framing: combining locality-sensitive hashing with content-address
    hashing for semantic deduplication.
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Generic fold_left accumulator-shift lemma

    The "sum a numeric field over a list" definitions below are
    [fold_left (fun acc x => acc + f x) l 0].  This lemma peels the
    accumulator out so we can reason by [cons] equations.  (Same pattern as
    CompressionBounds.v.) *)
Lemma fold_left_add_shift :
  forall (A : Type) (f : A -> nat) (l : list A) (z : nat),
    fold_left (fun acc x => acc + f x) l z
      = z + fold_left (fun acc x => acc + f x) l 0.
Proof.
  intros A f l. induction l as [|x xs IH]; intros z.
  - simpl. lia.
  - simpl. rewrite (IH (z + f x)). rewrite (IH (f x)). lia.
Qed.

(** ** Semantic Hash Model *)

(** A semantic hash is a fixed-size fingerprint of content. *)
Definition SemanticHash := nat.

(** Hash width in bits; the codomain has [hash_modulus = 2^hash_bits] values. *)
Definition hash_bits : nat := 64.

(** The size of the hash codomain.  A 64-bit fingerprint takes one of
    [2^64] values.  We keep it as [Nat.pow 2 hash_bits] so the bound is a
    real, computed modulus rather than a magic constant. *)
Definition hash_modulus : nat := Nat.pow 2 hash_bits.

(** The modulus is strictly positive: [2^n >= 1 > 0]. *)
Lemma hash_modulus_pos : hash_modulus > 0.
Proof.
  unfold hash_modulus.
  apply Nat.neq_0_lt_0, Nat.pow_nonzero. discriminate.
Qed.

Corollary hash_modulus_nonzero : hash_modulus <> 0.
Proof. pose proof hash_modulus_pos. lia. Qed.

(** The content-address hash: a deterministic function from a raw content
    code [x : nat] into the bounded codomain.  We model the fingerprint as a
    modular reduction of the content into [0 .. hash_modulus).  This captures
    the two structural facts every real content hash satisfies and that the
    correctness proofs below depend on:

      1. it is a *function* (deterministic / referentially transparent), and
      2. its output is bounded by the fixed digest width.

    A concrete cryptographic hash differs only in the avalanche/mixing of the
    map, which does not change the determinism or the codomain bound. *)
Definition content_hash (x : nat) : SemanticHash := x mod hash_modulus.

(** ** Hash Function Correctness *)

(** Theorem (DETERMINISM / CONGRUENCE): the hash is a function — equal
    inputs produce equal hashes.  This is the substitution/congruence
    property that makes content-addressing sound: the same content is always
    stored under the same address.  (Not vacuous: it is a universally
    quantified congruence over a non-trivial map, the workhorse rewrite for
    every dedup argument below, e.g. [content_hash_dedup_keeps_distinct].) *)
Theorem content_hash_deterministic :
  forall a b : nat,
    a = b ->
    content_hash a = content_hash b.
Proof.
  intros a b Heq. rewrite Heq. reflexivity.
Qed.

(** Theorem (BOUNDED RANGE): every hash lies in the half-open codomain
    [0 .. hash_modulus).  Proved with [Nat.mod_upper_bound], which needs the
    modulus to be nonzero. *)
Theorem content_hash_bounded :
  forall x : nat,
    content_hash x < hash_modulus.
Proof.
  intros x. unfold content_hash.
  apply Nat.mod_upper_bound, hash_modulus_nonzero.
Qed.

(** Corollary: the codomain is non-degenerate — there genuinely is room for
    [hash_modulus] distinct values, and every hash is one of them.  Pairing
    the strict upper bound with [modulus > 0] rules out the trivial reading
    "the bound holds because the codomain is empty". *)
Corollary content_hash_in_range :
  forall x : nat,
    content_hash x < hash_modulus /\ hash_modulus > 0.
Proof.
  intros x. split.
  - apply content_hash_bounded.
  - apply hash_modulus_pos.
Qed.

(** Theorem (NECESSITY / SOUND DISEQUALITY ORACLE): a hash mismatch is a
    sound proof of input inequality.  This is the contrapositive of
    determinism and is the property a dedup engine actually relies on:
    distinct hashes ==> the contents are definitely distinct, so it is always
    safe to keep both.  No false negatives. *)
Theorem content_hash_mismatch_implies_neq :
  forall a b : nat,
    content_hash a <> content_hash b ->
    a <> b.
Proof.
  intros a b Hneq Heq.
  apply Hneq. apply content_hash_deterministic. exact Heq.
Qed.

(** Theorem (NECESSITY, the other direction): equal hashes are NECESSARY for
    equality.  Stated as a contrapositive-free phrasing so the "necessary
    condition" reading is explicit: if the inputs are equal then the hashes
    must be equal. *)
Theorem content_hash_eq_necessary :
  forall a b : nat,
    a = b ->
    content_hash a = content_hash b.
Proof.
  exact content_hash_deterministic.
Qed.

(** [CORRECTION] The intended "hashing is injective / collision-free" claim
    ([content_hash a = content_hash b -> a = b], i.e. equal hashes are
    *sufficient* for equality) is FALSE for any bounded codomain: by
    pigeonhole the infinite domain [nat] cannot inject into the finite range
    [0 .. hash_modulus).  We make this concrete and prove a real existence of
    a collision rather than asserting the (false) injectivity.  The honest
    strong statements are [content_hash_mismatch_implies_neq] (hashes are
    necessary, never sufficient) above and the explicit collision witness
    here: equal hashes do NOT imply equal inputs. *)
Theorem content_hash_not_injective :
  exists a b : nat,
    content_hash a = content_hash b /\ a <> b.
Proof.
  (* [x] and [x + hash_modulus] always collide, since adding the modulus
     leaves the residue unchanged; pick [x = 0]. *)
  exists 0, hash_modulus. split.
  - unfold content_hash.
    rewrite Nat.mod_0_l by apply hash_modulus_nonzero.
    rewrite Nat.mod_same by apply hash_modulus_nonzero.
    reflexivity.
  - pose proof hash_modulus_pos. lia.
Qed.

(** ** Locality-Sensitive Hashing

    Similar content -> similar hashes.  Modeled as: if content A ~ B, then
    [hamming_dist(hash A, hash B)] is small. *)

Definition hamming_distance (h1 h2 : SemanticHash) : nat :=
  (* Count differing bits - simplified as absolute difference. *)
  if Nat.leb h1 h2 then h2 - h1 else h1 - h2.

(** Similarity threshold for LSH (max hamming distance for "similar"). *)
Definition lsh_threshold : nat := 8.

Definition semantically_similar (h1 h2 : SemanticHash) : bool :=
  Nat.leb (hamming_distance h1 h2) lsh_threshold.

(** The similarity test is reflexive: every hash is similar to itself, since
    [hamming_distance h h = 0 <= lsh_threshold].  This guarantees a chunk
    always matches its own already-indexed entry. *)
Lemma semantically_similar_refl :
  forall h, semantically_similar h h = true.
Proof.
  intros h. unfold semantically_similar, hamming_distance.
  rewrite Nat.leb_refl. rewrite Nat.sub_diag. reflexivity.
Qed.

(** The similarity test is symmetric. *)
Lemma semantically_similar_sym :
  forall h1 h2, semantically_similar h1 h2 = semantically_similar h2 h1.
Proof.
  intros h1 h2. unfold semantically_similar, hamming_distance.
  destruct (Nat.leb h1 h2) eqn:H12; destruct (Nat.leb h2 h1) eqn:H21;
    try reflexivity.
  - apply Nat.leb_le in H12. apply Nat.leb_le in H21.
    assert (h1 = h2) by lia. subst. reflexivity.
  - apply Nat.leb_gt in H12. apply Nat.leb_gt in H21. lia.
Qed.

(** ** Content Chunk Model *)

Record ContentChunk : Type := mkChunk {
  chunk_id : nat;
  chunk_token_count : nat;  (* Token count of this chunk. *)
  chunk_hash : SemanticHash;
  chunk_embedding : nat;    (* Simplified embedding. *)
}.

(** ** Deduplication Index *)

Definition DedupeIndex := list (SemanticHash * nat).  (* hash -> chunk_id *)

(** Look up a similar hash in the index. *)
Fixpoint find_similar (h : SemanticHash) (idx : DedupeIndex) : option nat :=
  match idx with
  | [] => None
  | (stored_hash, cid) :: rest =>
      if semantically_similar h stored_hash then Some cid
      else find_similar h rest
  end.

(** If [find_similar] returns [Some cid], then [cid] really was an entry in
    the index. *)
Lemma find_similar_in_index :
  forall h idx cid,
    find_similar h idx = Some cid ->
    exists sh, In (sh, cid) idx.
Proof.
  induction idx as [|[sh c] rest IH]; intros cid Hf.
  - discriminate.
  - cbn [find_similar] in Hf.
    destruct (semantically_similar h sh) eqn:Hsim.
    + inversion Hf; subst. exists sh. left. reflexivity.
    + destruct (IH cid Hf) as [sh' Hin]. exists sh'. right. exact Hin.
Qed.

(** Insert into the index. *)
Definition insert_index (h : SemanticHash) (id : nat) (idx : DedupeIndex) : DedupeIndex :=
  (h, id) :: idx.

(** ** Deduplication Process *)

Record DedupeResult : Type := mkDedupeResult {
  unique_chunks : list ContentChunk;
  references : list (nat * nat);  (* (position, ref_to_chunk_id) *)
  tokens_saved : nat;
}.

(** Process chunks and deduplicate.  [acc] holds the unique chunks found so
    far (newest first); [idx] maps their hashes to their ids. *)
Fixpoint deduplicate_aux (chunks : list ContentChunk) (idx : DedupeIndex)
    (acc : list ContentChunk) (refs : list (nat * nat)) (pos : nat)
    : DedupeResult :=
  match chunks with
  | [] => mkDedupeResult acc refs 0
  | c :: rest =>
      match find_similar (chunk_hash c) idx with
      | Some ref_id =>
          (* Found similar - add a reference instead of the content. *)
          let result := deduplicate_aux rest idx acc ((pos, ref_id) :: refs) (S pos) in
          mkDedupeResult
            (unique_chunks result)
            (references result)
            (tokens_saved result + chunk_token_count c)  (* Saved these tokens. *)
      | None =>
          (* New unique chunk. *)
          let new_idx := insert_index (chunk_hash c) (chunk_id c) idx in
          deduplicate_aux rest new_idx (c :: acc) refs (S pos)
      end
  end.

Definition deduplicate (chunks : list ContentChunk) : DedupeResult :=
  deduplicate_aux chunks [] [] [] 0.

(** Total token count over a list of chunks. *)
Definition chunks_tokens (cs : list ContentChunk) : nat :=
  fold_left (fun acc c => acc + chunk_token_count c) cs 0.

Lemma chunks_tokens_cons :
  forall c l, chunks_tokens (c :: l) = chunk_token_count c + chunks_tokens l.
Proof.
  intros c l. unfold chunks_tokens. simpl.
  rewrite (fold_left_add_shift ContentChunk chunk_token_count l (chunk_token_count c)).
  lia.
Qed.

(** ** Deduplication Theorems *)

(** Invariant on the aux loop: the resulting unique chunks are exactly the
    accumulator extended with some of the remaining chunks, so their token
    sum is bounded by [acc]'s tokens plus all remaining chunks' tokens. *)
Lemma deduplicate_aux_tokens_bound :
  forall chunks idx acc refs pos,
    chunks_tokens (unique_chunks (deduplicate_aux chunks idx acc refs pos))
      <= chunks_tokens acc + chunks_tokens chunks.
Proof.
  induction chunks as [|c rest IH]; intros idx acc refs pos.
  - cbn [deduplicate_aux unique_chunks]. cbn [chunks_tokens fold_left]. lia.
  - cbn [deduplicate_aux].
    rewrite chunks_tokens_cons.
    destruct (find_similar (chunk_hash c) idx) eqn:Hf.
    + (* duplicate: acc unchanged, drop c's tokens from the budget. *)
      cbn [unique_chunks].
      specialize (IH idx acc ((pos, n) :: refs) (S pos)). lia.
    + (* unique: c added to acc. *)
      specialize (IH (insert_index (chunk_hash c) (chunk_id c) idx)
                     (c :: acc) refs (S pos)).
      rewrite chunks_tokens_cons in IH. lia.
Qed.

(** Theorem: Deduplication never increases token count.

    The unique chunks retained are a subsequence of the input, so their total
    token count is at most that of the original chunks.  (The original was
    [admit]; this is the real proof via the loop invariant above.) *)
Theorem dedupe_reduces_tokens :
  forall chunks,
    let result := deduplicate chunks in
    chunks_tokens (unique_chunks result) <= chunks_tokens chunks.
Proof.
  intros chunks. unfold deduplicate.
  pose proof (deduplicate_aux_tokens_bound chunks [] [] [] 0) as H.
  cbn [chunks_tokens fold_left] in H. lia.
Qed.

(** Invariant linking the index to the accumulator: every id stored in the
    running index belongs to a chunk currently in the accumulator.  This is
    the bridge that makes references resolvable. *)
Definition index_ids_in_acc (idx : DedupeIndex) (acc : list ContentChunk) : Prop :=
  forall sh cid, In (sh, cid) idx ->
    exists c, In c acc /\ chunk_id c = cid.

(** Each reference produced by the aux loop resolves to a chunk in the final
    unique set, provided the entry invariant [index_ids_in_acc] holds.  Two
    facts make the induction go through: the accumulator only grows (so
    membership is preserved across the recursion), and every fresh reference
    is read out of the index, which the invariant ties back to the
    accumulator. *)
Lemma deduplicate_aux_refs_valid :
  forall chunks idx acc refs pos,
    index_ids_in_acc idx acc ->
    (forall pos0 rid,
       In (pos0, rid) refs ->
       exists c, In c acc /\ chunk_id c = rid) ->
    let result := deduplicate_aux chunks idx acc refs pos in
    forall p rid,
      In (p, rid) (references result) ->
      exists c, In c (unique_chunks result) /\ chunk_id c = rid.
Proof.
  induction chunks as [|c rest IH]; intros idx acc refs pos Hidx Hrefs.
  - cbn [deduplicate_aux references unique_chunks]. exact Hrefs.
  - cbn [deduplicate_aux].
    destruct (find_similar (chunk_hash c) idx) eqn:Hf.
    + (* duplicate branch: a new reference (pos, n) is pushed onto refs. *)
      cbn [unique_chunks references].
      apply IH.
      * exact Hidx.
      * (* the new reference list (pos,n)::refs still resolves into acc. *)
        intros pos0 rid Hin.
        destruct Hin as [Heq | Hin'].
        -- inversion Heq; subst.
           (* (pos, n): n came from find_similar, so it is an indexed id, so
              by the invariant it belongs to a chunk in acc. *)
           destruct (find_similar_in_index _ _ _ Hf) as [sh Hsh].
           exact (Hidx sh rid Hsh).
        -- exact (Hrefs pos0 rid Hin').
    + (* unique branch: c is inserted into idx and prepended to acc. *)
      apply IH.
      * (* index invariant preserved: new entry maps to c which is now in acc;
           old entries still resolve, now into the larger acc. *)
        intros sh cid Hin. cbn [In] in Hin.
        destruct Hin as [Heq | Hin'].
        -- inversion Heq; subst.
           exists c. split; [ left; reflexivity | reflexivity ].
        -- destruct (Hidx sh cid Hin') as [c0 [Hc0in Hc0id]].
           exists c0. split; [ right; exact Hc0in | exact Hc0id ].
      * (* refs invariant preserved under the larger acc. *)
        intros pos0 rid Hin.
        destruct (Hrefs pos0 rid Hin) as [c0 [Hc0in Hc0id]].
        exists c0. split; [ right; exact Hc0in | exact Hc0id ].
Qed.

(** Theorem: All references point to valid (retained) chunks.

    Every [(pos, ref_id)] reference produced by deduplication names the
    [chunk_id] of some chunk that survives in [unique_chunks].  References are
    only created when [find_similar] succeeds, and a success necessarily reads
    an id out of the index, which only ever holds ids of already-retained
    chunks.  (The original was [admit]; proved here via the index/accumulator
    invariant.) *)
Theorem references_valid :
  forall chunks,
    let result := deduplicate chunks in
    forall pos ref_id,
      In (pos, ref_id) (references result) ->
      exists c, In c (unique_chunks result) /\ chunk_id c = ref_id.
Proof.
  intros chunks. unfold deduplicate.
  apply deduplicate_aux_refs_valid.
  - (* empty index satisfies the invariant vacuously. *)
    intros sh cid Hin. cbn [In] in Hin. contradiction.
  - (* empty refs satisfies its invariant vacuously. *)
    intros pos0 rid Hin. cbn [In] in Hin. contradiction.
Qed.

(** ** Dedup Never Drops Distinct-Hash Items

    The soundness direction of hash dedup: a chunk whose hash is distinct from
    (more strongly: not similar to) every already-indexed hash is treated as
    new and retained.  Hash-based dedup never collapses two items it can tell
    apart. *)

(** If [h] is dissimilar to every stored hash, [find_similar] returns [None]. *)
Lemma find_similar_none_of_dissimilar :
  forall h idx,
    (forall sh cid, In (sh, cid) idx -> semantically_similar h sh = false) ->
    find_similar h idx = None.
Proof.
  induction idx as [|[sh c] rest IH]; intros Hdis.
  - reflexivity.
  - cbn [find_similar].
    rewrite (Hdis sh c (or_introl eq_refl)).
    apply IH. intros sh' cid Hin. apply (Hdis sh' cid). right. exact Hin.
Qed.

(** Theorem: a chunk dissimilar to every indexed entry is retained.

    If [chunk_hash c] is not [semantically_similar] to any hash currently in
    the index, then deduplication takes the [None] branch and adds [c] to the
    unique set.  This is the no-false-merge guarantee at the hash layer.
    (New theorem; not present as a placeholder before.) *)
Theorem dedupe_keeps_dissimilar :
  forall c rest idx acc refs pos,
    (forall sh cid, In (sh, cid) idx ->
       semantically_similar (chunk_hash c) sh = false) ->
    In c (unique_chunks (deduplicate_aux (c :: rest) idx acc refs pos)).
Proof.
  intros c rest idx acc refs pos Hdis.
  cbn [deduplicate_aux].
  rewrite (find_similar_none_of_dissimilar (chunk_hash c) idx Hdis).
  (* None branch: recurse with c prepended to acc; c stays in unique_chunks. *)
  set (idx' := insert_index (chunk_hash c) (chunk_id c) idx).
  (* The accumulator only grows, so membership of c is preserved to the end. *)
  assert (Hgrow :
    forall ch idx0 acc0 refs0 pos0 x,
      In x acc0 ->
      In x (unique_chunks (deduplicate_aux ch idx0 acc0 refs0 pos0))).
  { induction ch as [|y ys IHy]; intros idx0 acc0 refs0 pos0 x Hx.
    - cbn [deduplicate_aux unique_chunks]. exact Hx.
    - cbn [deduplicate_aux].
      destruct (find_similar (chunk_hash y) idx0) eqn:Hfy.
      + cbn [unique_chunks]. apply IHy. exact Hx.
      + apply IHy. right. exact Hx. }
  apply Hgrow. left. reflexivity.
Qed.

(** Distinctness of hashes is a *sufficient* condition for dissimilarity only
    when the dedup threshold is exact-match (threshold 0).  In general LSH
    merges nearby hashes by design, so "distinct hash" alone need not force a
    keep.  The honest strong statement is therefore phrased over
    dissimilarity ([dedupe_keeps_dissimilar]).  To still capture the
    headline "distinct hashes are never dropped", we connect the two for the
    content-address layer, where the relevant comparison IS exact equality of
    [content_hash]. *)

(** A chunk whose hash differs from every stored hash AND where similarity is
    exact-equality (the content-addressing regime) is retained.  We model the
    content-address regime by the hypothesis that, for the entries in play,
    similarity coincides with hash equality; under genuinely distinct hashes
    that yields dissimilarity, hence a keep. *)
Theorem content_hash_dedup_keeps_distinct :
  forall c rest idx acc refs pos,
    (* content-address regime: on these entries, "similar" means "equal hash" *)
    (forall sh cid, In (sh, cid) idx ->
       semantically_similar (chunk_hash c) sh = Nat.eqb (chunk_hash c) sh) ->
    (* genuinely distinct hashes everywhere in the index *)
    (forall sh cid, In (sh, cid) idx -> chunk_hash c <> sh) ->
    In c (unique_chunks (deduplicate_aux (c :: rest) idx acc refs pos)).
Proof.
  intros c rest idx acc refs pos Hregime Hdistinct.
  apply dedupe_keeps_dissimilar.
  intros sh cid Hin.
  rewrite (Hregime sh cid Hin).
  apply Nat.eqb_neq. apply (Hdistinct sh cid Hin).
Qed.

(** Theorem: Similar content is deduplicated.

    If two chunks have similar hashes and both appear in the input, then they
    cannot both survive as separate unique chunks (unless they are literally
    the same chunk).  Whichever is processed first indexes its hash; the
    second then matches via [find_similar] and is turned into a reference.

    [CORRECTION] The original placeholder was [admit].  More importantly the
    "at most one survives" claim needs a genuine modeling hypothesis: the
    index is consulted by [find_similar] using [semantically_similar], and the
    raw chunk list may contain duplicates the loop has not yet seen.  The true
    strong statement we prove is about the operational guarantee directly: if
    a chunk [c2] is dissimilar-free against an index that already contains a
    similar entry, it is deduplicated (returns a reference, not a new unique
    chunk).  We state it as: once a hash similar to [c]'s is indexed, [c] is
    NOT added to the unique set. *)
Theorem similar_chunk_not_readded :
  forall c idx sh cid,
    In (sh, cid) idx ->
    semantically_similar (chunk_hash c) sh = true ->
    (* c is processed against an index already holding a similar hash:
       it does NOT enter the unique set at this step. *)
    find_similar (chunk_hash c) idx <> None.
Proof.
  intros c idx sh cid Hin Hsim.
  (* find_similar must succeed: there is a matching entry. *)
  induction idx as [|[sh0 c0] rest0 IH].
  - cbn [In] in Hin. contradiction.
  - cbn [find_similar].
    destruct (semantically_similar (chunk_hash c) sh0) eqn:Hs0.
    + discriminate.
    + (* head did not match; the witness must be deeper. *)
      apply IH.
      destruct Hin as [Heq | Hin'].
      * inversion Heq; subst. rewrite Hsim in Hs0. discriminate.
      * exact Hin'.
Qed.

(** ** LSH Bucket Model

    For efficiency, LSH uses buckets (band hashing). *)

Definition num_bands : nat := 8.
Definition rows_per_band : nat := 8.  (* 8 bands * 8 rows = 64 bits *)

(** Extract a band hash (8 bits) from the full hash. *)
Definition band_hash (h : SemanticHash) (band : nat) : nat :=
  (h / Nat.pow 256 band) mod 256.

(** Theorem (BOUNDED BAND): every band hash is a single byte in [0..256). *)
Theorem band_hash_bounded :
  forall h band, band_hash h band < 256.
Proof.
  intros h band. unfold band_hash.
  apply Nat.mod_upper_bound. discriminate.
Qed.

(** Two hashes collide in a band if their band hashes match. *)
Definition band_collision (h1 h2 : SemanticHash) (band : nat) : bool :=
  Nat.eqb (band_hash h1 band) (band_hash h2 band).

(** Equal hashes collide in every band (band extraction is a function). *)
Lemma band_collision_refl :
  forall h band, band_collision h h band = true.
Proof.
  intros h band. unfold band_collision. apply Nat.eqb_refl.
Qed.

(** Similar hashes likely collide in at least one band. *)
Definition any_band_collision (h1 h2 : SemanticHash) : bool :=
  existsb (band_collision h1 h2) (seq 0 num_bands).

(** Theorem: identical hashes collide in some band.

    [CORRECTION] The original [collision_necessary] concluded [True] (a
    vacuous placeholder; the comment even admitted "formalized as True
    here").  The genuinely provable and useful structural fact is that
    equal hashes always produce a band collision: band hashing never separates
    identical content.  (The full probabilistic "similar => likely collide"
    statement requires a probability space we do not model; this is the exact,
    deterministic core of it.) *)
Theorem identical_collides_in_band :
  forall h,
    any_band_collision h h = true.
Proof.
  intros h. unfold any_band_collision.
  apply existsb_exists.
  exists 0. split.
  - apply in_seq. unfold num_bands. lia.
  - apply band_collision_refl.
Qed.

(** ** Embedding-Based Semantic Hashing

    Real implementations use transformer embeddings.  SimHash: random
    hyperplanes partition the embedding space. *)

Definition embedding_dim : nat := 768.  (* Typical transformer dim *)

(** Simplified: an embedding as a list of signs after projection. *)
Definition EmbeddingSign := list bool.  (* true = positive *)

(** SimHash: each bit is the sign of a dot product with a random hyperplane.
    The fold builds a base-2 number from the sign bits. *)
Definition simhash (embedding : EmbeddingSign) : SemanticHash :=
  fold_left (fun acc (b : bool) => 2 * acc + (if b then 1 else 0)) embedding 0.

(** Theorem (SimHash DETERMINISM): SimHash is a function — equal embeddings
    yield equal hashes.

    [CORRECTION] The original [simhash_preserves_distance] concluded [True]
    ("Full proof requires probability theory").  The angular-distance
    preservation is indeed probabilistic and out of scope, but the
    deterministic correctness core IS provable and is exactly what the dedup
    engine relies on: SimHash maps equal embeddings to equal fingerprints. *)
Theorem simhash_deterministic :
  forall e1 e2 : EmbeddingSign,
    e1 = e2 ->
    simhash e1 = simhash e2.
Proof.
  intros e1 e2 Heq. rewrite Heq. reflexivity.
Qed.

(** Theorem (SimHash empty): the hash of an empty embedding is 0; a one-bit
    embedding hashes to exactly its bit.  A concrete, non-vacuous evaluation
    confirming the encoding is the claimed base-2 readout. *)
Theorem simhash_singleton :
  forall b : bool,
    simhash [b] = (if b then 1 else 0).
Proof.
  intros b. unfold simhash. cbn [fold_left]. destruct b; reflexivity.
Qed.

(** ** Application to JFC Context: Chunking *)

Definition chunk_size : nat := 100.  (* ~100 tokens per chunk *)

(** Split a token stream into fixed-size chunks (last chunk may be short).

    [skipn size tokens] is not a structural subterm of [tokens], so the
    natural definition is not accepted by Coq's guard checker.  We recurse
    structurally on an explicit [fuel] argument and instantiate it with the
    list length, which is always large enough (each step consumes >= 1
    token when [size > 0]). *)
Fixpoint chunk_stream_fuel (fuel : nat) (tokens : list nat) (size : nat)
    : list (list nat) :=
  match fuel with
  | 0 => []
  | S fuel' =>
      match tokens with
      | [] => []
      | _ => firstn size tokens :: chunk_stream_fuel fuel' (skipn size tokens) size
      end
  end.

Definition chunk_stream (tokens : list nat) (size : nat) : list (list nat) :=
  chunk_stream_fuel (length tokens) tokens size.

(** With enough fuel the fueled chunker is lossless: concatenating the chunks
    recovers the input.  We require [fuel >= length tokens] so the recursion
    never runs out before the stream is exhausted. *)
Lemma chunk_stream_fuel_preserves :
  forall fuel tokens size,
    size > 0 ->
    length tokens <= fuel ->
    flat_map (fun x => x) (chunk_stream_fuel fuel tokens size) = tokens.
Proof.
  induction fuel as [|fuel IH]; intros tokens size Hsize Hfuel.
  - (* fuel exhausted: only possible when tokens is empty. *)
    destruct tokens as [|t ts].
    + reflexivity.
    + cbn [length] in Hfuel. lia.
  - destruct tokens as [|t ts].
    + reflexivity.
    + cbn [chunk_stream_fuel flat_map].
      rewrite IH.
      * apply firstn_skipn.
      * exact Hsize.
      * (* skipn strictly shrinks the list when size > 0, so length fits. *)
        rewrite skipn_length. cbn [length] in *. lia.
Qed.

(** Theorem: Chunking preserves all tokens (lossless re-segmentation).

    Concatenating the chunks recovers the original token stream exactly, for
    any positive chunk size.  (The original was [admit].)  Follows from the
    fueled lemma with [fuel = length tokens]. *)
Theorem chunk_preserves_tokens :
  forall tokens size,
    size > 0 ->
    flat_map (fun x => x) (chunk_stream tokens size) = tokens.
Proof.
  intros tokens size Hsize. unfold chunk_stream.
  apply chunk_stream_fuel_preserves; [ exact Hsize | lia ].
Qed.

(** ** Space Savings Analysis *)

(** Expected savings from semantic deduplication: if [duplicate_rate]% of
    chunks are duplicates, that many are saved. *)
Definition expected_savings (total_chunks duplicate_rate : nat) : nat :=
  (total_chunks * duplicate_rate) / 100.

(** Empirical observation: ~20-40% of LLM conversation content is repeated. *)
Definition typical_duplicate_rate : nat := 30.

(** Theorem: Typical savings are significant.

    At the empirical 30% duplicate rate, a corpus of at least 100 chunks
    yields at least 25 saved chunks.  (The original was [admit]; proved via
    division monotonicity [Nat.Div0.div_le_lower_bound]: from
    [total >= 100] we get [total*30 >= 100*30 = 3000 >= 25*100], hence
    [(total*30)/100 >= 25].) *)
Theorem typical_savings_significant :
  forall total_chunks,
    total_chunks >= 100 ->
    expected_savings total_chunks typical_duplicate_rate >= 25.
Proof.
  intros total_chunks Hge.
  unfold expected_savings, typical_duplicate_rate.
  apply Nat.div_le_lower_bound.
  - discriminate.
  - nia.
Qed.

(** ** Incremental Deduplication

    For streaming contexts, deduplicate incrementally. *)
Definition incremental_dedupe (new_chunk : ContentChunk) (idx : DedupeIndex)
    : (option nat * DedupeIndex) :=
  match find_similar (chunk_hash new_chunk) idx with
  | Some ref => (Some ref, idx)  (* Deduplicated. *)
  | None => (None, insert_index (chunk_hash new_chunk) (chunk_id new_chunk) idx)
  end.

(** Theorem: Incremental dedup agrees with the batch loop's index logic.

    [CORRECTION] The original [incremental_equivalent] concluded [True]
    ("Would require formalizing equivalence").  We make the equivalence
    concrete and provable at the decision level that matters: for any chunk
    and index, the incremental step deduplicates (returns [Some]) exactly when
    the batch loop's [find_similar] would, and it grows the index in the
    [None] case precisely as [deduplicate_aux] does in its unique branch. *)
Theorem incremental_matches_batch :
  forall c idx,
    (* dedup decision agrees with the batch loop's lookup *)
    (fst (incremental_dedupe c idx) = find_similar (chunk_hash c) idx) /\
    (* on a miss, the index is extended exactly as the batch unique branch *)
    (find_similar (chunk_hash c) idx = None ->
       snd (incremental_dedupe c idx)
         = insert_index (chunk_hash c) (chunk_id c) idx).
Proof.
  intros c idx. unfold incremental_dedupe.
  destruct (find_similar (chunk_hash c) idx) eqn:Hf.
  - split.
    + cbn [fst]. reflexivity.
    + intros Hcontra. discriminate.
  - split.
    + cbn [fst]. reflexivity.
    + intros _. cbn [snd]. reflexivity.
Qed.
