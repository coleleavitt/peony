use peony_object::{InputObject, InputSection};
use peony_symbols::SymbolTable;
use rustc_hash::FxHashSet;
mod live;
mod parallel;
mod targets;
#[cfg(test)]
mod tests;

pub use live::LiveSections;
use parallel::{ParallelLevel, collect_parallel_level};
use targets::{GcTargetMaps, GcTargetStats};

// Minimum frontier-section count per worker before the GC BFS fans out across
// ws-deque threads. Set high: spawning a thread scope and idle-spinning on
// futex/sched_yield costs far more than the BFS edge-walk for any link under a
// few thousand live sections. Huge links still parallelize.
//
// NOTE (measured 2026-06-20): the ws-deque parallel path is a net LOSS at any
// triggering grain on real links — `thread::scope` spawns fresh OS threads per
// level and the workers busy-spin (`spin_loop`) on idle detection, so activating
// it on ripgrep's two ~8-9k-section levels REGRESSED the link by ~20ms. Left
// high (effectively serial) until a pooled/parked parallel mark replaces it.
const S3GC_GRAIN_SIZE: usize = 8192;

#[derive(Clone, Copy)]
pub(super) struct GcWorkItem {
    object_id: usize,
    section_index: usize,
    section_pos: usize,
}

#[derive(Clone, Copy)]
pub(super) struct GcContext<'objects, 'targets> {
    objects: &'objects [InputObject],
    targets: &'targets GcTargetMaps<'objects>,
}

impl GcContext<'_, '_> {
    fn item_for_key(&self, key: (usize, usize)) -> Option<GcWorkItem> {
        let obj = self.objects.get(key.0)?;
        let section_pos = obj.section_pos(key.1)?;
        Some(GcWorkItem {
            object_id: key.0,
            section_index: key.1,
            section_pos,
        })
    }

    fn section(&self, item: GcWorkItem) -> Option<&InputSection> {
        self.objects
            .get(item.object_id)?
            .sections
            .get(item.section_pos)
    }

    pub(super) fn collect_targets(&self, item: GcWorkItem, out: &mut Vec<(usize, usize)>) -> usize {
        let Some(sec) = self.section(item) else {
            return 0;
        };
        for reloc in &sec.relocs {
            if let Some(key) = self.targets.get(item.object_id, reloc.symbol.0) {
                out.push(key);
            }
        }
        sec.relocs.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcStats {
    pub roots: u64,
    pub traversed_sections: u64,
    pub scanned_relocs: u64,
    pub live_sections: u64,
    pub target_symbols: u64,
    pub dense_target_objects: u64,
    pub sparse_target_objects: u64,
}

impl GcStats {
    fn with_targets(targets: GcTargetStats) -> Self {
        Self {
            target_symbols: targets.entries as u64,
            dense_target_objects: targets.dense_objects as u64,
            sparse_target_objects: targets.sparse_objects as u64,
            ..Self::default()
        }
    }
}

pub struct GcOutput {
    pub live: LiveSections,
    pub stats: GcStats,
}

struct GcTraversal<'a> {
    objects: &'a [InputObject],
    targets: GcTargetMaps<'a>,
    live: LiveSections,
    frontier: Vec<GcWorkItem>,
    stats: GcStats,
}

impl<'a> GcTraversal<'a> {
    fn new(objects: &'a [InputObject], symbols: &'a SymbolTable) -> Self {
        let targets = GcTargetMaps::new(objects, symbols);
        let stats = GcStats::with_targets(targets.stats());
        Self {
            objects,
            targets,
            live: LiveSections::new(objects),
            frontier: Vec::new(),
            stats,
        }
    }

    fn context(&self) -> GcContext<'a, '_> {
        GcContext {
            objects: self.objects,
            targets: &self.targets,
        }
    }

    fn insert_key(&mut self, key: (usize, usize)) {
        if self.live.insert(key) {
            if let Some(item) = self.context().item_for_key(key) {
                self.frontier.push(item);
            }
        }
    }

    fn insert_root(&mut self, key: (usize, usize)) {
        if self.live.insert(key) {
            if let Some(item) = self.context().item_for_key(key) {
                self.frontier.push(item);
                self.stats.roots += 1;
            }
        }
    }

    fn insert_known_root(&mut self, item: GcWorkItem) {
        if self.live.insert((item.object_id, item.section_index)) {
            self.frontier.push(item);
            self.stats.roots += 1;
        }
    }

    fn seed_roots(&mut self, entry_symbol: &str, export_roots: bool) {
        if let Some(res) = self.targets.symbols().lookup(entry_symbol.as_bytes()) {
            if let (Some(def), Some(si)) = (res.defined_in, res.section_index) {
                self.insert_root((def.0 as usize, si));
            }
        }
        if export_roots {
            let mut roots = Vec::new();
            for (_, res) in self.targets.symbols().iter() {
                if !res.is_export() {
                    continue;
                }
                if let (Some(def), Some(si)) = (res.defined_in, res.section_index) {
                    roots.push((def.0 as usize, si));
                }
            }
            for key in roots {
                self.insert_root(key);
            }
        }
        for (object_id, obj) in self.objects.iter().enumerate() {
            for (section_pos, sec) in obj.sections.iter().enumerate() {
                let keep = sec.name.starts_with(b".init")
                    || sec.name.starts_with(b".fini")
                    || sec.name.starts_with(b".preinit_array")
                    || sec.name.starts_with(b".eh_frame")
                    || sec.name.starts_with(b".gcc_except_table")
                    || (sec.flags & peony_object::elf::SHF_GNU_RETAIN) != 0;
                if keep && sec.flags & peony_object::elf::SHF_ALLOC != 0 {
                    self.insert_known_root(GcWorkItem {
                        object_id,
                        section_index: sec.index.0,
                        section_pos,
                    });
                }
            }
        }
    }

    fn run(mut self) -> GcOutput {
        let max_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        while !self.frontier.is_empty() {
            let pl = max_threads
                .min(self.frontier.len())
                .min(self.frontier.len().div_ceil(S3GC_GRAIN_SIZE))
                .max(1);
            if pl == 1 {
                self.drain_serial_stack();
            } else {
                self.drain_parallel_level(pl);
            }
        }
        self.stats.live_sections = self.live.len() as u64;
        GcOutput {
            live: self.live,
            stats: self.stats,
        }
    }

    fn drain_serial_stack(&mut self) {
        let mut next = Vec::new();
        while let Some(item) = self.frontier.pop() {
            next.clear();
            self.stats.traversed_sections += 1;
            self.stats.scanned_relocs += self.context().collect_targets(item, &mut next) as u64;
            for key in next.iter().copied() {
                self.insert_key(key);
            }
        }
    }

    fn drain_parallel_level(&mut self, pl: usize) {
        let ctx = GcContext {
            objects: self.objects,
            targets: &self.targets,
        };
        let ParallelLevel {
            candidates,
            traversed_sections,
            scanned_relocs,
        } = collect_parallel_level(ctx, &mut self.frontier, pl);
        self.stats.traversed_sections += traversed_sections;
        self.stats.scanned_relocs += scanned_relocs;
        for key in candidates {
            self.insert_key(key);
        }
    }
}

/// Return live `(object_id, input_section_index)` pairs reachable from the
/// entry symbol. `.init*`, `.fini*`, retained sections, and EH metadata are
/// roots in addition to the entry section.
pub fn gc_sections(
    objects: &[InputObject],
    symbols: &SymbolTable,
    entry_symbol: &str,
) -> FxHashSet<(usize, usize)> {
    gc_sections_rooted(objects, symbols, entry_symbol, false)
}

pub fn gc_sections_rooted(
    objects: &[InputObject],
    symbols: &SymbolTable,
    entry_symbol: &str,
    export_roots: bool,
) -> FxHashSet<(usize, usize)> {
    gc_sections_rooted_with_stats(objects, symbols, entry_symbol, export_roots)
        .live
        .into_hash_set()
}

pub fn gc_sections_rooted_with_stats(
    objects: &[InputObject],
    symbols: &SymbolTable,
    entry_symbol: &str,
    export_roots: bool,
) -> GcOutput {
    let mut traversal = GcTraversal::new(objects, symbols);
    traversal.seed_roots(entry_symbol, export_roots);
    traversal.run()
}
