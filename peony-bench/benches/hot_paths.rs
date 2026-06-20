//! Micro-benchmarks of peony's hottest internal operations. These localise a
//! regression to a specific operation; the end-to-end wall-clock harness
//! (`bench/bench.sh`) remains the source of truth for "is peony faster than
//! mold". CodSpeed-compatible (`cargo codspeed run`).

use criterion::{BatchSize, BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use peony_cache::{
    Fingerprint,
    RelocReverseIndex,
    SectionRecord,
    compute_red_green,
    record_link_with_sections,
};
use peony_layout::{
    DynamicInfo,
    HashStyle,
    LayoutConfig,
    SectionFilter,
    TlsGotInfo,
    compute_layout,
    gc_sections,
};
use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};
use peony_reloc::{r_x86_64, scan_relocations};
use peony_symbols::{SymbolTable, fx_hash};
use rustc_hash::FxHashMap;

/// Synthetic symbol names resembling mangled Rust/C++ symbols (the resolver's
/// real input). Deterministic so runs are comparable.
fn gen_names(n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| format!("_ZN4core3fmt9Formatter12pad_integral17h{i:016x}E").into_bytes())
        .collect()
}

/// `fx_hash` is called on every symbol name and every `PreHashed` lookup; it is
/// the single most-executed function in symbol resolution.
fn bench_fx_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("fx_hash");
    for &len in &[8usize, 32, 128] {
        let data = vec![0xABu8; len];
        group.bench_with_input(BenchmarkId::from_parameter(len), &data, |b, d| {
            b.iter(|| fx_hash(black_box(d)));
        });
    }
    group.finish();
}

/// Build a fully-populated symbol table from a name list (the resolve loop).
fn build_table(names: &[Vec<u8>]) -> SymbolTable {
    let mut t = SymbolTable::new();
    for (i, nm) in names.iter().enumerate() {
        t.define_absolute(nm, i as u64);
    }
    t
}

/// Count lookup hits for every name (pure lookup throughput).
fn count_lookups(table: &SymbolTable, names: &[Vec<u8>]) -> u64 {
    let mut hits = 0u64;
    for nm in names {
        if table.lookup(black_box(nm)).is_some() {
            hits += 1;
        }
    }
    hits
}

/// Inserting N defined symbols then looking each up — the core resolve loop.
/// This is what dominates when linking a large object set.
fn bench_symbol_define_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbol_table");
    for &n in &[1_000usize, 10_000] {
        let names = gen_names(n);
        group.bench_with_input(BenchmarkId::new("define", n), &names, |b, names| {
            b.iter(|| black_box(build_table(names)));
        });
        let table = build_table(&names);
        group.bench_with_input(BenchmarkId::new("lookup", n), &names, |b, names| {
            b.iter(|| black_box(count_lookups(&table, names)));
        });
    }
    group.finish();
}

fn synthetic_chain(n: usize, reloc_type: u32) -> (Vec<InputObject>, SymbolTable) {
    let mut sections = Vec::with_capacity(n);
    let mut symbols = Vec::with_capacity(n);
    let mut section_map = FxHashMap::default();
    let mut symbol_map = FxHashMap::default();

    for i in 0..n {
        let sec_index = SectionIndex(i + 1);
        let sym_index = SymbolIndex(i);
        section_map.insert(sec_index.0, i);
        symbol_map.insert(sym_index.0, i);

        let relocs = (i + 1 < n)
            .then(|| InputReloc {
                offset: 0,
                r_type: reloc_type,
                symbol: SymbolIndex(i + 1),
                addend: -4,
            })
            .into_iter()
            .collect();
        sections.push(InputSection {
            index: sec_index,
            name: Name::from(format!(".text.fn{i}").into_bytes()),
            kind: SectionKind::Text,
            sh_type: elf::SHT_PROGBITS,
            data: SectionData::EMPTY,
            align: 16,
            size: 4,
            flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            relocs,
        });

        let name = if i == 0 {
            b"_start".to_vec()
        } else {
            format!("fn_{i}").into_bytes()
        };
        symbols.push(InputSymbol {
            index: sym_index,
            name: Name::from(name),
            binding: Binding::Global,
            is_undefined: false,
            is_common: false,
            is_ifunc: false,
            st_type: elf::STT_FUNC,
            visibility: 0,
            section: Some(sec_index),
            value: 0,
            size: 4,
        });
    }

    let obj = InputObject {
        path: "bench.o".to_string(),
        sections,
        symbols,
        section_map: IndexLookup::Sparse(section_map),
        symbol_map: IndexLookup::Sparse(symbol_map),
        comdat_groups: Vec::new(),
    };
    let mut table = SymbolTable::with_capacity(n);
    let oid = table.add_object(obj.path.clone());
    table.process_object(oid, &obj).expect("synthetic object");
    (vec![obj], table)
}

fn bench_gc_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_sections");
    for &n in &[1_000usize, 10_000] {
        let (objects, symbols) = synthetic_chain(n, r_x86_64::PC32);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| black_box(gc_sections(&objects, &symbols, "_start")));
        });
    }
    group.finish();
}

fn bench_compute_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("compute_layout");
    let arena = InputArena::new();
    let config = LayoutConfig::default();
    let tls = TlsGotInfo::default();

    for &n in &[1_000usize, 10_000] {
        let (objects, symbols) = synthetic_chain(n, r_x86_64::PC32);
        group.bench_with_input(BenchmarkId::new("static_sections", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    compute_layout(
                        &arena,
                        &objects,
                        &symbols,
                        &[],
                        &[],
                        SectionFilter::All,
                        None,
                        &config,
                        &tls,
                    )
                    .expect("synthetic layout"),
                )
            });
        });
    }
    group.finish();
}

fn synthetic_dynamic(imports: usize, needed: usize) -> DynamicInfo {
    DynamicInfo {
        imports: (0..imports)
            .map(|i| format!("imported_symbol_{i}").into_bytes())
            .collect(),
        import_versions: vec![None; imports],
        import_sonames: vec![None; imports],
        needed: (0..needed).map(|i| format!("libbench{i}.so")).collect(),
        interp: Some(b"/lib64/ld-linux-x86-64.so.2".to_vec()),
        rpath: Some("$ORIGIN/../lib".to_string()),
        enable_new_dtags: true,
        hash_style: HashStyle::Both,
        ..DynamicInfo::default()
    }
}

fn bench_dynamic_layout_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("dynamic_layout_metadata");
    let arena = InputArena::new();
    let config = LayoutConfig::default();
    let tls = TlsGotInfo::default();
    let (objects, symbols) = synthetic_chain(256, r_x86_64::GOTPCREL);

    for &(imports, needed) in &[(64usize, 8usize), (512, 32)] {
        let dynamic = synthetic_dynamic(imports, needed);
        group.bench_with_input(
            BenchmarkId::new("imports_needed", imports),
            &(imports, needed),
            |b, _| {
                b.iter(|| {
                    black_box(
                        compute_layout(
                            &arena,
                            &objects,
                            &symbols,
                            &[],
                            &[],
                            SectionFilter::All,
                            Some(&dynamic),
                            &config,
                            &tls,
                        )
                        .expect("synthetic dynamic layout"),
                    )
                });
            },
        );
    }
    group.finish();
}

fn bench_relocation_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("relocation_scan");
    for &n in &[1_000usize, 10_000] {
        let (objects, symbols) = synthetic_chain(n, r_x86_64::GOTPCREL);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| black_box(scan_relocations(&objects, &symbols, false)));
        });
    }
    group.finish();
}

fn bench_reloc_reverse_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("reloc_reverse_index");
    for &(symbols, relocs) in &[(1_000usize, 10_000usize), (10_000, 100_000)] {
        group.bench_with_input(
            BenchmarkId::new("insert_iter", relocs),
            &(symbols, relocs),
            |b, &(symbols, relocs)| {
                b.iter_batched(
                    || RelocReverseIndex::new(symbols, relocs),
                    |idx| {
                        for r in 0..relocs {
                            idx.insert((r % symbols) as u32, r as u32);
                        }
                        black_box(idx.iter_relocs(0).count())
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_red_green_coloring(c: &mut Criterion) {
    let dir = std::env::temp_dir().join(format!("peony-bench-rg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("bench temp dir");
    let output = dir.join("out.elf");
    std::fs::write(&output, vec![0xAAu8; 4096]).expect("bench output");
    let sections: Vec<SectionRecord> = (0u64..512)
        .map(|i| SectionRecord {
            name: format!(".text.{i}"),
            fingerprint: Fingerprint::of_bytes(&i.to_le_bytes()),
            file_offset: i * 8,
            size: 8,
            capacity: 8,
            virtual_address: 0x401000 + (i * 16),
        })
        .collect();
    record_link_with_sections(&output, &[], 0, &sections, &[]).expect("bench manifest");
    let current: Vec<(String, Fingerprint)> = sections
        .iter()
        .map(|s| (s.name.clone(), s.fingerprint))
        .collect();
    let rev = RelocReverseIndex::new(0, 0);

    c.bench_function("red_green_coloring/512_green", |b| {
        b.iter(|| black_box(compute_red_green(&output, &current, &[], &rev, &[]).unwrap()));
    });
}

criterion_group!(
    benches,
    bench_fx_hash,
    bench_symbol_define_lookup,
    bench_gc_bfs,
    bench_compute_layout,
    bench_dynamic_layout_metadata,
    bench_relocation_scan,
    bench_reloc_reverse_index,
    bench_red_green_coloring
);
criterion_main!(benches);
