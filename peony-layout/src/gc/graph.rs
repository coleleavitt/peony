use peony_object::InputSection;

use super::{GcTraversal, GcWorkItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcRootReason {
    Entry,
    Export,
    InitFini,
    EhFrame,
    GccExceptTable,
    RetainFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GcRoot {
    pub section: (usize, usize),
    pub reason: GcRootReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcEdgeReason {
    Relocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GcEdge {
    pub from: (usize, usize),
    pub to: (usize, usize),
    pub reason: GcEdgeReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcGraph {
    pub roots: Vec<GcRoot>,
    pub edges: Vec<GcEdge>,
}

pub(super) fn collect_roots(
    traversal: &GcTraversal<'_>,
    entry_symbol: &str,
    export_roots: bool,
) -> Vec<GcRoot> {
    let mut roots = Vec::new();
    if let Some(res) = traversal.targets.symbols().lookup(entry_symbol.as_bytes())
        && let (Some(def), Some(si)) = (res.defined_in, res.section_index)
    {
        roots.push(GcRoot {
            section: (def.0 as usize, si),
            reason: GcRootReason::Entry,
        });
    }
    if export_roots {
        for (_, res) in traversal.targets.symbols().iter() {
            if !res.is_export() {
                continue;
            }
            if let (Some(def), Some(si)) = (res.defined_in, res.section_index) {
                roots.push(GcRoot {
                    section: (def.0 as usize, si),
                    reason: GcRootReason::Export,
                });
            }
        }
    }
    for (object_id, obj) in traversal.objects.iter().enumerate() {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            visit_implicit_root_reasons(sec, |reason| {
                roots.push(GcRoot {
                    section: (object_id, sec.index.0),
                    reason,
                });
            });
        }
    }
    roots
}

pub(super) fn extract_graph(
    traversal: &GcTraversal<'_>,
    entry_symbol: &str,
    export_roots: bool,
) -> GcGraph {
    let ctx = traversal.context();
    let mut edges = Vec::new();
    let mut targets = Vec::new();
    for (object_id, object) in traversal.objects.iter().enumerate() {
        for (section_pos, section) in object.sections.iter().enumerate() {
            targets.clear();
            let item = GcWorkItem {
                object_id,
                section_index: section.index.0,
                section_pos,
            };
            ctx.collect_targets(item, &mut targets);
            for to in targets.iter().copied() {
                edges.push(GcEdge {
                    from: (object_id, section.index.0),
                    to,
                    reason: GcEdgeReason::Relocation,
                });
            }
        }
    }
    let mut roots = collect_roots(traversal, entry_symbol, export_roots);
    roots.sort_unstable();
    roots.dedup();
    edges.sort_unstable();
    edges.dedup();
    GcGraph { roots, edges }
}

fn visit_implicit_root_reasons(section: &InputSection, mut visit: impl FnMut(GcRootReason)) {
    if section.name.starts_with(b".init")
        || section.name.starts_with(b".fini")
        || section.name.starts_with(b".preinit_array")
    {
        visit(GcRootReason::InitFini);
    }
    if section.name.starts_with(b".eh_frame") {
        visit(GcRootReason::EhFrame);
    }
    if section.name.starts_with(b".gcc_except_table") {
        visit(GcRootReason::GccExceptTable);
    }
    if section.flags & peony_object::elf::SHF_GNU_RETAIN != 0 {
        visit(GcRootReason::RetainFlag);
    }
}
