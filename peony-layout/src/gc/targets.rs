use peony_object::{Binding, InputObject, InputSymbol};
use peony_symbols::SymbolTable;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

const NO_TARGET: u64 = u64::MAX;

#[derive(Clone, Copy, Default)]
pub(super) struct GcTargetStats {
    pub(super) entries: usize,
    pub(super) dense_objects: usize,
    pub(super) sparse_objects: usize,
}

pub(super) struct GcTargetMaps<'a> {
    symbols: &'a SymbolTable,
    objects: Vec<GcObjectTargets>,
    stats: GcTargetStats,
}

enum GcObjectTargets {
    Dense(Vec<u64>),
    Sparse(FxHashMap<usize, u64>),
    Empty,
}

impl<'a> GcTargetMaps<'a> {
    pub(super) fn new(objects: &[InputObject], symbols: &'a SymbolTable) -> Self {
        // Each object's target map is independent (it only reads the immutable
        // object + frozen symbol table), so build them in parallel on large links
        // and fold the per-object stat partials. Output is keyed by object_id, so
        // par_iter's order-preserving collect is bit-for-bit identical to serial.
        const PARALLEL_THRESHOLD: usize = 64;
        let (objects, stats) = if objects.len() >= PARALLEL_THRESHOLD {
            let built: Vec<(GcObjectTargets, GcTargetStats)> = objects
                .par_iter()
                .enumerate()
                .map(|(object_id, obj)| {
                    let mut s = GcTargetStats::default();
                    let t = build_object_targets(object_id, obj, symbols, &mut s);
                    (t, s)
                })
                .collect();
            let mut stats = GcTargetStats::default();
            let mut maps = Vec::with_capacity(built.len());
            for (map, s) in built {
                maps.push(map);
                stats.entries += s.entries;
                stats.dense_objects += s.dense_objects;
                stats.sparse_objects += s.sparse_objects;
            }
            (maps, stats)
        } else {
            let mut stats = GcTargetStats::default();
            let maps = objects
                .iter()
                .enumerate()
                .map(|(object_id, obj)| build_object_targets(object_id, obj, symbols, &mut stats))
                .collect();
            (maps, stats)
        };
        Self {
            symbols,
            objects,
            stats,
        }
    }

    pub(super) fn get(&self, object_id: usize, symbol_index: usize) -> Option<(usize, usize)> {
        match self.objects.get(object_id)? {
            GcObjectTargets::Dense(targets) => unpack(*targets.get(symbol_index)?),
            GcObjectTargets::Sparse(targets) => unpack(*targets.get(&symbol_index)?),
            GcObjectTargets::Empty => None,
        }
    }

    pub(super) fn stats(&self) -> GcTargetStats {
        self.stats
    }

    pub(super) fn symbols(&self) -> &'a SymbolTable {
        self.symbols
    }
}

fn build_object_targets(
    object_id: usize,
    obj: &InputObject,
    symbols: &SymbolTable,
    stats: &mut GcTargetStats,
) -> GcObjectTargets {
    let Some(max_index) = obj.symbols.iter().map(|sym| sym.index.0).max() else {
        return GcObjectTargets::Empty;
    };
    let dense_limit = obj.symbols.len().saturating_mul(4).max(64);
    if max_index <= dense_limit {
        stats.dense_objects += 1;
        let mut targets = vec![NO_TARGET; max_index + 1];
        for sym in &obj.symbols {
            if let Some(target) = resolve_symbol_target(object_id, sym, symbols) {
                targets[sym.index.0] = target;
                stats.entries += 1;
            }
        }
        return GcObjectTargets::Dense(targets);
    }

    stats.sparse_objects += 1;
    let mut targets = FxHashMap::default();
    targets.reserve(obj.symbols.len());
    for sym in &obj.symbols {
        if let Some(target) = resolve_symbol_target(object_id, sym, symbols) {
            targets.insert(sym.index.0, target);
            stats.entries += 1;
        }
    }
    GcObjectTargets::Sparse(targets)
}

fn resolve_symbol_target(
    object_id: usize,
    sym: &InputSymbol,
    symbols: &SymbolTable,
) -> Option<u64> {
    let key = if sym.binding == Binding::Local {
        sym.section.map(|section| (object_id, section.0))?
    } else {
        let res = symbols.lookup(&sym.name)?;
        let def = res.defined_in?;
        (def.0 as usize, res.section_index?)
    };
    pack(key)
}

fn pack(key: (usize, usize)) -> Option<u64> {
    let object_id = u32::try_from(key.0).ok()? as u64;
    let section_index = u32::try_from(key.1).ok()? as u64;
    Some((object_id << 32) | section_index)
}

fn unpack(value: u64) -> Option<(usize, usize)> {
    if value == NO_TARGET {
        return None;
    }
    Some(((value >> 32) as usize, (value & u32::MAX as u64) as usize))
}
