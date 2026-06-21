(** * RelocDisjoint.v — Relocation disjoint-write parallelism (QUAD.md §5)

    Machine-checks QUAD Theorem 5.1: if every relocation writes to a pairwise
    disjoint byte range (its footprint), then applying the relocations in any
    order — in particular, fully in parallel across threads — produces the same
    output memory as applying them sequentially.

    Following the model council's guidance, the disjointness of the partition is
    a *derived* property (a frame condition), not a bare assumption: we prove
    [apply_comm] (any two disjoint-footprint relocations commute) and lift it to
    [apply_all_perm_invariant] (the whole list is order-independent), which is
    the actual justification for peony's `write_section_data_parallel`.

    This is the separation-logic frame argument (council direction (d)) done
    concretely over a finite byte-addressed memory.

    Maps to: QUAD.md §5.1 (Definition 5.1), Theorem 5.1, Theorem 12.1. *)

From Stdlib Require Import List Arith Bool Lia.
From Stdlib Require Import Permutation.
From Stdlib Require Import FunctionalExtensionality.
Import ListNotations.

(* ------------------------------------------------------------------ *)
(** ** Memory and relocations                                          *)
(* ------------------------------------------------------------------ *)

(** Output memory: a total function from byte address to byte value. This is the
    abstract model of the mmap'd output buffer. *)
Definition addr := nat.
Definition byte := nat.
Definition mem := addr -> byte.

(** A relocation writes a fixed byte value at each address in its footprint.
    [r_off] is the start offset, [r_bytes] the list of bytes written (length =
    width). This models [apply_reloc] writing `width` little-endian bytes at
    `offset`. The *footprint* is the contiguous range [r_off, r_off+|r_bytes|). *)
Record reloc := {
  r_off   : addr;
  r_bytes : list byte
}.

Definition footprint (r : reloc) : list addr :=
  map (fun i => r_off r + i) (seq 0 (length (r_bytes r))).

(** Apply one relocation: overwrite each footprint byte with the corresponding
    value from [r_bytes]; leave all other addresses unchanged. *)
Definition apply1 (r : reloc) (m : mem) : mem :=
  fun a =>
    if andb (Nat.leb (r_off r) a) (Nat.ltb a (r_off r + length (r_bytes r)))
    then nth (a - r_off r) (r_bytes r) (m a)
    else m a.

(** Apply a whole list of relocations left to right. *)
Definition apply_all (rs : list reloc) (m : mem) : mem :=
  fold_left (fun m r => apply1 r m) rs m.

(* ------------------------------------------------------------------ *)
(** ** Disjointness                                                    *)
(* ------------------------------------------------------------------ *)

(** Two relocations are disjoint iff their footprints share no address. We state
    it as range non-overlap, which is decidable and easy to discharge. *)
Definition disjoint (r1 r2 : reloc) : Prop :=
  r_off r1 + length (r_bytes r1) <= r_off r2 \/
  r_off r2 + length (r_bytes r2) <= r_off r1.

Lemma disjoint_sym : forall r1 r2, disjoint r1 r2 -> disjoint r2 r1.
Proof. unfold disjoint; intros r1 r2 [H|H]; [right|left]; exact H. Qed.

(** Key lemma: an address written by [r1] is not in [r2]'s range when they are
    disjoint, hence [apply1 r2] leaves it untouched. *)
Lemma apply1_outside :
  forall r m a,
    ~ (r_off r <= a /\ a < r_off r + length (r_bytes r)) ->
    apply1 r m a = m a.
Proof.
  intros r m a Hout. unfold apply1.
  destruct (Nat.leb (r_off r) a) eqn:Hle;
  destruct (Nat.ltb a (r_off r + length (r_bytes r))) eqn:Hlt; simpl; try reflexivity.
  apply Nat.leb_le in Hle. apply Nat.ltb_lt in Hlt. exfalso; apply Hout; split; assumption.
Qed.

Lemma apply1_inside :
  forall r m a,
    r_off r <= a -> a < r_off r + length (r_bytes r) ->
    apply1 r m a = nth (a - r_off r) (r_bytes r) (m a).
Proof.
  intros r m a Hle Hlt. unfold apply1.
  apply Nat.leb_le in Hle. apply Nat.ltb_lt in Hlt. rewrite Hle, Hlt. reflexivity.
Qed.

(* ------------------------------------------------------------------ *)
(** ** Commutation of disjoint relocations (the frame property)        *)
(* ------------------------------------------------------------------ *)

(** THE separation lemma: two relocations with disjoint footprints commute.
    Applying r1 then r2 equals applying r2 then r1 — pointwise on every address.
    This is the frame condition that makes parallel application sound. *)
Theorem apply_comm :
  forall r1 r2 m,
    disjoint r1 r2 ->
    apply1 r1 (apply1 r2 m) = apply1 r2 (apply1 r1 m).
Proof.
  intros r1 r2 m Hdis.
  apply functional_extensionality; intro a.
  unfold disjoint in Hdis.
  (* Decide the two range-membership tests on each side arithmetically.
     Disjointness makes "a in both ranges" contradictory; every other case
     yields the same byte on both sides. *)
  unfold apply1.
  destruct (Nat.leb (r_off r1) a) eqn:E1l;
  destruct (Nat.ltb a (r_off r1 + length (r_bytes r1))) eqn:E1u;
  destruct (Nat.leb (r_off r2) a) eqn:E2l;
  destruct (Nat.ltb a (r_off r2 + length (r_bytes r2))) eqn:E2u;
  repeat match goal with
  | [ H : Nat.leb _ _ = true  |- _ ] => apply Nat.leb_le in H
  | [ H : Nat.leb _ _ = false |- _ ] => apply Nat.leb_nle in H
  | [ H : Nat.ltb _ _ = true  |- _ ] => apply Nat.ltb_lt in H
  | [ H : Nat.ltb _ _ = false |- _ ] => apply Nat.ltb_ge in H
  end;
  try reflexivity; try (exfalso; lia).
Qed.

(* ------------------------------------------------------------------ *)
(** ** Pairwise disjoint lists                                         *)
(* ------------------------------------------------------------------ *)

(** A list of relocations is pairwise disjoint. *)
Inductive pairwise_disjoint : list reloc -> Prop :=
| pd_nil  : pairwise_disjoint []
| pd_cons : forall r rs,
    Forall (disjoint r) rs ->
    pairwise_disjoint rs ->
    pairwise_disjoint (r :: rs).

(** Pushing one relocation through a disjoint prefix: if [r] is disjoint from
    every reloc in [rs], applying [r] commutes with applying the whole list. *)
Lemma apply_all_push :
  forall rs r m,
    Forall (disjoint r) rs ->
    apply_all rs (apply1 r m) = apply1 r (apply_all rs m).
Proof.
  induction rs as [|r' rs IH]; intros r m HF; simpl.
  - reflexivity.
  - inversion HF as [|? ? Hd Hrest]; subst.
    rewrite apply_comm by (apply disjoint_sym; exact Hd).
    apply IH. exact Hrest.
Qed.

(** The Rust dispatcher checks half-open mmap output ranges before any worker
    creates mutable slices. A work item is one input-section contribution and may
    contain many relocations; relocations inside that item are still applied
    sequentially by one worker. The range bridge therefore has two shapes:

    - the simple one-range/one-relocation special case, which implies
      [pairwise_disjoint] for a flat relocation list;
    - the implementation-shaped batch case, which implies cross-batch
      disjointness for lists of relocations grouped by accepted work range. *)
Record write_range := {
  wr_off : addr;
  wr_len : nat
}.

Definition range_disjoint (w1 w2 : write_range) : Prop :=
  wr_off w1 + wr_len w1 <= wr_off w2 \/
  wr_off w2 + wr_len w2 <= wr_off w1.

Inductive pairwise_range_disjoint : list write_range -> Prop :=
| prd_nil : pairwise_range_disjoint []
| prd_cons : forall w ws,
    Forall (range_disjoint w) ws ->
    pairwise_range_disjoint ws ->
    pairwise_range_disjoint (w :: ws).

(** A write footprint belongs to a checked output range when every byte it may
    touch is contained in that half-open range. In the Rust emit path, the
    section copy touches the whole range and relocations touch subranges inside
    the same mutable slice. *)
Definition footprint_within (w : write_range) (r : reloc) : Prop :=
  wr_off w <= r_off r /\
  r_off r + length (r_bytes r) <= wr_off w + wr_len w.

Inductive footprints_fit : list write_range -> list reloc -> Prop :=
| ff_nil : footprints_fit [] []
| ff_cons : forall w ws r rs,
    footprint_within w r ->
    footprints_fit ws rs ->
    footprints_fit (w :: ws) (r :: rs).

Lemma range_disjoint_to_reloc :
  forall w1 w2 r1 r2,
    footprint_within w1 r1 ->
    footprint_within w2 r2 ->
    range_disjoint w1 w2 ->
    disjoint r1 r2.
Proof.
  intros w1 w2 r1 r2 Hfp1 Hfp2 Hrange.
  unfold footprint_within in Hfp1, Hfp2.
  destruct Hfp1 as [Hlo1 Hhi1].
  destruct Hfp2 as [Hlo2 Hhi2].
  unfold range_disjoint in Hrange; unfold disjoint.
  destruct Hrange as [H|H].
  - left. lia.
  - right. lia.
Qed.

Lemma footprints_fit_forall_disjoint :
  forall w ws r rs,
    footprint_within w r ->
    footprints_fit ws rs ->
    Forall (range_disjoint w) ws ->
    Forall (disjoint r) rs.
Proof.
  intros w ws r rs Hw Hfit Hforall.
  induction Hfit.
  - constructor.
  - inversion Hforall as [|? ? Hhead Htail]; subst.
    constructor.
    + eapply range_disjoint_to_reloc; eauto.
    + apply IHHfit. exact Htail.
Qed.

Theorem accepted_emit_ranges_reloc_precondition :
  forall ranges relocs,
    footprints_fit ranges relocs ->
    pairwise_range_disjoint ranges ->
    pairwise_disjoint relocs.
Proof.
  intros ranges relocs Hfit. induction Hfit; intro Hpairwise.
  - constructor.
  - inversion Hpairwise as [|? ? Hhead Htail]; subst.
    constructor.
    + eapply footprints_fit_forall_disjoint; eauto.
    + apply IHHfit. exact Htail.
Qed.

(** Current Rust shape: one accepted output range owns a batch of relocation
    writes applied sequentially inside that range. Cross-batch disjointness is
    the property needed by parallel workers; intra-batch relocation order remains
    the original sequential order and need not be modeled as commutative. *)
Definition all_footprints_within (w : write_range) (rs : list reloc) : Prop :=
  Forall (footprint_within w) rs.

Inductive batches_fit : list write_range -> list (list reloc) -> Prop :=
| bf_nil : batches_fit [] []
| bf_cons : forall w ws rs batches,
    all_footprints_within w rs ->
    batches_fit ws batches ->
    batches_fit (w :: ws) (rs :: batches).

Definition batch_disjoint (rs1 rs2 : list reloc) : Prop :=
  Forall (fun r1 => Forall (disjoint r1) rs2) rs1.

Inductive pairwise_batch_disjoint : list (list reloc) -> Prop :=
| pbd_nil : pairwise_batch_disjoint []
| pbd_cons : forall rs batches,
    Forall (batch_disjoint rs) batches ->
    pairwise_batch_disjoint batches ->
    pairwise_batch_disjoint (rs :: batches).

Definition apply_batches (batches : list (list reloc)) (m : mem) : mem :=
  fold_left (fun m rs => apply_all rs m) batches m.

Lemma range_disjoint_to_batch :
  forall w1 w2 rs1 rs2,
    all_footprints_within w1 rs1 ->
    all_footprints_within w2 rs2 ->
    range_disjoint w1 w2 ->
    batch_disjoint rs1 rs2.
Proof.
  unfold all_footprints_within, batch_disjoint.
  intros w1 w2 rs1 rs2 Hrs1 Hrs2 Hrange.
  apply Forall_forall. intros r1 Hin1.
  apply Forall_forall. intros r2 Hin2.
  pose proof (proj1 (Forall_forall _ _) Hrs1 r1 Hin1) as Hfp1.
  pose proof (proj1 (Forall_forall _ _) Hrs2 r2 Hin2) as Hfp2.
  eapply range_disjoint_to_reloc; eauto.
Qed.

Lemma batches_fit_forall_disjoint :
  forall w ws rs batches,
    all_footprints_within w rs ->
    batches_fit ws batches ->
    Forall (range_disjoint w) ws ->
    Forall (batch_disjoint rs) batches.
Proof.
  intros w ws rs batches Hrs Hfit Hforall.
  induction Hfit.
  - constructor.
  - inversion Hforall as [|? ? Hhead Htail]; subst.
    constructor.
    + eapply range_disjoint_to_batch; eauto.
    + apply IHHfit. exact Htail.
Qed.

Theorem accepted_emit_ranges_batch_precondition :
  forall ranges batches,
    batches_fit ranges batches ->
    pairwise_range_disjoint ranges ->
    pairwise_batch_disjoint batches.
Proof.
  intros ranges batches Hfit. induction Hfit; intro Hpairwise.
  - constructor.
  - inversion Hpairwise as [|? ? Hhead Htail]; subst.
    constructor.
    + eapply batches_fit_forall_disjoint; eauto.
    + apply IHHfit. exact Htail.
Qed.

Lemma batch_disjoint_sym :
  forall rs1 rs2,
    batch_disjoint rs1 rs2 ->
    batch_disjoint rs2 rs1.
Proof.
  unfold batch_disjoint. intros rs1 rs2 Hcross.
  apply Forall_forall. intros r2 Hin2.
  apply Forall_forall. intros r1 Hin1.
  apply disjoint_sym.
  pose proof (proj1 (Forall_forall _ _) Hcross r1 Hin1) as Hrs2.
  exact (proj1 (Forall_forall _ _) Hrs2 r2 Hin2).
Qed.

Lemma batch_comm :
  forall rs1 rs2 m,
    batch_disjoint rs1 rs2 ->
    apply_all rs1 (apply_all rs2 m) = apply_all rs2 (apply_all rs1 m).
Proof.
  induction rs1 as [|r rs1 IH]; intros rs2 m Hcross; simpl.
  - reflexivity.
  - inversion Hcross as [|? ? Hr Htail]; subst.
    rewrite <- (apply_all_push rs2 r m Hr).
    apply IH. exact Htail.
Qed.

Lemma batch_push :
  forall batches batch m,
    Forall (batch_disjoint batch) batches ->
    apply_batches batches (apply_all batch m) =
    apply_all batch (apply_batches batches m).
Proof.
  induction batches as [|batch' batches IH]; intros batch m HF; simpl.
  - reflexivity.
  - inversion HF as [|? ? Hhead Htail]; subst.
    rewrite <- (batch_comm batch batch' m Hhead).
    apply IH. exact Htail.
Qed.

Lemma Forall_perm_batches : forall (P : list reloc -> Prop) l1 l2,
  Permutation l1 l2 -> Forall P l1 -> Forall P l2.
Proof.
  intros P l1 l2 HP. induction HP; intro HF; auto.
  - inversion HF; subst. constructor; auto.
  - inversion HF as [|? ? H1 H2]; subst. inversion H2; subst.
    constructor; [assumption| constructor; assumption].
Qed.

Lemma pairwise_batch_disjoint_perm : forall l1 l2,
  Permutation l1 l2 ->
  pairwise_batch_disjoint l1 ->
  pairwise_batch_disjoint l2.
Proof.
  intros l1 l2 HP. induction HP; intro HD.
  - assumption.
  - inversion HD; subst. constructor.
    + apply (Forall_perm_batches (batch_disjoint x) l l'); assumption.
    + apply IHHP; assumption.
  - inversion HD as [|? ? Hy Hrest]; subst.
    inversion Hrest as [|? ? Hx Hxs]; subst.
    inversion Hy as [|? ? Hyx Hyl]; subst.
    constructor; [constructor; [apply batch_disjoint_sym; exact Hyx | exact Hx]|].
    constructor; [exact Hyl | exact Hxs].
  - auto.
Qed.

Theorem apply_batches_perm_invariant :
  forall l1 l2 m,
    Permutation l1 l2 ->
    pairwise_batch_disjoint l1 ->
    apply_batches l1 m = apply_batches l2 m.
Proof.
  intros l1 l2 m HP. revert m. induction HP; intros m HD; simpl.
  - reflexivity.
  - inversion HD; subst. apply IHHP. assumption.
  - inversion HD as [|? ? Hy Hrest]; subst.
    inversion Hrest as [|? ? Hx Hxs]; subst.
    inversion Hy as [|? ? Hyx Hyl]; subst.
    rewrite batch_comm by (apply batch_disjoint_sym; exact Hyx). reflexivity.
  - rewrite IHHP1 by assumption.
    apply IHHP2. apply (pairwise_batch_disjoint_perm l l'); assumption.
Qed.

(* ------------------------------------------------------------------ *)
(** ** Order independence: parallel = sequential                       *)
(* ------------------------------------------------------------------ *)

(** Forall is preserved by permutation (needed to carry disjointness across a
    reordering of the relocation list). *)
Lemma Forall_perm : forall (P : reloc -> Prop) l1 l2,
  Permutation l1 l2 -> Forall P l1 -> Forall P l2.
Proof.
  intros P l1 l2 HP. induction HP; intro HF; auto.
  - inversion HF; subst. constructor; auto.
  - inversion HF as [|? ? H1 H2]; subst. inversion H2; subst.
    constructor; [assumption| constructor; assumption].
Qed.

Lemma pairwise_disjoint_perm : forall l1 l2,
  Permutation l1 l2 -> pairwise_disjoint l1 -> pairwise_disjoint l2.
Proof.
  intros l1 l2 HP. induction HP; intro HD.
  - assumption.
  - inversion HD; subst. constructor.
    + apply (Forall_perm (disjoint x) l l'); assumption.
    + apply IHHP; assumption.
  - inversion HD as [|? ? Hy Hrest]; subst. inversion Hrest as [|? ? Hx Hxs]; subst.
    inversion Hy as [|? ? Hyx Hyl]; subst.
    constructor; [constructor; [apply disjoint_sym; exact Hyx | exact Hx]|].
    constructor; [exact Hyl | exact Hxs].
  - auto.
Qed.

(** MAIN THEOREM (QUAD Theorem 5.1 / 12.1, relocation part):
    For a pairwise-disjoint relocation list, [apply_all] is invariant under any
    permutation of the list. Hence applying relocations in parallel (any thread
    interleaving = some permutation of the sequential order) yields exactly the
    sequential result — the output binary is deterministic and parallel-safe. *)
Theorem apply_all_perm_invariant :
  forall l1 l2 m,
    Permutation l1 l2 ->
    pairwise_disjoint l1 ->
    apply_all l1 m = apply_all l2 m.
Proof.
  intros l1 l2 m HP. revert m. induction HP; intros m HD; simpl.
  - reflexivity.
  - (* skip head x *)
    inversion HD; subst. apply IHHP. assumption.
  - (* swap first two y x *)
    inversion HD as [|? ? Hy Hrest]; subst.
    inversion Hrest as [|? ? Hx Hxs]; subst.
    inversion Hy as [|? ? Hyx Hyl]; subst.
    (* both sides reduce to apply_all l (apply1 _ (apply1 _ m)); commute the two heads *)
    rewrite apply_comm by (apply disjoint_sym; exact Hyx). reflexivity.
  - (* transitivity *)
    rewrite IHHP1 by assumption.
    apply IHHP2. apply (pairwise_disjoint_perm l l'); assumption.
Qed.

(** If the implementation accepts the output ranges and each worker's relocation
    batch is contained in its owning range, then the worker batches may run in
    any order while preserving the sequential order inside each batch. This is
    the bridge shape used by `peony-emit`: parallelism is over section work
    items, not over individual relocation writes inside one section. *)
Corollary accepted_emit_ranges_parallel_batches_deterministic :
  forall ranges sched1 sched2 m,
    batches_fit ranges sched1 ->
    Permutation sched1 sched2 ->
    pairwise_range_disjoint ranges ->
    apply_batches sched1 m = apply_batches sched2 m.
Proof.
  intros ranges sched1 sched2 m Hmatch Hperm Hranges.
  eapply apply_batches_perm_invariant; eauto.
  eapply accepted_emit_ranges_batch_precondition; eauto.
Qed.

(** Special case: if each accepted range owns exactly one relocation, the
    existing flat relocation permutation theorem applies. *)
Corollary accepted_emit_ranges_parallel_deterministic :
  forall ranges sched1 sched2 m,
    footprints_fit ranges sched1 ->
    Permutation sched1 sched2 ->
    pairwise_range_disjoint ranges ->
    apply_all sched1 m = apply_all sched2 m.
Proof.
  intros ranges sched1 sched2 m Hmatch Hperm Hranges.
  eapply apply_all_perm_invariant; eauto.
  eapply accepted_emit_ranges_reloc_precondition; eauto.
Qed.

(** Corollary: determinism of the parallel relocation phase. Any two thread
    schedules (modeled as permutations of the disjoint relocation set) produce
    identical output memory. *)
Corollary parallel_reloc_deterministic :
  forall sched1 sched2 m,
    Permutation sched1 sched2 ->
    pairwise_disjoint sched1 ->
    apply_all sched1 m = apply_all sched2 m.
Proof. intros; eapply apply_all_perm_invariant; eauto. Qed.
