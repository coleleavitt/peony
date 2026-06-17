(* Check the incremental_cost definition behavior *)

Definition byte := nat.

Record section := {
  s_id       : nat;
  s_offset   : nat;
  s_capacity : nat;
  s_content  : list byte
}.

Definition content_eqb (a b : list byte) : bool :=
  if list_eq_dec Nat.eq_dec a b then true else false.

Definition is_red (s s' : section) : bool :=
  negb (content_eqb (s_content s) (s_content s')).

Fixpoint incremental_cost (old new : list section) : nat :=
  match old, new with
  | s :: os, s' :: ns =>
      (if is_red s s' then 1 else 0) + incremental_cost os ns
  | _, _ => 0
  end.

(* Test case: what happens if lists are mismatched? *)
(* Example: old = [s1], new = [s1', s2'] *)
(* incremental_cost matches s1 with s1', then hits the (_, _) case and returns 0. *)
(* So the cost is (if is_red s1 s1' then 1 else 0) + 0 *)

(* This is a critical issue if the capacity_stable_cost_bounded theorem
   assumes equal-length lists but the cost function doesn't validate it *)

