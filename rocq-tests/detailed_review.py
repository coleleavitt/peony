"""
Detailed logical review of the three Rocq proof files.
Looking for:
1. Over-claiming or vacuous definitions
2. Missing preconditions
3. Mismatches between informal claims and formal statements
"""

issues = []

# IncrementalCostBound.v analysis
issues.append({
    'file': 'IncrementalCostBound.v',
    'line': 86,
    'severity': 'low',
    'category': 'Model definition',
    'issue': 'fromscratch_cost only counts length, not actual cost of sections',
    'detail': 'The definition treats all sections as cost 1 unit (by counting). Real linkers pay variable cost per section. But for incremental vs from-scratch ratio, this is correct: from-scratch pays sum-of-lengths, incremental pays sum-of-red-lengths.',
    'verdict': 'NOT A BUG - The model abstracts section costs as 1 each, which is valid for the comparative bound.'
})

issues.append({
    'file': 'IncrementalCostBound.v',
    'line': 269,
    'severity': 'low',
    'category': 'Deprecated notation',
    'issue': 'seq_length is deprecated, should use length_seq',
    'detail': 'The build produces warnings about seq_length being deprecated since 8.20. This is a style issue, not a correctness issue.',
    'verdict': 'TECHNICAL DEBT - Low priority, cosmetic fix'
})

issues.append({
    'file': 'IncrementalCostBound.v',
    'line': 203,
    'severity': 'medium',
    'category': 'Model claim mismatch',
    'issue': 'single_edit_cost_is_one proves for a specific constructed example, not the general claim',
    'detail': '''The theorem constructs explicit lists `pre` and `post` with the same content, 
then appends a red pair. This proves: "there EXISTS a scenario where incremental=1 and from-scratch=n".
However, the informal claim (line 167-170) says "a single-file edit of an n-section link costs 1".
This requires the OLD list to have equal length to NEW, which is enforced by precondition length pre = length post.
But the theorem also requires that the green pairs match pointwise (Hgreen), which is strong.
The general case is: if exactly one section is red, cost = 1. But this theorem only proves one witness.''',
    'verdict': 'NOT A BUG - The theorem is correctly stated. It proves existence of a separating example, which is sufficient for the claim. A stronger theorem would prove "for any n-section link where all but one section is green, cost = 1", but existence suffices for the asymptotic argument.'
})

issues.append({
    'file': 'ParallelSchedule.v',
    'line': 43,
    'severity': 'low',
    'category': 'Model abstraction',
    'issue': 'task cost as nat treats all costs as discrete units',
    'detail': 'The model uses nat for task costs, not real numbers. This is adequate for abstract scheduling bounds but may lose precision in practice.',
    'verdict': 'NOT A BUG - nat is appropriate for a formal model.'
})

issues.append({
    'file': 'ParallelSchedule.v',
    'line': 191,
    'severity': 'medium',
    'category': 'Definition precision',
    'issue': 'brent_admissible definition uses P * m <= work + P * span',
    'detail': '''This is the SCALED form of Brent''s theorem: P*T_P <= T1 + P*T_inf.
The theorem comment says this is correct (line 194-195).
However, with P=0, the inequality becomes 0 <= work, which is vacuously true.
With P=1, it becomes m <= work + span, which is correct.
The theorem constraints require P >= 1 (line 192), so P=0 case is excluded.''',
    'verdict': 'NOT A BUG - P >= 1 constraint is properly enforced.'
})

issues.append({
    'file': 'ICFSoundness.v',
    'line': 60,
    'severity': 'high',
    'category': 'Logical vacuity',
    'issue': 'fold_map rep_sound constraint allows only content-equal functions to be folded',
    'detail': '''The record fold_map requires rep_sound: forall f, content_eq P f (rep f).
This means a function MUST map to something with identical content.
But the informal claim is "ICF merges functions with identical content".
The formal statement doesn\'t enforce that if two functions have equal content, 
they CAN be folded (only that if they are folded, content must be equal).
The fold_map is just an abstract "representative function", not a proof that 
ICF-as-partition-refinement is sound.
The soundness side-condition address_safe (line 142) only restricts folding,
it doesn\'t prove that partition refinement (the standard ICF algorithm) is optimal.''',
    'verdict': 'NOT A BUG BUT INCOMPLETE - The theorem proves that a given fold_map is sound if address-safe. It does NOT prove that the standard partition-refinement algorithm (Hopcroft) is optimal. Theorem I4 claims to do this (line 25-26), but it only proves icf_rel is an equivalence that refines content-equality. It does not prove I4 is the GREATEST FIXPOINT.'
})

issues.append({
    'file': 'ICFSoundness.v',
    'line': 197,
    'severity': 'medium',
    'category': 'Theorem scope claim',
    'issue': 'icf_rel is defined as "rep F f = rep F g" but the theorem name suggests it\'s the ICF relation itself',
    'detail': '''icf_rel is defined as equality of representatives under a GIVEN fold_map F.
The relation depends on F, not just on program P.
Theorem I4 (line 211-225) proves icf_rel_refines_content, but this is stated
as: "if two functions have the same representative, they have equal content".
The claim in the README (line 94) is: "the ICF equivalence is the greatest fixpoint".
Formal statement: the relation "same representative" refines content-equality.
This is true but does NOT prove greatest-fixpoint property without additional 
assumptions about how F was constructed (e.g., via partition refinement).''',
    'verdict': 'CLAIM MISMATCH - The README claims theorem I4 proves "the ICF equivalence IS the greatest fixpoint". The formal theorem only proves "IF two functions share a representative, they are content-equal". This does not establish the fixpoint characterization without additional theorems about partition-refinement convergence.'
})

for i, issue in enumerate(issues, 1):
    print(f"\n[{i}] {issue['file']} : {issue['category']}")
    print(f"    Line {issue['line']}: {issue['issue']}")
    print(f"    Severity: {issue['severity']}")
    print(f"    Verdict: {issue['verdict']}")

