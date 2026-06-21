use std::collections::{BTreeMap, BTreeSet};

use peony_object::InputObject;
use peony_symbols::SymbolTable;

use crate::{
    GcEdgeReasonWitness,
    GcEdgeWitness,
    GcReachabilityWitness,
    GcRootReasonWitness,
    GcRootWitness,
    SectionRefWitness,
    WitnessError,
};

#[cfg(test)]
#[path = "tests/g1.rs"]
mod tests;

pub fn extract_gc_witness(
    objects: &[InputObject],
    symbols: &SymbolTable,
    entry_symbol: &str,
    export_roots: bool,
) -> GcReachabilityWitness {
    let graph = peony_layout::gc_graph_rooted(objects, symbols, entry_symbol, export_roots);
    let rust_live = sorted_sections(peony_layout::gc_sections_rooted(
        objects,
        symbols,
        entry_symbol,
        export_roots,
    ));
    let mut roots: Vec<GcRootWitness> = graph
        .roots
        .into_iter()
        .map(|root| GcRootWitness {
            root: section_ref(root.section),
            reason: root_reason(root.reason),
        })
        .collect();
    roots.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then_with(|| left.reason.cmp(&right.reason))
    });
    let mut edges: Vec<GcEdgeWitness> = graph
        .edges
        .into_iter()
        .map(|edge| GcEdgeWitness {
            from: section_ref(edge.from),
            to: section_ref(edge.to),
            reason: edge_reason(edge.reason),
        })
        .collect();
    edges.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    GcReachabilityWitness {
        roots,
        edges,
        rust_live,
    }
}

pub fn check_gc_witness(
    witness: &GcReachabilityWitness,
) -> Result<Vec<SectionRefWitness>, WitnessError> {
    let reachable = model_gc_reachable(&witness.roots, &witness.edges);
    let rust_live = sorted_unique(&witness.rust_live);
    if reachable == rust_live {
        return Ok(reachable);
    }
    let model_set: BTreeSet<_> = reachable.iter().copied().collect();
    let rust_set: BTreeSet<_> = rust_live.iter().copied().collect();
    Err(WitnessError::GcReachabilityMismatch {
        model_only: model_set.difference(&rust_set).copied().collect(),
        rust_only: rust_set.difference(&model_set).copied().collect(),
    })
}

pub fn model_gc_reachable(
    roots: &[GcRootWitness],
    edges: &[GcEdgeWitness],
) -> Vec<SectionRefWitness> {
    let mut successors: BTreeMap<SectionRefWitness, Vec<SectionRefWitness>> = BTreeMap::new();
    for edge in edges {
        successors.entry(edge.from).or_default().push(edge.to);
    }

    let mut seen = BTreeSet::new();
    let mut stack = Vec::new();
    for root in roots {
        if seen.insert(root.root) {
            stack.push(root.root);
        }
    }
    while let Some(section) = stack.pop() {
        if let Some(next_sections) = successors.get(&section) {
            for next in next_sections {
                if seen.insert(*next) {
                    stack.push(*next);
                }
            }
        }
    }
    seen.into_iter().collect()
}

fn sorted_sections(sections: impl IntoIterator<Item = (usize, usize)>) -> Vec<SectionRefWitness> {
    let mut out: Vec<_> = sections.into_iter().map(section_ref).collect();
    out.sort_unstable();
    out
}

fn sorted_unique(sections: &[SectionRefWitness]) -> Vec<SectionRefWitness> {
    let mut out = sections.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

const fn section_ref(section: (usize, usize)) -> SectionRefWitness {
    SectionRefWitness::new(section.0, section.1)
}

const fn root_reason(reason: peony_layout::GcRootReason) -> GcRootReasonWitness {
    match reason {
        peony_layout::GcRootReason::Entry => GcRootReasonWitness::Entry,
        peony_layout::GcRootReason::Export => GcRootReasonWitness::Export,
        peony_layout::GcRootReason::InitFini => GcRootReasonWitness::InitFini,
        peony_layout::GcRootReason::EhFrame => GcRootReasonWitness::EhFrame,
        peony_layout::GcRootReason::GccExceptTable => GcRootReasonWitness::GccExceptTable,
        peony_layout::GcRootReason::RetainFlag => GcRootReasonWitness::RetainFlag,
    }
}

const fn edge_reason(reason: peony_layout::GcEdgeReason) -> GcEdgeReasonWitness {
    match reason {
        peony_layout::GcEdgeReason::Relocation => GcEdgeReasonWitness::Relocation,
    }
}
