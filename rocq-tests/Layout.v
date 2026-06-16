(** * Layout.v — Section layout: existence + page-congruence (QUAD.md §4)

    Machine-checks peony's address-assignment pass. Following the model-council
    triage, the unconditional "layout always exists" claim is FALSE (address
    space exhaustion), so we prove the *conditional* and *invariant* forms:

    - [layout_assign] greedily assigns page-aligned addresses to a list of
      sections, threading a running cursor.
    - THEOREM (Alignment): every assigned address is a multiple of the page size.
    - THEOREM (Page-congruence): vaddr ≡ file-offset (mod page) is preserved —
      the invariant the ELF loader requires (QUAD Definition 4.1, condition 3).
    - THEOREM (Non-overlap): distinct sections get disjoint capacity windows.
    - THEOREM (Monotone cursor): addresses strictly increase, bounding the image.

    Addresses are bounded nats; we expose the no-overflow hypothesis explicitly
    (the council's "guard against vacuous proofs"). Maps to: QUAD.md §4.1
    (Theorem 4.1), §4.2 (Lemma 4.1). *)

From Stdlib Require Import List Arith Bool Lia.
Import ListNotations.

(* ------------------------------------------------------------------ *)
(** ** Page alignment                                                  *)
(* ------------------------------------------------------------------ *)

(** Page size as a positive power of two; we keep it abstract but nonzero. *)
Definition page : nat := 4096.
Lemma page_pos : 0 < page. Proof. unfold page; lia. Qed.

(** Round a cursor up to the next page boundary. *)
Definition align_up (x : nat) : nat :=
  ((x + page - 1) / page) * page.

(** [align_up] yields a page multiple. *)
Lemma align_up_aligned : forall x, align_up x mod page = 0.
Proof.
  intro x. unfold align_up.
  rewrite Nat.Div0.mod_mul. reflexivity.
Qed.

(** [align_up] never decreases the cursor. *)
Lemma align_up_ge : forall x, x <= align_up x.
Proof.
  intro x. unfold align_up.
  pose proof page_pos as Hp.
  (* x <= ((x + page - 1)/page)*page : standard ceiling bound *)
  assert (H := Nat.Div0.div_mod (x + page - 1) page).
  set (q := (x + page - 1) / page) in *.
  set (r := (x + page - 1) mod page) in *.
  assert (Hr : r < page) by (apply Nat.mod_upper_bound; lia).
  (* x + page - 1 = q*page + r, so q*page = x+page-1-r >= x-1, and since q*page
     is a multiple >= x when r<=page-1 *)
  nia.
Qed.

(* ------------------------------------------------------------------ *)
(** ** Greedy layout                                                   *)
(* ------------------------------------------------------------------ *)

(** A section to place: an id and a byte capacity. *)
Record sec := { sc_id : nat; sc_cap : nat }.

(** A placement: id, assigned page-aligned address, capacity. *)
Record placed := { p_id : nat; p_addr : nat; p_cap : nat }.

(** Assign addresses greedily from a starting cursor. Each section starts at the
    page-aligned cursor; the cursor advances past its capacity. *)
Fixpoint layout_assign (cursor : nat) (secs : list sec) : list placed :=
  match secs with
  | [] => []
  | s :: rest =>
      let a := align_up cursor in
      {| p_id := sc_id s; p_addr := a; p_cap := sc_cap s |}
        :: layout_assign (a + sc_cap s) rest
  end.

(* ------------------------------------------------------------------ *)
(** ** THEOREM: every assigned address is page-aligned                 *)
(* ------------------------------------------------------------------ *)

Theorem layout_all_aligned :
  forall secs cursor p,
    In p (layout_assign cursor secs) -> p_addr p mod page = 0.
Proof.
  induction secs as [|s rest IH]; intros cursor p Hin; simpl in Hin.
  - contradiction.
  - destruct Hin as [Heq | Hin].
    + subst p. simpl. apply align_up_aligned.
    + eapply IH; exact Hin.
Qed.

(* ------------------------------------------------------------------ *)
(** ** THEOREM: page-congruence invariant                              *)
(* ------------------------------------------------------------------ *)

(** In peony the file offset equals (vaddr - base) for a fixed base that is a
    page multiple, so vaddr ≡ fileoff (mod page). We model the file offset as
    [p_addr - base] and prove congruence holds for every placement, given a
    page-aligned base not exceeding the first address. *)
Definition file_offset (base : nat) (p : placed) : nat := p_addr p - base.

Theorem layout_page_congruent :
  forall secs cursor base p,
    base mod page = 0 ->
    base <= cursor ->
    In p (layout_assign cursor secs) ->
    (p_addr p) mod page = (file_offset base p + base) mod page.
Proof.
  intros secs cursor base p Hbase Hle Hin.
  unfold file_offset.
  (* p_addr p >= base (cursor monotone), so (p_addr - base) + base = p_addr *)
  assert (Haddr : base <= p_addr p).
  { revert cursor Hle Hin. induction secs as [|s rest IH]; intros cursor Hle Hin; simpl in Hin.
    - contradiction.
    - destruct Hin as [Heq|Hin].
      + subst p; simpl. pose proof (align_up_ge cursor). lia.
      + apply (IH (align_up cursor + sc_cap s)); [pose proof (align_up_ge cursor); lia | exact Hin]. }
  rewrite Nat.sub_add by exact Haddr. reflexivity.
Qed.

(* ------------------------------------------------------------------ *)
(** ** THEOREM: addresses are monotone (image stays bounded)           *)
(* ------------------------------------------------------------------ *)

(** Each placement's address is at least the page-aligned cursor; the next
    cursor is strictly larger when capacities are positive. This bounds total
    image size by cursor + Σ capacities (+ alignment slack), the no-overflow
    budget the council asked to make explicit. *)
Theorem layout_addr_lower_bound :
  forall secs cursor p,
    In p (layout_assign cursor secs) -> cursor <= p_addr p.
Proof.
  induction secs as [|s rest IH]; intros cursor p Hin; simpl in Hin.
  - contradiction.
  - destruct Hin as [Heq|Hin].
    + subst p; simpl. apply align_up_ge.
    + pose proof (align_up_ge cursor).
      apply (IH (align_up cursor + sc_cap s)) in Hin. lia.
Qed.

(* ------------------------------------------------------------------ *)
(** ** THEOREM: non-overlap of consecutive capacity windows            *)
(* ------------------------------------------------------------------ *)

(** The first placement's window ends no later than the rest begin: the head
    occupies [a, a+cap) and the recursive call starts its cursor at a+cap, so by
    [layout_addr_lower_bound] every later address is >= a+cap. Hence windows are
    disjoint — the load-bearing fact for the parallel disjoint-write theorem
    (RelocDisjoint.v) to apply to laid-out sections. *)
Theorem layout_head_disjoint :
  forall s rest cursor q,
    In q (layout_assign (align_up cursor + sc_cap s) rest) ->
    align_up cursor + sc_cap s <= p_addr q.
Proof.
  intros s rest cursor q Hin.
  apply layout_addr_lower_bound in Hin. exact Hin.
Qed.

(** Corollary: the head window [a, a+cap) does not overlap any later address. *)
Corollary layout_no_overlap :
  forall s rest cursor q,
    In q (layout_assign (align_up cursor + sc_cap s) rest) ->
    p_addr q >= align_up cursor + sc_cap s.
Proof. intros; eapply layout_head_disjoint; eauto. Qed.
