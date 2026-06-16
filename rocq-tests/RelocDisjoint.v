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
    (* apply1 r' (apply1 r m) = apply1 r (apply1 r' m) by commutation *)
    rewrite apply_comm by (apply disjoint_sym; exact Hd).
    apply IH. exact Hrest.
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

(** Corollary: determinism of the parallel relocation phase. Any two thread
    schedules (modeled as permutations of the disjoint relocation set) produce
    identical output memory. *)
Corollary parallel_reloc_deterministic :
  forall sched1 sched2 m,
    Permutation sched1 sched2 ->
    pairwise_disjoint sched1 ->
    apply_all sched1 m = apply_all sched2 m.
Proof. intros; eapply apply_all_perm_invariant; eauto. Qed.
