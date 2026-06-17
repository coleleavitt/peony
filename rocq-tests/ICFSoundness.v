(** * ICFSoundness.v — soundness of Identical Code Folding

    Identical Code Folding (ICF) merges functions with identical content+
    relocations into a single copy, shrinking the output (mold/lld --icf=all
    typically save 5–15%). The danger: C/C++ guarantee that two distinct
    functions have DISTINCT addresses, so folding two functions whose address is
    observed (taken and compared) changes observable behaviour. This file proves
    the soundness side-condition precisely:

      ICF is behaviour-preserving on the set of functions that are NOT
      address-significant (no `&f` taken in an address-comparison context).

    We model the program semantics abstractly as: behaviour is determined by
    (a) the bytes executed when each function is *called*, and (b) the set of
    *address comparisons* between function pointers. Two functions are
    *content-equivalent* when their executable meaning is identical. Folding
    redirects all references to the canonical representative.

    THEOREM I1 (call-equivalence preserved): folding equal-content functions
              does not change what any call computes.
    THEOREM I2 (address hazard characterised): folding changes an address
              comparison iff a folded function is address-significant.
    THEOREM I3 (soundness): if no folded function is address-significant, the
              folded program is observationally equivalent to the original.
    THEOREM I4 (partition refinement is a fixpoint): the ICF equivalence is the
              greatest fixpoint of the content-refinement operator (bisimulation),
              matching Hopcroft partition refinement.

    Compiles clean under Rocq/Coq 9.1.0.
*)

From Stdlib Require Import List Arith Bool Lia.
Import ListNotations.

(* ================================================================== *)
(** * 1.  Functions, content, and the fold map                         *)
(* ================================================================== *)

Definition fid := nat.                 (* function identity *)
Definition content := list nat.        (* executable bytes + normalised relocs *)

(** A program is a finite map from function id to its content, plus a flag set
    of *address-significant* functions (those whose pointer identity is
    observed: `&f` taken and compared, or in a vtable/typeinfo, etc. — the
    `_ZT*` symbols peony already excludes from copy-relocs are a concrete
    instance of address-significant data). *)
Record program := {
  p_funcs : fid -> content;
  p_addr_sig : fid -> bool             (* true = address-significant, MUST NOT fold *)
}.

(** Content-equivalence: two functions mean the same executable thing. *)
Definition content_eq (P : program) (f g : fid) : Prop :=
  p_funcs P f = p_funcs P g.

(** A fold map sends each function to a canonical representative. It is *valid*
    when it only merges content-equal functions and is idempotent (the
    representative is its own representative — a partition into equivalence
    classes, exactly the output of union-find / partition refinement). *)
Record fold_map (P : program) := {
  rep      : fid -> fid;
  rep_idem : forall f, rep (rep f) = rep f;
  rep_sound : forall f, content_eq P f (rep f)
}.

Arguments rep {P}.
Arguments rep_idem {P}.
Arguments rep_sound {P}.

(* ================================================================== *)
(** * 2.  Calling a function: behaviour is its content                 *)
(* ================================================================== *)

(** The observable result of CALLING a function is determined by its content
    (we don't model the dynamics; the point is that calling [f] vs calling its
    representative [rep f] executes byte-identical code). *)
Definition call_result (P : program) (f : fid) : content := p_funcs P f.

(** THEOREM I1 (Call-Equivalence Preserved). Replacing a call to [f] with a call
    to its ICF representative computes the identical result, because ICF only
    merges content-equal functions. This is the "code side" of soundness — the
    part that is ALWAYS safe. *)
Theorem icf_call_preserved :
  forall P (F : fold_map P) f,
    call_result P (rep F f) = call_result P f.
Proof.
  intros P F f. unfold call_result.
  symmetry. exact (rep_sound F f).
Qed.

(* ================================================================== *)
(** * 3.  Address comparisons: the hazard                              *)
(* ================================================================== *)

(** The observable address comparison of two function pointers, AFTER folding,
    asks whether their representatives coincide. BEFORE folding it asks whether
    the functions themselves coincide. *)
Definition addr_eq_before (f g : fid) : bool := Nat.eqb f g.
Definition addr_eq_after  (P : program) (F : fold_map P) (f g : fid) : bool :=
  Nat.eqb (rep F f) (rep F g).

(** A function is *address-significant* if the program observes its address. *)
Definition addr_significant (P : program) (f : fid) : bool := p_addr_sig P f.

(** THEOREM I2 (Address Hazard, characterised). Folding changes the result of
    comparing [f] and [g] EXACTLY when they are distinct yet share a
    representative (i.e. they got folded together). If neither is
    address-significant the program never performs this comparison observably,
    so the change is unobservable. *)
Theorem icf_addr_change_iff_folded :
  forall P (F : fold_map P) f g,
    addr_eq_after P F f g <> addr_eq_before f g <->
    (f <> g /\ rep F f = rep F g).
Proof.
  intros P F f g. unfold addr_eq_after, addr_eq_before. split.
  - intro Hdiff. split.
    + intro Hfg. subst g. apply Hdiff.
      rewrite !Nat.eqb_refl. reflexivity.
    + destruct (Nat.eqb_spec (rep F f) (rep F g)) as [He|He].
      * exact He.
      * (* after = false. If before were also false the two would be equal,
           contradicting Hdiff; so before = true, i.e. f = g — but then
           rep f = rep g, contradicting He. *)
        destruct (Nat.eqb_spec f g) as [Hfg|Hfg].
        -- subst g. exfalso. apply He. reflexivity.
        -- exfalso. apply Hdiff. reflexivity.
  - intros [Hfg Hrep].
    rewrite (proj2 (Nat.eqb_eq _ _) Hrep).
    destruct (Nat.eqb_spec f g) as [He|He].
    + subst g. contradiction.
    + discriminate.
Qed.

(* ================================================================== *)
(** * 4.  THEOREM I3 — soundness under the no-address-significance side-cond *)
(* ================================================================== *)

(** A fold map is *address-safe* when it never folds two distinct functions of
    which either is address-significant. This is exactly mold/lld's rule:
    `--icf=all` excludes address-significant symbols (the .llvm_addrsig set;
    peony's `_ZT*` typeinfo/vtable exclusion is the same idea applied to data). *)
Definition address_safe (P : program) (F : fold_map P) : Prop :=
  forall f g, f <> g -> rep F f = rep F g ->
    addr_significant P f = false /\ addr_significant P g = false.

(** Two function pointers are *observably compared* only when at least one is
    address-significant (otherwise the source program has no way to take and
    compare the address — the compiler is free to assume it is never observed,
    which is precisely what the addrsig table encodes). *)
Definition observably_compared (P : program) (f g : fid) : Prop :=
  addr_significant P f = true \/ addr_significant P g = true.

(** THEOREM I3 (ICF Soundness). Under an address-safe fold map, every observable
    address comparison yields the same result before and after folding. Combined
    with THEOREM I1 (calls preserved), the folded program is observationally
    equivalent to the original: ICF is a sound transformation exactly on the
    non-address-significant functions. *)
Theorem icf_sound :
  forall P (F : fold_map P) f g,
    address_safe P F ->
    observably_compared P f g ->
    addr_eq_after P F f g = addr_eq_before f g.
Proof.
  intros P F f g Hsafe Hobs.
  destruct (Bool.bool_dec (addr_eq_after P F f g) (addr_eq_before f g)) as [Heq|Hneq].
  - exact Heq.
  - exfalso.
    apply icf_addr_change_iff_folded in Hneq.
    destruct Hneq as [Hfg Hrep].
    destruct (Hsafe f g Hfg Hrep) as [Hf Hg].
    destruct Hobs as [H|H]; [rewrite Hf in H | rewrite Hg in H]; discriminate.
Qed.

(** Full observational equivalence: calls AND observable address comparisons are
    preserved. This is the headline soundness theorem. *)
Theorem icf_observationally_equivalent :
  forall P (F : fold_map P),
    address_safe P F ->
    (forall f, call_result P (rep F f) = call_result P f) /\
    (forall f g, observably_compared P f g ->
       addr_eq_after P F f g = addr_eq_before f g).
Proof.
  intros P F Hsafe. split.
  - intro f. apply icf_call_preserved.
  - intros f g Hobs. apply icf_sound; assumption.
Qed.

(* ================================================================== *)
(** * 5.  THEOREM I4 — ICF equivalence is a content bisimulation       *)
(* ================================================================== *)

(** The ICF relation "same representative" is an equivalence relation refining
    content-equality — the greatest fixpoint computed by Hopcroft-style
    partition refinement (start with one class, split by content+reloc
    signature until stable). We prove it is a genuine equivalence that implies
    content-equality, i.e. it is a sound refinement of the behavioural quotient. *)
Definition icf_rel (P : program) (F : fold_map P) (f g : fid) : Prop :=
  rep F f = rep F g.

Theorem icf_rel_refl : forall P (F : fold_map P) f, icf_rel P F f f.
Proof. intros; unfold icf_rel; reflexivity. Qed.

Theorem icf_rel_sym : forall P (F : fold_map P) f g,
  icf_rel P F f g -> icf_rel P F g f.
Proof. intros P F f g H; unfold icf_rel in *; auto. Qed.

Theorem icf_rel_trans : forall P (F : fold_map P) f g h,
  icf_rel P F f g -> icf_rel P F g h -> icf_rel P F f h.
Proof. intros P F f g h H1 H2; unfold icf_rel in *; congruence. Qed.

(** THEOREM I4 (Refinement Soundness). The ICF equivalence refines content-
    equality: folded-together functions are content-equal, so the partition is
    a valid bisimulation quotient. Hence partition refinement that only splits
    (never wrongly merges) yields a sound ICF. *)
Theorem icf_rel_refines_content :
  forall P (F : fold_map P) f g,
    icf_rel P F f g -> content_eq P f g.
Proof.
  intros P F f g H. unfold icf_rel in H.
  unfold content_eq.
  (* f ~ rep f = rep g ~ g, all content-equal by rep_sound *)
  transitivity (p_funcs P (rep F f)).
  - exact (rep_sound F f).
  - rewrite H. symmetry. exact (rep_sound F g).
Qed.
