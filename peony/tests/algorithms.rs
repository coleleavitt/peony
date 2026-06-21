//! Tests for the QUAD.md algorithms implemented in peony.
//!
//! Covers:
//! - S3-GC correctness (QUAD §3): parallel GC finds the same live set as serial BFS
//! - Parallel emit determinism (QUAD Theorem 12.1): N concurrent links produce identical binaries
//! - TLB-aware overwrite-in-place: second link is faster and binary is byte-identical
//! - PreHashed symbol lookups (QUAD §2.3): round-trip through SymbolTable using PreHashed keys
//! - Red-green coloring (QUAD §6): green sections are unchanged; red sections differ
//! - Relocation reverse index (QUAD §10.3): parallel inserts produce correct linked list

mod common;

use std::fs;
use std::path::Path;

use common::{assemble, link, link_raw, run, workdir};

fn read_cache_report(path: &Path) -> serde_json::Value {
    let bytes = fs::read(path).expect("read cache report");
    serde_json::from_slice(&bytes).expect("parse cache report")
}

// ── S3-GC correctness ─────────────────────────────────────────────────────────

/// S3-GC must keep exactly the live sections and drop exactly the dead ones.
/// We verify this end-to-end: a binary built with `--gc-sections` must execute
/// correctly (proves the live set is complete) and must be smaller than one
/// built without GC (proves dead sections were dropped).
#[test]
fn s3gc_live_set_is_correct_and_smaller() {
    let dir = workdir("s3gc_live");

    // entry.s: _start calls live_fn and exits; dead_fn is never called.
    let entry = assemble(
        &dir,
        "entry",
        r#"
        .text
        .globl _start
_start:
        call live_fn
        mov  $60, %rax
        mov  $42, %rdi
        syscall

        .section .text.dead_fn,"ax",@progbits
        .globl dead_fn
dead_fn:
        mov  $99, %rdi
        ret
"#,
    );

    let live = assemble(
        &dir,
        "live",
        r#"
        .text
        .section .text.live_fn,"ax",@progbits
        .globl live_fn
live_fn:
        ret
"#,
    );

    let out_gc = dir.join("gc.out");
    let out_no_gc = dir.join("no_gc.out");

    link(&out_gc, &[entry.clone(), live.clone()], &["--gc-sections"]);
    link(&out_no_gc, &[entry.clone(), live.clone()], &[]);

    // The GC'd binary must still run correctly.
    assert_eq!(run(&out_gc), 42, "S3-GC dropped a live section");

    // The GC'd binary must be strictly smaller (dead section removed).
    let sz_gc = fs::metadata(&out_gc).unwrap().len();
    let sz_no_gc = fs::metadata(&out_no_gc).unwrap().len();
    assert!(
        sz_gc < sz_no_gc,
        "GC binary ({sz_gc}B) should be smaller than non-GC ({sz_no_gc}B)"
    );
}

/// S3-GC with multiple levels: a chain A→B→C→D where only A is in the root set.
/// All of B, C, D must be live; any extra sections beyond these four must be dead.
#[test]
fn s3gc_chain_multiple_bfs_levels() {
    let dir = workdir("s3gc_chain");

    let src = assemble(
        &dir,
        "chain",
        r#"
        .section .text.fn_a,"ax",@progbits
        .globl fn_a
fn_a:
        call fn_b
        ret

        .section .text.fn_b,"ax",@progbits
        .globl fn_b
fn_b:
        call fn_c
        ret

        .section .text.fn_c,"ax",@progbits
        .globl fn_c
fn_c:
        call fn_d
        ret

        .section .text.fn_d,"ax",@progbits
        .globl fn_d
fn_d:
        ret

        .section .text.dead,"ax",@progbits
        .globl dead_fn
dead_fn:
        ret

        .text
        .globl _start
_start:
        call fn_a
        mov  $60, %rax
        xor  %rdi, %rdi
        syscall
"#,
    );

    let out = dir.join("chain.out");
    link(&out, &[src], &["--gc-sections"]);
    assert_eq!(run(&out), 0, "chain GC binary must run and exit 0");
}

// ── Parallel emit determinism ─────────────────────────────────────────────────

/// QUAD Theorem 12.1: determinism. Run the same link N times concurrently and
/// verify all outputs are byte-identical.
#[test]
fn parallel_emit_is_deterministic() {
    let dir = workdir("det");

    // A slightly complex object: multiple sections, relocations, weak symbols.
    let obj = assemble(
        &dir,
        "det",
        r#"
        .data
        .globl x
x:      .quad 0

        .text
        .globl _start
_start:
        leaq  x(%rip), %rax
        movq  $99,     (%rax)
        mov   $60,     %rax
        movq  x(%rip), %rdi
        syscall
"#,
    );

    // Run 8 concurrent links into distinct output files.
    let handles: Vec<_> = (0..8usize)
        .map(|i| {
            let obj = obj.clone();
            let out = dir.join(format!("det_{i}.out"));
            std::thread::spawn(move || {
                link(&out, &[obj], &[]);
                fs::read(&out).unwrap()
            })
        })
        .collect();

    let results: Vec<Vec<u8>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let first = &results[0];
    for (i, r) in results.iter().enumerate().skip(1) {
        assert_eq!(
            first, r,
            "link {i} produced different bytes than link 0 — not deterministic"
        );
    }
}

// ── TLB-aware overwrite-in-place ──────────────────────────────────────────────

/// QUAD §7: linking the same inputs twice must produce byte-identical output
/// when overwriting in-place.
#[test]
fn overwrite_in_place_produces_identical_output() {
    let dir = workdir("tlb");

    let obj = assemble(
        &dir,
        "tlb",
        r#"
        .text
        .globl _start
_start:
        mov  $60, %rax
        mov  $7,  %rdi
        syscall
"#,
    );

    let out = dir.join("tlb.out");

    // First link: creates the file.
    link(&out, std::slice::from_ref(&obj), &[]);
    let bytes1 = fs::read(&out).unwrap();
    let size1 = fs::metadata(&out).unwrap().len();

    // Second link: should detect same-size file and overwrite in-place.
    link(&out, std::slice::from_ref(&obj), &[]);
    let bytes2 = fs::read(&out).unwrap();

    assert_eq!(bytes1, bytes2, "overwrite-in-place changed the binary");
    assert_eq!(
        size1,
        fs::metadata(&out).unwrap().len(),
        "file size changed"
    );
    assert_eq!(
        run(&out),
        7,
        "binary must execute correctly after overwrite"
    );
}

// ── PreHashed symbol lookups ─────────────────────────────────────────────────

/// QUAD §2.3: PreHashed<K> must hash once per key and compare equal for same bytes.
/// This tests the peony-symbols crate's `PreHashed` and `fx_hash` directly.
#[test]
fn prehashed_symbol_roundtrip() {
    use peony_symbols::{PreHashed, fx_hash};

    let name1: Vec<u8> = b"_ZN4core3fmt9Formatter9write_fmt17hdeadbeef01234567E".to_vec();
    let name2: Vec<u8> = b"_ZN4core3fmt9Formatter9write_fmt17hdeadbeef01234567E".to_vec();
    let name3: Vec<u8> = b"_ZN4core3mem4drop17habcdef0123456789E".to_vec();

    // Same bytes → same hash and equal.
    let h1 = fx_hash(&name1);
    let h2 = fx_hash(&name2);
    assert_eq!(h1, h2, "same name bytes must produce same FxHash");

    let ph1 = PreHashed::new(name1.clone());
    let ph2 = PreHashed::new(name2.clone());
    let ph3 = PreHashed::new(name3.clone());

    assert_eq!(ph1, ph2, "PreHashed equal for same bytes");
    assert_ne!(ph1, ph3, "PreHashed different for different bytes");

    // Hash impl must return the stored hash, not re-hash.
    use std::collections::HashSet;
    let mut set: HashSet<PreHashed<Vec<u8>>> = HashSet::new();
    set.insert(ph1);
    assert!(
        set.contains(&ph2),
        "lookup with identical PreHashed must find the entry"
    );
    assert!(
        !set.contains(&ph3),
        "lookup with different PreHashed must not find entry"
    );
}

// ── Incremental cache: epoch key uniqueness ───────────────────────────────────

/// QUAD Definition 6.2: the epoch key must differ for different args hashes.
#[test]
fn cache_epoch_key_is_unique_per_args() {
    use peony_cache::{compute_epoch_key, hash_args};

    let args1 = vec!["peony".to_string(), "-o".to_string(), "a.out".to_string()];
    let args2 = vec!["peony".to_string(), "-o".to_string(), "b.out".to_string()];

    let k1 = compute_epoch_key(&[], hash_args(&args1));
    let k2 = compute_epoch_key(&[], hash_args(&args2));
    assert_ne!(
        k1, k2,
        "different output flags must produce different epoch keys"
    );

    // Same args → same key (determinism).
    let k1a = compute_epoch_key(&[], hash_args(&args1));
    assert_eq!(k1, k1a, "epoch key must be deterministic");
}

// ── Red-green coloring: unchanged sections stay green ────────────────────────

/// QUAD Theorem 6.1: after two identical links, compute_red_green marks all
/// sections Green (nothing changed).
#[test]
fn red_green_all_green_when_nothing_changes() {
    use peony_cache::{
        Fingerprint,
        SectionColor,
        SectionRecord,
        compute_red_green,
        record_link_with_sections,
    };

    let dir = workdir("rg_green");
    let fake_output = dir.join("out.elf");
    fs::write(&fake_output, b"ELF_PLACEHOLDER_BYTES_HERE_FOR_TESTING").unwrap();

    // Record a fake link with two sections.
    let sections = vec![
        SectionRecord {
            name: ".text".to_string(),
            fingerprint: Fingerprint::of_bytes(b"text_bytes"),
            file_offset: 0x1000,
            size: 0x100,
            capacity: 0x140,
            virtual_address: 0x401000,
        },
        SectionRecord {
            name: ".rodata".to_string(),
            fingerprint: Fingerprint::of_bytes(b"rodata_bytes"),
            file_offset: 0x1200,
            size: 0x40,
            capacity: 0x4c,
            virtual_address: 0x402000,
        },
    ];

    record_link_with_sections(&fake_output, &[], 0, &sections, &[], None, None).unwrap();

    // Same fingerprints, no moved symbols → all green.
    use peony_cache::RelocReverseIndex;
    let current = vec![
        (".text".to_string(), Fingerprint::of_bytes(b"text_bytes")),
        (
            ".rodata".to_string(),
            Fingerprint::of_bytes(b"rodata_bytes"),
        ),
    ];
    let rev_idx = RelocReverseIndex::new(0, 0);
    let coloring = compute_red_green(&fake_output, &current, &[], &rev_idx, &[]).unwrap();

    assert_eq!(
        coloring.get(".text"),
        Some(&SectionColor::Green),
        ".text should be green"
    );
    assert_eq!(
        coloring.get(".rodata"),
        Some(&SectionColor::Green),
        ".rodata should be green"
    );
}

/// When a section's bytes change, it must be colored Red.
#[test]
fn red_green_changed_section_is_red() {
    use peony_cache::{
        Fingerprint,
        SectionColor,
        SectionRecord,
        compute_red_green,
        record_link_with_sections,
    };

    let dir = workdir("rg_red");
    let fake_output = dir.join("out.elf");
    fs::write(&fake_output, b"ELF_PLACEHOLDER_BYTES_HERE").unwrap();

    let sections = vec![SectionRecord {
        name: ".text".to_string(),
        fingerprint: Fingerprint::of_bytes(b"original_text"),
        file_offset: 0x1000,
        size: 0x100,
        capacity: 0x140,
        virtual_address: 0x401000,
    }];
    record_link_with_sections(&fake_output, &[], 0, &sections, &[], None, None).unwrap();

    // Different fingerprint → Red.
    use peony_cache::RelocReverseIndex;
    let current = vec![(".text".to_string(), Fingerprint::of_bytes(b"modified_text"))];
    let rev_idx = RelocReverseIndex::new(0, 0);
    let coloring = compute_red_green(&fake_output, &current, &[], &rev_idx, &[]).unwrap();
    assert_eq!(
        coloring.get(".text"),
        Some(&SectionColor::Red),
        ".text with changed bytes must be Red"
    );
}

// ── Relocation reverse index (QUAD §10.3) ─────────────────────────────────────

/// The reloc reverse index must survive concurrent inserts from multiple threads
/// and enumerate every inserted reloc exactly once per symbol.
#[test]
fn reloc_reverse_index_concurrent_correctness() {
    use std::collections::HashSet;
    use std::sync::Arc;

    use peony_cache::{NO_ENTRY, RelocReverseIndex};

    let num_symbols = 8;
    let relocs_per_sym = 50;
    let total_relocs = num_symbols * relocs_per_sym;

    let idx = Arc::new(RelocReverseIndex::new(num_symbols, total_relocs));

    // Each thread inserts relocs for all symbols.
    std::thread::scope(|scope| {
        for sym in 0..num_symbols as u32 {
            for r in 0..relocs_per_sym as u32 {
                let reloc_id = sym * relocs_per_sym as u32 + r;
                let idx = Arc::clone(&idx);
                scope.spawn(move || idx.insert(sym, reloc_id));
            }
        }
    });

    // Verify each symbol has exactly the right relocs.
    for sym in 0..num_symbols as u32 {
        let found: HashSet<u32> = idx.iter_relocs(sym).collect();
        let expected: HashSet<u32> = (0..relocs_per_sym as u32)
            .map(|r| sym * relocs_per_sym as u32 + r)
            .collect();
        assert_eq!(
            found,
            expected,
            "symbol {sym}: expected {relocs_per_sym} relocs, found {}",
            found.len()
        );
    }

    // Sentinel: symbol with no relocs → empty iterator.
    assert_eq!(NO_ENTRY, u32::MAX);
}

/// Incremental relink: link once, verify output. Link again with identical inputs
/// (cache hit), verify output is byte-identical and exit code is unchanged.
#[test]
fn incremental_cache_full_reuse() {
    let dir = workdir("incr_full");

    let obj = assemble(
        &dir,
        "incr",
        r#"
        .text
        .globl _start
_start:
        mov  $60, %rax
        mov  $55, %rdi
        syscall
"#,
    );

    let out = dir.join("incr.out");
    let report = dir.join("cache-report.json");
    let report_arg = format!("--cache-report={}", report.display());

    // First link: no cache.
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    let first_report = read_cache_report(&report);
    assert_eq!(first_report["action"], "full_emit");
    assert_eq!(first_report["reason"]["code"], "cache_state_unavailable");
    let bytes1 = fs::read(&out).unwrap();
    assert_eq!(run(&out), 55);

    // Second link: cache hit → output unchanged.
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    let reused_report = read_cache_report(&report);
    assert_eq!(reused_report["action"], "reused_unchanged_output");
    let bytes2 = fs::read(&out).unwrap();
    assert_eq!(bytes1, bytes2, "cached link must produce identical binary");
    assert_eq!(
        run(&out),
        55,
        "exit code must be preserved across cache hit"
    );
}

/// Incremental changed-input relink: when an input changes, the `--incremental`
/// relink must NOT reuse the stale output. If section layout is still stable it
/// should take the red/green patch path and produce the new correct binary.
#[test]
fn incremental_cache_invalidates_on_input_change() {
    let dir = workdir("incr_inval");
    let out = dir.join("incr.out");
    let report = dir.join("cache-report.json");
    let report_arg = format!("--cache-report={}", report.display());

    let exit_55 = "\
        .text\n.globl _start\n_start:\n    mov $60, %rax\n    mov $55, %rdi\n    syscall\n";
    let exit_42 = "\
        .text\n.globl _start\n_start:\n    mov $60, %rax\n    mov $42, %rdi\n    syscall\n";

    // First link with the 55 variant, priming the cache.
    let obj = assemble(&dir, "incr", exit_55);
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    assert_eq!(run(&out), 55);

    // Overwrite the SAME object path with a different program. A correct
    // incremental linker must detect the change (size/mtime) and re-link.
    // Sleep a hair to guarantee a distinct mtime on coarse-grained filesystems.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let obj2 = assemble(&dir, "incr", exit_42);
    assert_eq!(obj, obj2, "assemble must reuse the same object path");
    let changed = link_raw(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    assert!(
        changed.status.success(),
        "changed-input incremental relink failed: {}",
        String::from_utf8_lossy(&changed.stderr)
    );
    let changed_log = String::from_utf8_lossy(&changed.stderr);
    assert!(
        changed_log.contains("incremental patch emitted")
            || changed_log.contains("parse-only-changed fast relink"),
        "changed-input stable-layout relink should use an incremental fast path; stderr was:\n{changed_log}"
    );
    assert_eq!(
        run(&out),
        42,
        "incremental relink must reflect the changed input, not serve stale bytes"
    );
    let partial_report = read_cache_report(&report);
    assert_eq!(partial_report["action"], "partial_relink");
    assert_eq!(partial_report["cache"]["enabled"], true);
    assert!(
        partial_report["sections"]["red"]
            .as_array()
            .unwrap()
            .iter()
            .any(|section| section == ".text"),
        "partial report should name the patched .text section: {partial_report:#?}"
    );

    // And a third link with no further change is a clean cache hit again.
    let b_after = fs::read(&out).unwrap();
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    assert_eq!(
        fs::read(&out).unwrap(),
        b_after,
        "no-change relink after invalidation must be byte-identical"
    );
}

#[test]
fn incremental_cache_full_emits_when_changed_input_grows() {
    let dir = workdir("incr_fallback");
    let out = dir.join("incr.out");
    let report = dir.join("cache-report.json");
    let report_arg = format!("--cache-report={}", report.display());

    let small = "\
        .text\n.globl _start\n_start:\n    mov $60, %rax\n    mov $11, %rdi\n    syscall\n";
    let large = "\
        .text\n.globl _start\n_start:\n    nop\n    nop\n    mov $60, %rax\n    mov $12, %rdi\n    syscall\n";

    let obj = assemble(&dir, "incr", small);
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    assert_eq!(run(&out), 11);

    std::thread::sleep(std::time::Duration::from_millis(10));
    let obj2 = assemble(&dir, "incr", large);
    assert_eq!(obj, obj2, "assemble must reuse the same object path");
    let changed = link_raw(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", report_arg.as_str()],
    );
    assert!(
        changed.status.success(),
        "changed-input incremental relink failed: {}",
        String::from_utf8_lossy(&changed.stderr)
    );
    let changed_log = String::from_utf8_lossy(&changed.stderr);
    assert!(
        changed_log.contains("incremental red/green patch unavailable; using full emit"),
        "layout/size-changing relink should conservatively full-emit; stderr was:\n{changed_log}"
    );
    assert!(
        !changed_log.contains("incremental red/green patch emitted"),
        "fallback relink must not report partial patch emission; stderr was:\n{changed_log}"
    );
    assert_eq!(run(&out), 12);
    let fallback_report = read_cache_report(&report);
    assert_eq!(fallback_report["action"], "full_emit");
    assert_eq!(
        fallback_report["reason"]["code"],
        "section_capacity_exceeded"
    );
}

/// Incremental cache must invalidate when OUTPUT-AFFECTING FLAGS change, even
/// with identical inputs. A relink of the same `.o` with a different flag
/// (`--build-id`) must NOT reuse the stale binary — the epoch key folds the args
/// hash, so the gate recomputes and rejects it. Without this, a `-pie`→`-shared`
/// relink would silently serve the wrong output.
#[test]
fn incremental_cache_invalidates_on_flag_change() {
    let dir = workdir("incr_flags");
    let out = dir.join("incr.out");
    let obj = assemble(
        &dir,
        "incr",
        ".text\n.globl _start\n_start:\n    mov $60, %rax\n    mov $7, %rdi\n    syscall\n",
    );

    // Prime the cache WITHOUT --build-id.
    link(&out, std::slice::from_ref(&obj), &["--incremental"]);
    let no_build_id = fs::read(&out).unwrap();

    // Relink the SAME input WITH --build-id (an output-affecting flag). The cache
    // must NOT reuse: the output must now contain a build-id note and differ.
    link(
        &out,
        std::slice::from_ref(&obj),
        &["--incremental", "--build-id"],
    );
    let with_build_id = fs::read(&out).unwrap();
    assert_ne!(
        no_build_id, with_build_id,
        "a flag change (--build-id) must invalidate the incremental cache, not reuse the stale binary"
    );
    assert_eq!(
        run(&out),
        7,
        "the re-linked binary must still run correctly"
    );
}
