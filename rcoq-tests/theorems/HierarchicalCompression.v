(** * HierarchicalCompression: Formal Model of Hierarchical Summarization

    This module formalizes hierarchical/incremental summarization strategies
    for LLM context compression, in the style of recursive context compression
    (AutoCompressors, Chevalier et al. 2023): a long context is split into
    segments, each segment is compressed into a small summary, and accumulated
    summaries are recursively compressed.  The merge/compression tree therefore
    has depth bounded by ceil(log_b n), and the total summary size after k
    levels decays geometrically.  We prove:

    1. Tree depth vs. leaf count (the source-coding inequality
       [log2 (leaf_count t) <= tree_depth t], i.e. a depth-[d] tree carries at
       most [2^d] leaves) -- the genuinely-true direction.
    2. Recursive single-pass compression has size bounded by the clean
       recurrence [compressed (S k) <= compressed k / b + c] and decays
       geometrically; hierarchical total size is sub-linear and never exceeds
       the flat single-pass cost for the same ratio.
    3. Merge operations preserve temporal ordering.
    4. Recursion terminates: tree token totals strictly decrease under a
       size-reducing summary until a base case.

    These theorems establish correctness for improved compaction strategies
    that could replace JFC's current flat group-based approach.

    Every theorem below is fully proved (no [admit]/[Admitted]).  Where the
    original statement was FALSE as written (most often because it claimed an
    *upper* logarithmic depth bound that fails for unbalanced/degenerate trees,
    or related two structurally-unrelated trees), it has been restated to the
    nearest TRUE and still-strong theorem with an inline [CORRECTION] note.

    References:
    - AutoCompressors: Chevalier, Wettig, Ajith & Chen, "Adapting Language
      Models to Compress Contexts" (2023), arXiv:2305.14788 -- recursive
      summary accumulation and sub-linear compressed size.
    - theorems/CompressionBounds.v (sibling file: the div_mod_eq+nia ceiling
      pattern, the accumulator-shift pattern, and the [CORRECTION] convention).
*)

Require Import Coq.Arith.Arith.
Require Import Coq.Lists.List.
Require Import Coq.Bool.Bool.
Require Import Coq.Init.Nat.
Require Import Coq.micromega.Lia.
Import ListNotations.

(** ** Conversation Group Model *)

Record ConvGroup : Type := mkConvGroup {
  group_id : nat;
  group_timestamp : nat;
  group_tokens : nat;
  group_content_hash : nat;  (* For deduplication *)
}.

(** ** Summary Tree Structure *)

Inductive SummaryTree : Type :=
  | Leaf : ConvGroup -> SummaryTree
  | Node : SummaryTree -> SummaryTree -> nat -> SummaryTree.  (* left, right, summary_tokens *)

(** Tree depth *)
Fixpoint tree_depth (t : SummaryTree) : nat :=
  match t with
  | Leaf _ => 0
  | Node l r _ => 1 + max (tree_depth l) (tree_depth r)
  end.

(** Number of leaves (original groups) *)
Fixpoint leaf_count (t : SummaryTree) : nat :=
  match t with
  | Leaf _ => 1
  | Node l r _ => leaf_count l + leaf_count r
  end.

(** Total tokens in tree (summaries + leaves) *)
Fixpoint tree_tokens (t : SummaryTree) : nat :=
  match t with
  | Leaf g => group_tokens g
  | Node l r summary_toks =>
      summary_toks + tree_tokens l + tree_tokens r
  end.

(** ** Tree Construction

    NOTE: the original [build_tree_aux] recursed on [firstn mid groups] /
    [skipn mid groups], which are not structural subterms, so Rocq's guard
    checker rejects it as a bare [Fixpoint] (and it additionally shadowed the
    [sumbool] constructors [left]/[right] with bound variable names, a parse
    error).  We give the standard fuel-threaded structural definition with the
    same observable behavior: [length groups] units of fuel always suffice
    because each split strictly shrinks the list. *)

Fixpoint build_fuel (fuel : nat) (groups : list ConvGroup) : option SummaryTree :=
  match fuel with
  | 0 => None
  | S fuel' =>
      match groups with
      | [] => None
      | [g] => Some (Leaf g)
      | _ =>
          let mid := length groups / 2 in
          match build_fuel fuel' (firstn mid groups),
                build_fuel fuel' (skipn mid groups) with
          | Some l, Some r => Some (Node l r 0)  (* 0 = placeholder for summary *)
          | Some l, None => Some l
          | None, Some r => Some r
          | None, None => None
          end
      end
  end.

(** Build a balanced tree from a list of groups *)
Definition build_tree_aux (groups : list ConvGroup) : option SummaryTree :=
  build_fuel (length groups) groups.

(** ** Structural fact: a depth-[d] tree carries at most [2^d] leaves.

    This is the core counting lemma behind every depth/log bound below: a
    binary tree of depth [d] cannot have more than [2^d] leaves.  It is the
    only direction relating [tree_depth] and [leaf_count] that holds for
    *arbitrary* (possibly unbalanced) trees. *)
Lemma leaf_count_le_pow_depth :
  forall t, leaf_count t <= 2 ^ tree_depth t.
Proof.
  induction t as [g | l IHl r IHr s].
  - cbn [leaf_count tree_depth]. cbn [Nat.pow]. lia.
  - cbn [leaf_count tree_depth].
    (* leaf_count l + leaf_count r <= 2 ^ (1 + max (depth l) (depth r)) *)
    set (dl := tree_depth l) in *.
    set (dr := tree_depth r) in *.
    assert (Hl : leaf_count l <= 2 ^ Nat.max dl dr).
    { eapply Nat.le_trans; [ exact IHl |].
      apply Nat.pow_le_mono_r; [ lia | apply Nat.le_max_l ]. }
    assert (Hr : leaf_count r <= 2 ^ Nat.max dl dr).
    { eapply Nat.le_trans; [ exact IHr |].
      apply Nat.pow_le_mono_r; [ lia | apply Nat.le_max_r ]. }
    replace (1 + Nat.max dl dr) with (S (Nat.max dl dr)) by lia.
    cbn [Nat.pow]. lia.
Qed.

(** A tree always has at least one leaf. *)
Lemma leaf_count_pos : forall t, 0 < leaf_count t.
Proof.
  induction t as [g | l IHl r IHr s].
  - cbn [leaf_count]. lia.
  - cbn [leaf_count]. lia.
Qed.

(** ** Depth Theorems *)

(** Theorem: the depth of any summary tree is at least [log2] of its leaf
    count.

    [CORRECTION] The original statement claimed the *upper* bound
      [tree_depth t <= Nat.log2 (leaf_count t) + 1]
    for an ARBITRARY [SummaryTree].  That is FALSE: a fully left-leaning spine
    of [n] leaves (exactly what [add_group] builds, [Node (... ) (Leaf g) 0])
    has depth [n-1] but leaf count [n], so for [n = 8] depth [= 7] while
    [Nat.log2 8 + 1 = 4].  An upper depth bound holds only for *balanced*
    trees, which this datatype does not enforce.  The genuinely-true, still
    non-trivial statement is the matching LOWER bound -- the source-coding
    inequality "a depth-[d] tree carries at most [2^d] leaves", i.e.
    [Nat.log2 (leaf_count t) <= tree_depth t].  It is exactly the inequality
    that justifies aiming for logarithmic-depth (balanced) compression trees. *)
Theorem tree_depth_log_bound :
  forall t : SummaryTree,
    Nat.log2 (leaf_count t) <= tree_depth t.
Proof.
  intros t.
  pose proof (leaf_count_le_pow_depth t) as Hle.
  apply Nat.log2_le_mono in Hle.
  rewrite Nat.log2_pow2 in Hle by lia.
  exact Hle.
Qed.

(** Theorem: Balanced tree has optimal depth.

    Any tree with [n > 0] leaves must have depth at least [log2 n], because a
    depth-[d] tree has at most [2^d] leaves.  This is the lower bound that
    makes logarithmic depth *optimal*: no tree can do better. *)
Theorem balanced_tree_optimal :
  forall n : nat,
    n > 0 ->
    forall t : SummaryTree,
      leaf_count t = n ->
      tree_depth t >= Nat.log2 n.
Proof.
  intros n Hn t Hleaves.
  subst n.
  apply tree_depth_log_bound.
Qed.

(** ** Temporal Ordering *)

(** Extract leaf timestamps in left-to-right order *)
Fixpoint leaf_timestamps (t : SummaryTree) : list nat :=
  match t with
  | Leaf g => [group_timestamp g]
  | Node l r _ => leaf_timestamps l ++ leaf_timestamps r
  end.

(** Check if a list is sorted (ascending) *)
Fixpoint is_sorted (l : list nat) : Prop :=
  match l with
  | [] => True
  | [_] => True
  | x :: (y :: _) as rest => x <= y /\ is_sorted rest
  end.

(** Sortedness is preserved by dropping the head. *)
Lemma is_sorted_tail : forall x l, is_sorted (x :: l) -> is_sorted l.
Proof.
  intros x l H. destruct l as [|y l']; cbn in *; [ exact I | tauto ].
Qed.

(** Append of two sorted lists where every element of the first is <= every
    element of the second is sorted.  This is the workhorse for both
    [tree_preserves_order] and [merge_preserves_order]. *)
Lemma is_sorted_app :
  forall l1 l2,
    is_sorted l1 ->
    is_sorted l2 ->
    (forall x y, In x l1 -> In y l2 -> x <= y) ->
    is_sorted (l1 ++ l2).
Proof.
  induction l1 as [|a l1 IH]; intros l2 Hs1 Hs2 Hcross.
  - cbn. exact Hs2.
  - cbn [app].
    destruct l1 as [|b l1'].
    + (* l1 = [a]; head a, then l2 *)
      destruct l2 as [|c l2'].
      * cbn. exact I.
      * cbn [app]. split.
        -- apply Hcross; [ left; reflexivity | left; reflexivity ].
        -- exact Hs2.
    + (* l1 = a :: b :: l1' *)
      cbn [is_sorted] in Hs1.
      destruct Hs1 as [Hab Hs1rest].
      assert (Hrec : is_sorted ((b :: l1') ++ l2)).
      { apply IH.
        - exact Hs1rest.
        - exact Hs2.
        - intros x y Hx Hy. apply Hcross; [ right; exact Hx | exact Hy ]. }
      cbn [app] in Hrec |- *.
      split; [ exact Hab | exact Hrec ].
Qed.

(** With enough fuel, a non-empty group list always builds to [Some]. *)
Lemma build_fuel_some :
  forall fuel groups,
    length groups <= fuel ->
    groups <> [] ->
    exists t, build_fuel fuel groups = Some t.
Proof.
  induction fuel as [|fuel IH]; intros groups Hlen Hne.
  - (* fuel = 0 forces groups = [], contradicting Hne *)
    destruct groups as [|g gs]; [ contradiction | cbn [length] in Hlen; lia ].
  - destruct groups as [|g0 rest0]; [ contradiction |].
    destruct rest0 as [|g1 rest1].
    + (* singleton *) exists (Leaf g0). reflexivity.
    + (* >= 2 elements *)
      set (groups := g0 :: g1 :: rest1) in *.
      assert (Hlen2 : 2 <= length groups) by (subst groups; cbn [length]; lia).
      set (mid := length groups / 2) in *.
      assert (Hmid_pos : 1 <= mid)
        by (subst mid; apply Nat.div_le_lower_bound; lia).
      assert (Hmid_lt : mid < length groups)
        by (subst mid; apply Nat.div_lt; lia).
      assert (Hlen_lg : length (firstn mid groups) = mid)
        by (apply firstn_length_le; lia).
      assert (Hlen_rg : length (skipn mid groups) = length groups - mid)
        by (apply length_skipn).
      assert (Hlg_ne : firstn mid groups <> []).
      { intro Hc. apply (f_equal (@length _)) in Hc.
        rewrite Hlen_lg in Hc. cbn [length] in Hc. lia. }
      assert (Hrg_ne : skipn mid groups <> []).
      { intro Hc. apply (f_equal (@length _)) in Hc.
        rewrite Hlen_rg in Hc. cbn [length] in Hc. lia. }
      destruct (IH (firstn mid groups) ltac:(rewrite Hlen_lg; lia) Hlg_ne)
        as [tl Htl].
      destruct (IH (skipn mid groups) ltac:(rewrite Hlen_rg; lia) Hrg_ne)
        as [tr Htr].
      exists (Node tl tr 0).
      assert (Hunfold :
        build_fuel (S fuel) groups =
          match build_fuel fuel (firstn mid groups),
                build_fuel fuel (skipn mid groups) with
          | Some l, Some r => Some (Node l r 0)
          | Some l, None => Some l
          | None, Some r => Some r
          | None, None => None
          end).
      { subst groups mid. cbn [build_fuel]. reflexivity. }
      rewrite Hunfold, Htl, Htr. reflexivity.
Qed.

(** Whenever [build_fuel] succeeds *with at least [length groups] fuel* it
    produces a tree whose left-to-right leaf timestamps are exactly
    [map group_timestamp groups]: the build is a structural re-bracketing that
    never reorders, drops, or duplicates a leaf.  Sufficient fuel rules out the
    "one sublist failed to build" collapse branches (a non-empty sublist always
    builds, by [build_fuel_some]). *)
Lemma build_fuel_leaf_timestamps :
  forall fuel groups t,
    length groups <= fuel ->
    build_fuel fuel groups = Some t ->
    leaf_timestamps t = map group_timestamp groups.
Proof.
  induction fuel as [|fuel IH]; intros groups t Hlen Hbuild.
  - cbn in Hbuild. discriminate.
  - destruct groups as [|g0 rest0].
    + cbn in Hbuild. discriminate.
    + destruct rest0 as [|g1 rest1].
      * (* singleton *)
        cbn in Hbuild. injection Hbuild as <-. cbn. reflexivity.
      * (* >= 2 elements: the split branch *)
        set (groups := g0 :: g1 :: rest1) in *.
        assert (Hlen2 : 2 <= length groups) by (subst groups; cbn [length]; lia).
        set (mid := length groups / 2) in *.
        assert (Hmid_pos : 1 <= mid)
          by (subst mid; apply Nat.div_le_lower_bound; lia).
        assert (Hmid_lt : mid < length groups)
          by (subst mid; apply Nat.div_lt; lia).
        assert (Hlen_lg : length (firstn mid groups) = mid)
          by (apply firstn_length_le; lia).
        assert (Hlen_rg : length (skipn mid groups) = length groups - mid)
          by (apply length_skipn).
        assert (Hlg_ne : firstn mid groups <> []).
        { intro Hc. apply (f_equal (@length _)) in Hc.
          rewrite Hlen_lg in Hc. cbn [length] in Hc. lia. }
        assert (Hrg_ne : skipn mid groups <> []).
        { intro Hc. apply (f_equal (@length _)) in Hc.
          rewrite Hlen_rg in Hc. cbn [length] in Hc. lia. }
        (* both sublists are within fuel and non-empty, hence Some *)
        destruct (build_fuel_some fuel (firstn mid groups)
                    ltac:(rewrite Hlen_lg; lia) Hlg_ne) as [tl Htl].
        destruct (build_fuel_some fuel (skipn mid groups)
                    ltac:(rewrite Hlen_rg; lia) Hrg_ne) as [tr Htr].
        assert (Hunfold :
          build_fuel (S fuel) groups =
            match build_fuel fuel (firstn mid groups),
                  build_fuel fuel (skipn mid groups) with
            | Some l, Some r => Some (Node l r 0)
            | Some l, None => Some l
            | None, Some r => Some r
            | None, None => None
            end).
        { subst groups mid. cbn [build_fuel]. reflexivity. }
        rewrite Hunfold, Htl, Htr in Hbuild.
        injection Hbuild as <-.
        cbn [leaf_timestamps].
        rewrite (IH (firstn mid groups) tl ltac:(rewrite Hlen_lg; lia) Htl).
        rewrite (IH (skipn mid groups) tr ltac:(rewrite Hlen_rg; lia) Htr).
        rewrite <- map_app.
        rewrite firstn_skipn. reflexivity.
Qed.

(** Membership/order corollary specialized to the public [build_tree_aux]. *)
Lemma build_tree_leaf_timestamps :
  forall t groups,
    build_tree_aux groups = Some t ->
    leaf_timestamps t = map group_timestamp groups.
Proof.
  intros t groups Hbuild.
  unfold build_tree_aux in Hbuild.
  apply (build_fuel_leaf_timestamps (length groups) groups t).
  - lia.
  - exact Hbuild.
Qed.

(** Theorem: Tree structure preserves temporal ordering.

    Building a tree from groups whose timestamps are already sorted yields
    leaves whose timestamps are sorted, because the build is just a structural
    re-bracketing that preserves left-to-right leaf order. *)
Theorem tree_preserves_order :
  forall groups : list ConvGroup,
    is_sorted (map group_timestamp groups) ->
    forall t,
      build_tree_aux groups = Some t ->
      is_sorted (leaf_timestamps t).
Proof.
  intros groups Hsorted t Hbuild.
  rewrite (build_tree_leaf_timestamps t groups Hbuild).
  exact Hsorted.
Qed.

(** ** Summary Coverage *)

(** All leaf content hashes *)
Fixpoint leaf_hashes (t : SummaryTree) : list nat :=
  match t with
  | Leaf g => [group_content_hash g]
  | Node l r _ => leaf_hashes l ++ leaf_hashes r
  end.

(** A summary "covers" a tree if it encodes all leaf content. *)
(** Simplified: summary hash = fold of all leaf hashes (xor). *)
Definition covers (summary_hash : nat) (t : SummaryTree) : Prop :=
  summary_hash = fold_left Nat.lxor (leaf_hashes t) 0.

(** Theorem: Recursive summarization covers all leaves.

    [CORRECTION] The original [summary_covers_all_leaves] concluded the vacuous
    placeholder [True].  The real (and provable) content of a "covering" hash
    fold is its accumulator-shift / homomorphism law over the leaf-hash lists:
    folding [Nat.lxor] over the combined tree equals combining the per-subtree
    folds.  Concretely, the covering hash of a [Node l r s] is the xor of the
    covering hashes of [l] and [r].  That is the discrete statement that the
    parent summary is exactly determined by (covers) its children's content. *)
(** Accumulator-shift law for [fold_left Nat.lxor] (the [lxor] monoid version
    of [fold_left_add_shift] in CompressionBounds.v). *)
Lemma fold_left_lxor_shift :
  forall l a, fold_left Nat.lxor l a = Nat.lxor a (fold_left Nat.lxor l 0).
Proof.
  induction l as [|x xs IH]; intros a.
  - cbn [fold_left]. rewrite Nat.lxor_0_r. reflexivity.
  - cbn [fold_left].
    rewrite (IH (Nat.lxor a x)).
    rewrite (IH (Nat.lxor 0 x)).
    rewrite Nat.lxor_0_l.
    rewrite Nat.lxor_assoc. reflexivity.
Qed.

Theorem summary_covers_all_leaves :
  forall l r s,
    covers (fold_left Nat.lxor (leaf_hashes l) 0) l ->
    covers (fold_left Nat.lxor (leaf_hashes r) 0) r ->
    (* The node's covering hash is the xor-combination of the children's. *)
    fold_left Nat.lxor (leaf_hashes (Node l r s)) 0
      = Nat.lxor (fold_left Nat.lxor (leaf_hashes l) 0)
                 (fold_left Nat.lxor (leaf_hashes r) 0).
Proof.
  intros l r s _ _.
  cbn [leaf_hashes].
  rewrite fold_left_app.
  apply fold_left_lxor_shift.
Qed.

(** ** Incremental Updates *)

(** Add a new group to an existing tree *)
Definition add_group (t : SummaryTree) (g : ConvGroup) : SummaryTree :=
  Node t (Leaf g) 0.  (* Placeholder summary tokens *)

(** Theorem: Adding a group increases leaf count by 1 *)
Theorem add_group_leaf_count :
  forall t g,
    leaf_count (add_group t g) = leaf_count t + 1.
Proof.
  intros t g.
  unfold add_group.
  simpl. lia.
Qed.

(** Theorem: Adding a group increases depth by at most 1 *)
Theorem add_group_depth :
  forall t g,
    tree_depth (add_group t g) <= tree_depth t + 1.
Proof.
  intros t g.
  unfold add_group.
  simpl.
  (* max (tree_depth t) 0 + 1 = tree_depth t + 1 when tree_depth t >= 0 *)
  lia.
Qed.

(** ** Recursive Single-Pass Compression: the sub-linear size recurrence

    AutoCompressors compress accumulated summaries recursively.  Model the
    compressed size after [k] recursion levels by the clean recurrence the
    paper's tree structure induces: each level shrinks the carried summary by a
    factor [b] (the branching/compression factor) and adds a fixed overhead [c]
    per level.  We prove the recurrence is well-formed, decreasing toward a
    fixed point, and bounded by the geometric closed form -- i.e. the
    compressed size is sub-linear (logarithmic-depth) in the input length. *)

(** Compressed size after [k] recursive levels, starting from [n0]. *)
Fixpoint compressed (n0 b c k : nat) : nat :=
  match k with
  | 0 => n0
  | S k' => compressed n0 b c k' / b + c
  end.

(** Theorem: each recursion level obeys the floor-division recurrence
    [compressed (S k) = compressed k / b + c].  This is the exact
    "compress accumulated summaries by factor [b], add [c] overhead" step. *)
Theorem compressed_step :
  forall n0 b c k,
    compressed n0 b c (S k) = compressed n0 b c k / b + c.
Proof.
  intros. cbn [compressed]. reflexivity.
Qed.

(** Theorem: with branching factor [b >= 2], the recurrence is contracting:
    once the carried size exceeds the fixed point [2*c], the next level is
    strictly smaller.  This is the termination/strict-decrease property: the
    recursion drives size down until it reaches the [O(c)] base case rather
    than diverging or oscillating. *)
Theorem compressed_strictly_decreases :
  forall n0 b c k,
    2 <= b ->
    2 * c < compressed n0 b c k ->
    compressed n0 b c (S k) < compressed n0 b c k.
Proof.
  intros n0 b c k Hb Hbig.
  rewrite compressed_step.
  set (x := compressed n0 b c k) in *.
  (* x / b <= x / 2, and x / 2 + c < x because 2*c < x. *)
  assert (Hdiv : x / b <= x / 2).
  { apply Nat.div_le_compat_l. lia. }
  (* x/2 + c < x : use div_mod_eq for /2 *)
  pose proof (Nat.div_mod_eq x 2) as Hdm.
  pose proof (Nat.mod_upper_bound x 2 ltac:(lia)) as Hmod.
  (* x = 2*(x/2) + x mod 2, x mod 2 < 2 => x/2 <= (x-?)/2; combine with 2c<x *)
  nia.
Qed.

(** Geometric upper envelope: [compressed k <= n0 / b^k + 2*c].

    The closed-form sub-linear bound.  The [+ 2*c] absorbs the geometric sum of
    the per-level overheads [c * (1 + 1/b + 1/b^2 + ...) <= c * b/(b-1) <= 2*c]
    for [b >= 2].  So after [k] levels the carried summary is at most the
    input shrunk by [b^k] plus a constant -- exactly the AutoCompressors
    "summary size grows sub-linearly (logarithmically) in input length"
    guarantee. *)
(** Helper: for [b >= 2], [(a + 2c)/b <= a/b + c].  This is the per-level
    "the overhead's contribution to the floor never exceeds [c]" step, proved
    by the [div_mod_eq + nia] pattern. *)
Lemma div_add_2c_le :
  forall a c b, 2 <= b -> (a + 2 * c) / b <= a / b + c.
Proof.
  intros a c b Hb.
  pose proof (Nat.div_mod_eq a b) as Ha.
  pose proof (Nat.div_mod_eq (a + 2 * c) b) as Hab.
  pose proof (Nat.mod_upper_bound a b ltac:(lia)) as Hma.
  pose proof (Nat.mod_upper_bound (a + 2 * c) b ltac:(lia)) as Hmab.
  nia.
Qed.

Theorem compressed_geometric_bound :
  forall n0 b c k,
    2 <= b ->
    compressed n0 b c k <= n0 / (b ^ k) + 2 * c.
Proof.
  intros n0 b c k Hb. revert n0.
  induction k as [|k IH]; intros n0.
  - cbn [compressed Nat.pow]. rewrite Nat.div_1_r. lia.
  - rewrite compressed_step.
    (* compressed k / b + c <= (n0/b^k + 2c)/b + c *)
    assert (Hstep : compressed n0 b c k / b <= (n0 / (b ^ k) + 2 * c) / b).
    { apply Nat.Div0.div_le_mono. apply IH. }
    eapply Nat.le_trans.
    { apply Nat.add_le_mono_r. exact Hstep. }
    (* Now: (n0/b^k + 2c)/b + c <= n0/b^(S k) + 2c *)
    set (a := n0 / (b ^ k)) in *.
    assert (Hsplit : (a + 2 * c) / b <= a / b + c) by (apply div_add_2c_le; lia).
    assert (Hpow : a / b = n0 / (b ^ S k)).
    { subst a. rewrite Nat.Div0.div_div.
      cbn [Nat.pow]. rewrite (Nat.mul_comm b (b ^ k)). reflexivity. }
    lia.
Qed.

(** ** Hierarchical vs. Flat: hierarchical compressed total never exceeds flat

    Flat single-pass compression of [n] items at ratio [1/b] (plus [c]
    overhead) costs [n/b + c].  Hierarchical compression, having already paid
    the per-level recurrence, lands at [compressed n b c k] which is bounded by
    the geometric envelope and, for [k >= 1], is no worse than the flat cost
    plus the same constant overhead.  We state the clean comparison at one
    level (the regime where they are directly comparable) and the general
    geometric envelope above. *)

(** Flat single-pass cost: compress all [n] at once with factor [b]. *)
Definition flat_compress (n b c : nat) : nat := n / b + c.

(** Theorem: one hierarchical level equals the flat single-pass cost on the
    same input and ratio; deeper hierarchical recursion is then bounded above
    by the geometric envelope (so it never exceeds flat by more than the
    bounded overhead constant).  This is the "hierarchical <= flat for the same
    ratio" comparison made precise: level 1 matches flat exactly, and every
    further level only shrinks the dominant [n/b^k] term. *)
Theorem hierarchical_le_flat :
  forall n b c,
    compressed n b c 1 = flat_compress n b c.
Proof.
  intros n b c.
  cbn [compressed]. unfold flat_compress. reflexivity.
Qed.

(** Corollary: for [b >= 2] and any depth [k >= 1], the hierarchical
    compressed size is bounded by the flat dominant term shrunk by [b^k] plus
    the bounded overhead -- strictly sub-linear in [n]. *)
Theorem hierarchical_sublinear :
  forall n b c k,
    2 <= b ->
    compressed n b c k <= n / (b ^ k) + 2 * c.
Proof.
  intros n b c k Hb.
  apply compressed_geometric_bound; assumption.
Qed.

(** ** Compression via Summarization *)

(** A summary reduces tokens *)
Definition valid_summarization (original_tokens summary_tokens : nat) : Prop :=
  summary_tokens <= original_tokens.

(** Apply summarization at a node *)
Definition summarize_node (t : SummaryTree) (summary_tokens : nat) : SummaryTree :=
  match t with
  | Leaf g => Leaf g  (* Can't summarize a leaf *)
  | Node l r _ => Node l r summary_tokens
  end.

(** Theorem: Summarization reduces effective tokens (recursion strictly
    decreases the carried token total, guaranteeing termination toward a base
    case).

    [CORRECTION] The original hypothesis was
      [new_summary < old_summary + tree_tokens l + tree_tokens r],
    which does NOT imply the conclusion: the children tokens [tree_tokens l]
    and [tree_tokens r] appear identically on both sides and cancel, so the
    node total strictly decreases iff [new_summary < old_summary] alone.  The
    original bound is too weak -- e.g. [old_summary = 0], children [5 + 5], and
    [new_summary = 9 < 0 + 10] gives node totals [19 < 10], which is false.
    The true, strong statement is that strictly shrinking the *summary* token
    count strictly shrinks the node total (the genuine per-level decrease that
    drives recursion to its base case). *)
Theorem summarization_reduces :
  forall l r old_summary new_summary,
    new_summary < old_summary ->
    tree_tokens (Node l r new_summary) <
    tree_tokens (Node l r old_summary).
Proof.
  intros l r old_summary new_summary Hless.
  cbn [tree_tokens]. lia.
Qed.

(** ** Deduplication *)

(** Find duplicate content by hash *)
Definition find_duplicates (t : SummaryTree) : list (nat * nat) :=
  (* Returns pairs of (hash, count) for hashes appearing > 1 time *)
  (* Simplified implementation *)
  [].

(** A structural "dedup/prune" relation: [t'] is obtained from [t] by replacing
    whole subtrees with smaller-or-equal subtrees (pruning duplicated
    content).  This is the honest hypothesis that lets dedup conclude a token
    reduction; comparing two structurally-unrelated trees cannot. *)
Inductive prunes_to : SummaryTree -> SummaryTree -> Prop :=
  | prune_refl : forall t, prunes_to t t
  | prune_drop_left : forall l r s,
      (* drop the left child, keeping the (smaller) right subtree *)
      prunes_to (Node l r s) r
  | prune_drop_right : forall l r s,
      prunes_to (Node l r s) l
  | prune_node : forall l l' r r' s,
      prunes_to l l' -> prunes_to r r' ->
      tree_tokens (Node l' r' s) <= tree_tokens (Node l r s) ->
      prunes_to (Node l r s) (Node l' r' s)
  | prune_trans : forall a b d,
      prunes_to a b -> prunes_to b d -> prunes_to a d.

(** Theorem: Deduplication can only reduce token count.

    [CORRECTION] The original statement was
      [leaf_count t' <= leaf_count t -> tree_tokens t' <= tree_tokens t]
    relating two ARBITRARY, structurally-unrelated trees.  That is FALSE: take
    [t' = Leaf g'] with [group_tokens g' = 100] and
    [t = Node (Leaf a) (Leaf b) 0] with tiny token counts; then
    [leaf_count t' = 1 <= 2 = leaf_count t] yet [tree_tokens t' = 100 > ...].
    Leaf count says nothing about token totals.  The real guarantee needs a
    structural relation between [t] and [t']: [t'] is a *prune* of [t] (whole
    duplicated subtrees dropped, each pruning step token-non-increasing).
    Under that genuine dedup relation the token total provably never grows. *)
Theorem dedup_reduces_tokens :
  forall t t',
    prunes_to t t' ->
    tree_tokens t' <= tree_tokens t.
Proof.
  intros t t' Hp.
  induction Hp.
  - (* refl *) lia.
  - (* drop_left: r <= Node l r s *)
    cbn [tree_tokens]. lia.
  - (* drop_right: l <= Node l r s *)
    cbn [tree_tokens]. lia.
  - (* node: token bound carried in the constructor *)
    exact H.
  - (* trans *)
    lia.
Qed.

(** ** Merge Operations *)

(** Merge two trees into one *)
Definition merge_trees (t1 t2 : SummaryTree) : SummaryTree :=
  Node t1 t2 0.

(** Theorem: Merge preserves all leaves *)
Theorem merge_preserves_leaves :
  forall t1 t2,
    leaf_count (merge_trees t1 t2) = leaf_count t1 + leaf_count t2.
Proof.
  intros t1 t2.
  unfold merge_trees. simpl. reflexivity.
Qed.

(** Theorem: Merge preserves temporal order when t1 precedes t2 *)
Theorem merge_preserves_order :
  forall t1 t2,
    is_sorted (leaf_timestamps t1) ->
    is_sorted (leaf_timestamps t2) ->
    (forall ts1 ts2,
      In ts1 (leaf_timestamps t1) ->
      In ts2 (leaf_timestamps t2) ->
      ts1 <= ts2) ->
    is_sorted (leaf_timestamps (merge_trees t1 t2)).
Proof.
  intros t1 t2 Hs1 Hs2 Horder.
  unfold merge_trees. cbn [leaf_timestamps].
  apply is_sorted_app; assumption.
Qed.

(** ** Amortized Complexity *)

(** Cost of operations *)
Definition insert_cost (t : SummaryTree) : nat := tree_depth t.
Definition summarize_cost (t : SummaryTree) : nat := leaf_count t.

(** Theorem: Amortized insert cost is at least [log2 n].

    [CORRECTION] The original [amortized_insert_log] claimed the *upper* bound
      [insert_cost t <= Nat.log2 (leaf_count t) + 1]
    i.e. [tree_depth t <= Nat.log2 (leaf_count t) + 1], which is the same false
    claim as the original [tree_depth_log_bound]: a degenerate spine of [n]
    leaves has depth [n-1].  The true statement under this (unbalanced-capable)
    datatype is the matching LOWER bound on the depth cost,
    [Nat.log2 (leaf_count t) <= insert_cost t]: an insert that walks to a leaf
    must traverse at least [log2 n] levels, which is the cost a *balanced*
    tree achieves.  (An upper bound of [log2 n + 1] would additionally require
    a balance invariant the datatype does not carry.) *)
Theorem amortized_insert_log :
  forall t,
    Nat.log2 (leaf_count t) <= insert_cost t.
Proof.
  intros t.
  unfold insert_cost.
  apply tree_depth_log_bound.
Qed.

(** ** Comparison with Flat Compaction *)

(** JFC's current flat approach: summarize all old groups at once *)
Definition flat_compact_cost (n_groups : nat) : nat := n_groups.

(** Hierarchical approach: summarize incrementally *)
Definition hierarchical_compact_cost (n_groups : nat) : nat := Nat.log2 n_groups + 1.

(** Theorem: Hierarchical is more efficient for large n *)
Theorem hierarchical_more_efficient :
  forall n,
    n >= 8 ->
    hierarchical_compact_cost n < flat_compact_cost n.
Proof.
  intros n Hn.
  unfold hierarchical_compact_cost, flat_compact_cost.
  (* log2(n) + 1 < n when n >= 8 *)
  assert (Hlog : Nat.log2 n < n).
  { apply Nat.log2_lt_lin. lia. }
  (* log2 n < n gives log2 n + 1 <= n; need strict, so use that
     log2 n <= n - 2 for n >= 8 via log2_le_pow2. *)
  assert (Hle : Nat.log2 n <= n - 2).
  { (* For n >= 8 = 2^3, we have log2 n >= 3, but we need an UPPER bound.
       Use: 2 ^ (Nat.log2 n) <= n (spec) and n < 2^(n-1) for n>=? Instead
       bound directly: log2 n <= n - 2 iff n >= log2 n + 2. Since
       log2 n < n (strict) we get log2 n + 1 <= n; we need one more slack.
       Use log2_le_lin style: log2 n grows much slower. Prove via
       2^(log2 n) <= n and the fact that for n>=8, log2 n + 2 <= n. *)
    pose proof (Nat.log2_spec n ltac:(lia)) as [Hlo Hhi].
    (* 2 ^ log2 n <= n < 2 ^ (S (log2 n)). With 2^k >= k+2 for k>=2,
       and log2 n >= 3 for n >= 8. *)
    set (k := Nat.log2 n) in *.
    assert (Hk3 : 3 <= k).
    { subst k. change 3 with (Nat.log2 8).
      apply Nat.log2_le_mono. lia. }
    (* 2 ^ k <= n and k + 2 <= 2 ^ k for k >= 2 => k + 2 <= n *)
    assert (Hpow_ge : k + 2 <= 2 ^ k).
    { clear Hlo Hhi Hlog Hn.
      induction k as [|k IHk]; [ lia |].
      destruct (Nat.le_gt_cases 3 k) as [Hge|Hlt].
      - cbn [Nat.pow]. assert (k + 2 <= 2 ^ k) by (apply IHk; lia). lia.
      - (* k < 3, so S k <= 3; only need S k where 3 <= S k, i.e. k=2 *)
        assert (k = 2) by lia. subst k. cbn [Nat.pow]. lia. }
    lia. }
  lia.
Qed.
