(** * RelocMonoid.v — NOVEL: relocation application as a partial commutative monoid

    The model council's recommended "fifth direction": give the algebraic
    structure of relocation composition. The deep-research prior-art survey found
    NO published treatment of relocations as a monoid/algebraic structure. Here
    we prove that relocation *patches* (finite maps address ⇸ byte with disjoint
    domains) form a PARTIAL COMMUTATIVE MONOID under disjoint union, acting on
    output memory by overwrite. This subsumes the disjoint-write separation
    result (RelocDisjoint.v) as the commutativity law of the monoid and connects
    the linker to the algebra of separation logic resources (PCMs / Iris camera).

    Key results:
    - [oplus] (disjoint union of patches) is commutative and associative where
      defined; the empty patch is the unit.
    - The monoid ACTS on memory: [act (p ⊕ q) = act p ∘ act q] when p,q disjoint
      (a homomorphism from the patch PCM to endofunctions on memory).
    - Determinism of parallel relocation is the commutativity of this action.

    Maps to: QUAD.md §5 (relocation algebra) — new algebraic framing. *)

From Stdlib Require Import List Arith Bool Lia.
From Stdlib Require Import FunctionalExtensionality.
Import ListNotations.

Definition addr := nat.
Definition byte := nat.

(* ------------------------------------------------------------------ *)
(** ** Patches: partial maps addr ⇸ byte                               *)
(* ------------------------------------------------------------------ *)

(** A patch is a partial function: [None] means "this address is untouched". A
    relocation contributes finitely many defined points; we work with the
    semantic partial map directly. *)
Definition patch := addr -> option byte.

Definition empty : patch := fun _ => None.

Definition dom (p : patch) (a : addr) : Prop := p a <> None.

(** Two patches are compatible (disjoint domains) — the side condition that
    makes their union well-defined. This is the PCM "defined-ness" relation. *)
Definition compatible (p q : patch) : Prop :=
  forall a, p a = None \/ q a = None.

Lemma compatible_sym : forall p q, compatible p q -> compatible q p.
Proof. intros p q H a. destruct (H a); auto. Qed.

(** Disjoint union: take whichever side is defined. On overlap (which
    [compatible] forbids) we bias to [p], but the laws below only hold under
    [compatible]. *)
Definition oplus (p q : patch) : patch :=
  fun a => match p a with
           | Some b => Some b
           | None   => q a
           end.

Notation "p ⊕ q" := (oplus p q) (at level 50, left associativity).

(* ------------------------------------------------------------------ *)
(** ** Monoid laws                                                     *)
(* ------------------------------------------------------------------ *)

(** [empty] is the left and right unit. *)
Theorem oplus_empty_l : forall p, empty ⊕ p = p.
Proof. intro p. apply functional_extensionality; intro a. reflexivity. Qed.

Theorem oplus_empty_r : forall p, p ⊕ empty = p.
Proof.
  intro p. apply functional_extensionality; intro a.
  unfold oplus, empty. destruct (p a); reflexivity.
Qed.

(** Associativity holds unconditionally (it is just option-bind nesting). *)
Theorem oplus_assoc : forall p q r, (p ⊕ q) ⊕ r = p ⊕ (q ⊕ r).
Proof.
  intros p q r. apply functional_extensionality; intro a.
  unfold oplus. destruct (p a); reflexivity.
Qed.

(** COMMUTATIVITY under compatibility — the partial-commutative-monoid law. *)
Theorem oplus_comm : forall p q, compatible p q -> p ⊕ q = q ⊕ p.
Proof.
  intros p q Hc. apply functional_extensionality; intro a.
  unfold oplus. destruct (Hc a) as [Hp | Hq].
  - rewrite Hp. destruct (q a); reflexivity.
  - rewrite Hq. destruct (p a); reflexivity.
Qed.

(** Compatibility is preserved by union on the left (frame-preservation). *)
Theorem compatible_oplus :
  forall p q r,
    compatible p r -> compatible q r -> compatible (p ⊕ q) r.
Proof.
  intros p q r Hpr Hqr a. unfold oplus.
  destruct (p a) eqn:Hp.
  - destruct (Hpr a) as [H|H]; [rewrite Hp in H; discriminate | right; exact H].
  - destruct (Hqr a) as [H|H]; [left; exact H | right; exact H].
Qed.

(* ------------------------------------------------------------------ *)
(** ** Action on memory (the monoid acts by overwrite)                 *)
(* ------------------------------------------------------------------ *)

Definition mem := addr -> byte.

(** Apply a patch to memory: defined points overwrite, undefined points show the
    base memory through. *)
Definition act (p : patch) (m : mem) : mem :=
  fun a => match p a with Some b => b | None => m a end.

(** [act] is a monoid action: empty acts as identity. *)
Theorem act_empty : forall m, act empty m = m.
Proof. intro m. apply functional_extensionality; intro a. reflexivity. Qed.

(** HOMOMORPHISM: acting by a union = composing the actions, for ANY patches
    (the bias in [oplus] makes this hold unconditionally). *)
Theorem act_oplus : forall p q m, act (p ⊕ q) m = act p (act q m).
Proof.
  intros p q m. apply functional_extensionality; intro a.
  unfold act, oplus. destruct (p a); reflexivity.
Qed.

(** PARALLEL DETERMINISM (the headline): for compatible patches, the action is
    order-independent — applying p then q equals q then p. This is the algebraic
    restatement of RelocDisjoint.apply_comm: disjoint relocations commute because
    they are compatible elements of the patch PCM, and [act] is a homomorphism. *)
Theorem act_comm :
  forall p q m, compatible p q ->
    act p (act q m) = act q (act p m).
Proof.
  intros p q m Hc.
  rewrite <- act_oplus, <- act_oplus.
  rewrite (oplus_comm p q Hc). reflexivity.
Qed.

(* ------------------------------------------------------------------ *)
(** ** Folding a disjoint patch list (the whole relocation phase)      *)
(* ------------------------------------------------------------------ *)

(** Combine a list of patches by ⊕. *)
Definition combine (ps : list patch) : patch := fold_right oplus empty ps.

(** Acting by the combined patch = folding the actions: the relocation phase as
    a single monoid element acting once, equivalently as sequential application.
    This justifies materializing all relocations into one output image. *)
Theorem act_combine :
  forall ps m, act (combine ps) m = fold_right act m ps.
Proof.
  induction ps as [|p ps IH]; intro m; simpl.
  - apply act_empty.
  - rewrite act_oplus. rewrite IH. reflexivity.
Qed.

(** Unit + associativity + (partial) commutativity package: relocations form a
    commutative monoid on their defined-disjoint fragment, with a homomorphic
    action on the output buffer. This is the algebraic kernel underlying both
    determinism and parallel safety of peony's emit phase. *)
Theorem reloc_pcm_summary :
  (forall p, empty ⊕ p = p) /\
  (forall p, p ⊕ empty = p) /\
  (forall p q r, (p ⊕ q) ⊕ r = p ⊕ (q ⊕ r)) /\
  (forall p q, compatible p q -> p ⊕ q = q ⊕ p) /\
  (forall p q m, act (p ⊕ q) m = act p (act q m)).
Proof.
  split; [apply oplus_empty_l|].
  split; [apply oplus_empty_r|].
  split; [apply oplus_assoc|].
  split; [apply oplus_comm|].
  apply act_oplus.
Qed.
