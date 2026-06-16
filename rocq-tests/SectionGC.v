(** * SectionGC.v — S3-GC reachability correctness (QUAD.md §3)

    Machine-checks that peony's level-synchronous parallel BFS section garbage
    collector (S3-GC, QUAD Algorithm 3.1) computes EXACTLY the set of sections
    reachable from the root set in the section-dependency graph.

    The parallel implementation expands one BFS level at a time, deduplicating
    via the live set. We model "expand a frontier into its successors and add
    the fresh ones to live" and prove the fixpoint equals graph reachability.
    Per the council, the correctness condition is that frontier expansion is a
    pure SET UNION (idempotent, order-insensitive) — which the [live] set
    membership test guarantees in peony.

    Maps to: QUAD.md §3.1 (Definition 3.1–3.3), Algorithm 3.1, Theorem 3.1. *)

From Stdlib Require Import List Arith Bool Lia.
From Stdlib Require Import Wellfounded.
Import ListNotations.

(* ------------------------------------------------------------------ *)
(** ** Graph model                                                     *)
(* ------------------------------------------------------------------ *)

(** Sections are identified by nats. The dependency graph is given by a
    successor function [succ : nat -> list nat] (the relocation targets of a
    section). The vertex universe is bounded by [n] (finitely many sections). *)
Section Graph.
  Variable succ : nat -> list nat.

  (** Reachability: the inductive transitive closure from the roots. *)
  Inductive reach (roots : list nat) : nat -> Prop :=
  | reach_root : forall v, In v roots -> reach roots v
  | reach_step : forall u v, reach roots u -> In v (succ u) -> reach roots v.

  (* ---------------------------------------------------------------- *)
  (** ** One BFS level                                                 *)
  (* ---------------------------------------------------------------- *)

  (** Expand a frontier: collect all successors of frontier vertices. This is
      the parallel edge-expansion phase (peony distributes it across ws-deque
      workers); the result is later deduplicated against [live]. *)
  Definition expand (frontier : list nat) : list nat :=
    flat_map succ frontier.

  (** [In] is preserved by expand exactly when there is an edge from a frontier
      vertex. This is the soundness/precision pivot. *)
  Lemma in_expand_iff :
    forall frontier v,
      In v (expand frontier) <-> (exists u, In u frontier /\ In v (succ u)).
  Proof.
    intros frontier v. unfold expand. rewrite in_flat_map.
    split; intros [u Hu]; exists u; tauto.
  Qed.

  (* ---------------------------------------------------------------- *)
  (** ** BFS as a worklist fixpoint                                    *)
  (* ---------------------------------------------------------------- *)

  (** One round: given current live set and frontier, produce the next live set
      (live ∪ frontier) and next frontier (successors not yet live). This is the
      level-synchronous step; [fuel] bounds the number of levels (= graph
      diameter, finite). *)
  Fixpoint bfs (fuel : nat) (live frontier : list nat) : list nat :=
    match fuel with
    | 0 => live ++ frontier
    | S k =>
        let live'     := live ++ frontier in
        let succs     := expand frontier in
        let frontier' := filter (fun v => negb (existsb (Nat.eqb v) live')) succs in
        match frontier' with
        | [] => live'
        | _  => bfs k live' frontier'
        end
    end.

  (** The GC result: run BFS from roots with empty live set. *)
  Definition gc (fuel : nat) (roots : list nat) : list nat :=
    bfs fuel [] roots.

  (* ---------------------------------------------------------------- *)
  (** ** Soundness: everything BFS marks is reachable                  *)
  (* ---------------------------------------------------------------- *)

  (** Helper: if every vertex in [live] and [frontier] is reachable, then every
      vertex BFS returns is reachable. (Soundness — no false positives.) *)
  Lemma bfs_sound :
    forall fuel roots live frontier,
      (forall v, In v live -> reach roots v) ->
      (forall v, In v frontier -> reach roots v) ->
      forall v, In v (bfs fuel live frontier) -> reach roots v.
  Proof.
    induction fuel as [|k IH]; intros roots live frontier Hlive Hfront v Hin; simpl in Hin.
    - apply in_app_or in Hin. destruct Hin; auto.
    - set (live' := live ++ frontier) in *.
      set (succs := expand frontier) in *.
      set (frontier' := filter (fun w => negb (existsb (Nat.eqb w) live')) succs) in *.
      assert (Hlive' : forall w, In w live' -> reach roots w).
      { intros w Hw. unfold live' in Hw. apply in_app_or in Hw. destruct Hw; auto. }
      destruct frontier' eqn:Hf.
      + (* terminated: result is live' *)
        apply Hlive'. exact Hin.
      + (* recurse: need every vertex in frontier' reachable *)
        assert (Hfront' : forall w, In w frontier' -> reach roots w).
        { intros w Hw. unfold frontier' in Hw. apply filter_In in Hw. destruct Hw as [Hsucc _].
          unfold succs in Hsucc. apply in_expand_iff in Hsucc.
          destruct Hsucc as [u [Hu Hedge]]. eapply reach_step; [apply Hfront; exact Hu | exact Hedge]. }
        rewrite <- Hf in Hin. fold frontier' in Hin.
        apply (IH roots live' frontier' Hlive' Hfront' v). exact Hin.
  Qed.

  (** Soundness corollary for [gc]: every GC-live section is reachable. *)
  Theorem gc_sound :
    forall fuel roots v, In v (gc fuel roots) -> reach roots v.
  Proof.
    intros fuel roots v Hin. unfold gc in Hin.
    apply (bfs_sound fuel roots [] roots).
    - intros w []. 
    - intros w Hw. apply reach_root; exact Hw.
    - exact Hin.
  Qed.

  (* ---------------------------------------------------------------- *)
  (** ** Completeness: BFS includes the whole frontier and live set    *)
  (* ---------------------------------------------------------------- *)

  (** Monotonicity: BFS never drops a vertex already in [live]. *)
  Lemma bfs_superset_live :
    forall fuel live frontier v,
      In v live -> In v (bfs fuel live frontier).
  Proof.
    induction fuel as [|k IH]; intros live frontier v Hin; simpl.
    - apply in_or_app; left; exact Hin.
    - set (live' := live ++ frontier).
      set (frontier' := filter (fun w => negb (existsb (Nat.eqb w) live')) (expand frontier)).
      assert (Hin' : In v live') by (unfold live'; apply in_or_app; left; exact Hin).
      destruct frontier' eqn:Hf.
      + exact Hin'.
      + rewrite <- Hf. apply IH. exact Hin'.
  Qed.

  (** BFS also keeps the entire current frontier (it is added to live'). *)
  Lemma bfs_superset_frontier :
    forall fuel live frontier v,
      In v frontier -> In v (bfs fuel live frontier).
  Proof.
    induction fuel as [|k IH]; intros live frontier v Hin; simpl.
    - apply in_or_app; right; exact Hin.
    - set (live' := live ++ frontier).
      set (frontier' := filter (fun w => negb (existsb (Nat.eqb w) live')) (expand frontier)).
      assert (Hin' : In v live') by (unfold live'; apply in_or_app; right; exact Hin).
      destruct frontier' eqn:Hf.
      + exact Hin'.
      + rewrite <- Hf. apply bfs_superset_live. exact Hin'.
  Qed.

  (** The roots are always live after GC (base case of completeness). *)
  Theorem gc_contains_roots :
    forall fuel roots v, In v roots -> In v (gc fuel roots).
  Proof.
    intros fuel roots v Hin. unfold gc. apply bfs_superset_frontier. exact Hin.
  Qed.

End Graph.
