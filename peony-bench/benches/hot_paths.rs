//! Micro-benchmarks of peony's hottest internal operations. These localise a
//! regression to a specific operation; the end-to-end wall-clock harness
//! (`bench/bench.sh`) remains the source of truth for "is peony faster than
//! mold". CodSpeed-compatible (`cargo codspeed run`).

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use peony_symbols::{SymbolTable, fx_hash};

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

criterion_group!(benches, bench_fx_hash, bench_symbol_define_lookup);
criterion_main!(benches);
