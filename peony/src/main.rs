//! `peony` — driver binary for the incremental parallel ELF linker.
//!
//! ## Usage (ld-compatible subset)
//!
//! ```text
//! peony [OPTIONS] <inputs>...
//!   -o <file>          Output file (default: a.out)
//!   --incremental      Enable the incremental cache
//!   --threads <N>      rayon worker threads (0 = auto)
//!   --base-address <A> First-segment base VA (default 0x400000)
//!   -e, --entry <SYM>  Entry symbol (default _start)
//! ```
//!
//! ## Pipeline (MaskRay's 9-pass model)
//!
//! 1. parse the command line;
//! 2. parse input objects in parallel; expand archives lazily (pass 2);
//! 3. resolve the global symbol table (pass 2);
//! 4. scan relocations to find GOT slots (pass 6);
//! 5. compute layout: sections, segments, headers, `.got`, `.symtab` (passes 5+8);
//! 6. write symbol VAs / GOT addresses back; check for undefined symbols;
//! 7. (incremental) consult/refresh the cache;
//! 8. emit the output ELF (pass 9).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

// A linker allocates intensely (one allocation per symbol/section/reloc) and
// frees almost nothing until exit. mimalloc's thread-local sharded heaps avoid
// the system malloc's mmap/brk churn that otherwise dominates a small link as
// page-faults. mold makes the same choice.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::{Context, Result};
use peony_emit::{EmitConfig, emit_full, emit_partial, emit_partial_objects};
use peony_layout::{
    LayoutConfig,
    ScriptLayout,
    check_undefined,
    finalize_symbols,
    patch_ifunc_plt_relocs,
};
use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    Name,
    iter_archive_members,
    iter_archive_members_matching,
    parse_object,
    parse_owned_member,
};
use peony_reloc::scan_relocations;
use peony_symbols::{SymbolId, SymbolResolution, SymbolTable};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use tracing_subscriber::EnvFilter;

mod args;
mod cache_report;
mod daemon;
mod handoff;
mod inputs;
mod provided_symbols;

use args::{ResolvedInput, cache_key_args, parse_args, print_help};
use cache_report::{CacheOutcome, CacheReportSink, FullEmitReason};
use handoff::{maybe_handoff_lto_plugin, maybe_handoff_relocatable, reject_unsupported_flags};
use inputs::{
    expand_inputs,
    inject_crt_objects,
    library_search_paths,
    parse_linker_script_controls,
    resolve_inputs,
    resolved_input_paths,
    strip_block_comments,
};
use provided_symbols::{predefine_linker_symbols, set_linker_addresses};

// ── Compatibility handoffs ──────────────────────────────────────────────────

fn check_required_defined(symbols: &SymbolTable, required: &[String]) -> Result<()> {
    let missing: Vec<&str> = required
        .iter()
        .map(String::as_str)
        .filter(|name| {
            symbols
                .lookup(name.as_bytes())
                .is_none_or(|r| !r.is_defined())
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!("required symbol(s) not defined: {}", missing.join(", "))
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    if let Ok(filter) = std::env::var("PEONY_LOG") {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(filter))
            .with_target(false)
            .with_writer(std::io::stderr) // diagnostics on stderr, like a real linker
            .try_init()
            .ok();
    }

    let mut args = parse_args()?;
    if args.help {
        print_help();
        return Ok(());
    }
    // Env opt-out for build systems that invoke the linker via `cc`/`ld` and
    // can't pass `--no-incremental`: `PEONY_INCREMENTAL=0` (also `false`/`no`)
    // turns off the (default-on) incremental cache for clean/CI builds.
    if matches!(
        std::env::var("PEONY_INCREMENTAL").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    ) {
        args.incremental = false;
        args.daemon = false;
    }
    let cache_report = CacheReportSink::new(args.cache_report.clone(), args.stats);
    // `--stats`: turn on the in-linker phase profiler so the breakdown table is
    // printed at the end. `--trace` additionally records the call-flow tree
    // (caller→callee by file:line) for following a bug through the pipeline.
    // Both are cheap no-ops for every normal link.
    if args.trace_stack && args.trace_detail {
        peony_prof::trace_stack_detail_enable();
    } else if args.trace_stack {
        peony_prof::trace_stack_enable();
    } else if args.trace_detail {
        peony_prof::trace_detail_enable();
    } else if args.trace {
        peony_prof::trace_enable();
    } else if args.stats {
        peony_prof::enable();
    }
    peony_prof::record_rss("start");
    reject_unsupported_flags(&args)?;
    if maybe_handoff_relocatable(&args)? {
        return Ok(());
    }
    if maybe_handoff_lto_plugin(&args)? {
        return Ok(());
    }
    let script_controls = parse_linker_script_controls(&args.linker_scripts)?;
    if !args.entry_explicit {
        if let Some(entry) = &script_controls.entry {
            args.entry = entry.clone();
        }
    }
    if !args.base_address_explicit {
        if let Some(base) = &script_controls.base_address {
            args.base_address = base.clone();
        }
    }
    // `-shared` wins over `-pie`: a shared object is ET_DYN but is not a PIE
    // (no DF_1_PIE, no PT_INTERP, no entry). Some drivers pass both.
    if args.shared {
        args.pie = false;
    }

    // A PIE or shared object is loaded at a kernel/loader-chosen bias, so it is
    // laid out from base 0 (ET_DYN). Only a fixed-address ET_EXEC uses a base.
    let base_address = if args.pie || args.shared {
        0
    } else {
        parse_hex_or_dec(&args.base_address)
            .with_context(|| format!("invalid --base-address `{}`", args.base_address))?
    };

    // Positional inputs plus `-l<name>` libraries resolved against `-L` paths,
    // with any GNU linker scripts (e.g. libc.so) expanded to real files.
    let inputs = {
        let _p = peony_prof::phase("resolve-inputs");
        let inputs = resolve_inputs(&args)?;
        expand_inputs(inputs, &library_search_paths(&args.library_paths))?
    };
    peony_prof::record_rss("after-inputs");
    // When invoked directly as the linker (e.g. by rustc), the C-runtime startup
    // objects that provide `_start`/`_init` are not passed; inject them as `cc`
    // would for a dynamic/PIE executable. A shared object has no `_start`/crt1.
    let (inputs, injected_crt) = {
        let _p = peony_prof::phase("inject-crt");
        if args.shared {
            (inputs, Vec::new())
        } else {
            inject_crt_objects(inputs, &args)
        }
    };
    let input_paths = resolved_input_paths(&inputs);

    // Incremental fast-path: if every input and the previous output are
    // unchanged since the last link, the existing output is already correct.
    // This runs BEFORE the thread pool is spun up — a no-change relink must be a
    // handful of stat()s, not a full link (the whole point of beating mold on
    // the edit–rebuild loop). The thread pool is only initialised once we know a
    // real link is needed.
    // Hash only output-affecting arguments: changes to diagnostics such as
    // `--stats` or `--cache-report` must not make an otherwise reusable output
    // look dirty, while real linker-mode changes still invalidate the cache.
    let cache_args = cache_key_args(&args.raw_args);
    let args_hash = peony_cache::hash_args(&cache_args);

    // Resident-daemon SERVER: load the incremental cache into RAM and serve
    // relinks until killed. Requires a prior `--incremental` link.
    if args.daemon {
        return daemon::serve(&args.output, &input_paths, args_hash);
    }

    // Resident-daemon CLIENT: if a daemon is serving this output, delegate the
    // relink to it (it holds the layout + symbols in RAM → sub-5ms) and exit.
    // Falls through to the one-shot path if the daemon declines or is absent.
    // With `PEONY_DAEMON=1`, auto-spawn one first (once a cache exists) so the
    // sub-5ms path is automatic in a dev shell.
    if args.incremental {
        daemon::ensure_autospawn(&args.output);
        if let Some(handled) = daemon::try_delegate(&args.output, args_hash) {
            if handled {
                tracing::info!(output = %args.output.display(), "link complete (daemon relink)");
                cache_report.record(&args.output, CacheOutcome::ReusedUnchanged)?;
                return Ok(());
            }
        }
    }

    if args.incremental
        && peony_cache::try_reuse(&args.output, &input_paths, args_hash)
            .context("incremental cache")?
    {
        tracing::info!(output = %args.output.display(), "incremental: inputs unchanged, reused cached output");
        cache_report.record(&args.output, CacheOutcome::ReusedUnchanged)?;
        return Ok(());
    }

    // Incremental parse-only-changed fast path (blueprint Phase 3-4): when only
    // plain object(s) changed size-stably, parse JUST those, reuse the cached
    // layout + symbol manifest, and re-emit in place — skipping the full
    // parse+resolve of the 400+ unchanged inputs. Any ineligibility falls
    // through to the full pipeline below (which still reuses the layout).
    if args.incremental && !args.gc_sections && !args.icf {
        let emit_config = EmitConfig::default();
        let _p = peony_prof::phase("parse-only-changed");
        if try_parse_only_changed(
            &args.output,
            &input_paths,
            args_hash,
            &emit_config,
            &cache_report,
        )? {
            drop(_p);
            tracing::info!(output = %args.output.display(), "link complete (fast relink)");
            peony_prof::report();
            return Ok(());
        }
    }

    init_thread_pool(args.threads, input_paths.len())?;

    let Resolved {
        arena,
        objects,
        mut symbols,
        comdat_excluded,
        needed,
    } = {
        let _p = peony_prof::phase("parse+resolve");
        load_and_resolve_with_crt_fallback(&inputs, &injected_crt, &args.undefined)?
    };
    peony_prof::record_items("parse+resolve", objects.len() as u64);
    peony_prof::count("objects_parsed", objects.len() as u64);
    peony_prof::count("symbols_resolved", symbols.len() as u64);
    peony_prof::record_items("name-intern", arena.interned_name_count() as u64);
    peony_prof::record_bytes("name-intern", arena.interned_name_bytes());
    peony_prof::record_rss("after-parse-resolve");

    // Weak-undefined symbols referenced through the GOT (e.g. `__gmon_start__`)
    // need a real SymbolId so their GOT slot gets a recorded address (holding 0).
    // Assign ids before the scan so the slots are tracked.
    {
        let _p = peony_prof::phase("weak-got-ids");
        peony_reloc::assign_weak_got_ids(&objects, &mut symbols);
    }

    tracing::info!("scanning relocations");
    let scan = {
        let _p = peony_prof::phase("reloc-scan");
        scan_relocations(&objects, &symbols, args.shared)
    };
    let input_relocs: usize = objects
        .iter()
        .map(|obj| {
            obj.sections
                .iter()
                .map(|sec| sec.relocs.len())
                .sum::<usize>()
        })
        .sum();
    peony_prof::count("relocs_scanned", input_relocs as u64);
    peony_prof::record_items("reloc-scan", input_relocs as u64);
    peony_prof::record_rss("after-reloc-scan");
    // Everything from here to the layout span was previously untimed ("other"):
    // GOT/PLT/TLS slot extraction, copy-reloc marking, import/dynsym assignment,
    // export collection and dynamic-section sizing. Attribute it.
    let _postscan = peony_prof::phase("reloc-postproc");
    let got_syms = {
        let _t = peony_prof::trace("reloc-postproc:got-symbols");
        scan.got_symbols()
    };
    let plt_syms = {
        let _t = peony_prof::trace("reloc-postproc:plt-symbols");
        scan.plt_symbols()
    };
    let tls_got = peony_layout::TlsGotInfo {
        gd: {
            let _t = peony_prof::trace("reloc-postproc:tls-gd");
            scan.tls_gd_refs()
        },
        ie: {
            let _t = peony_prof::trace("reloc-postproc:tls-ie");
            scan.tls_ie_refs()
        },
        desc: {
            let _t = peony_prof::trace("reloc-postproc:tls-desc");
            scan.tls_desc_refs()
        },
        ldm: scan.needs_tls_ldm(),
    };
    let copy_relocs = if args.shared {
        Vec::new()
    } else {
        let _t = peony_prof::trace("reloc-postproc:copy-relocs");
        peony_reloc::copy_reloc_symbols(&objects, &symbols)
    };
    let copy_names: Vec<Vec<u8>> = copy_relocs
        .iter()
        .filter_map(|id| symbols.name_by_id(*id).map(|n| n.to_vec()))
        .collect();
    for name in &copy_names {
        symbols.mark_copy_reloc(name);
    }
    tracing::info!(
        got_slots = got_syms.len(),
        plt_slots = plt_syms.len(),
        tls_gd = tls_got.gd.len(),
        tls_desc = tls_got.desc.len(),
        tls_ie = tls_got.ie.len(),
        tls_ldm = tls_got.ldm,
        copy_relocs = copy_relocs.len(),
        "relocation scan complete"
    );
    peony_prof::count("got_slots", got_syms.len() as u64);
    peony_prof::count("plt_slots", plt_syms.len() as u64);
    peony_prof::count("tls_gd_refs", tls_got.gd.len() as u64);
    peony_prof::count("tls_ie_refs", tls_got.ie.len() as u64);
    peony_prof::count("tls_desc_refs", tls_got.desc.len() as u64);
    peony_prof::count("copy_relocs", copy_relocs.len() as u64);

    // Combine the GC live-set with COMDAT deduplication into the section filter
    // the layout will apply.
    let export_roots =
        args.shared || args.export_dynamic || !args.export_dynamic_patterns.is_empty();
    let live = {
        let _t = peony_prof::trace_fields(
            "reloc-postproc:live-sections",
            [
                peony_prof::TraceField::count("objects", objects.len() as u64),
                peony_prof::TraceField::count("comdat_excluded", comdat_excluded.len() as u64),
                peony_prof::TraceField::count("gc", u64::from(args.gc_sections)),
                peony_prof::TraceField::count("export_roots", u64::from(export_roots)),
            ],
        );
        compute_live(
            &objects,
            args.gc_sections,
            &symbols,
            &args.entry,
            &comdat_excluded,
            export_roots,
            Some(&script_controls.layout),
        )
    };
    match &live {
        LiveFilter::Only(l) => {
            tracing::info!(live_sections = l.len(), "section selection complete (gc)")
        }
        LiveFilter::Except(d) => {
            tracing::info!(
                excluded_sections = d.len(),
                "section selection complete (comdat)"
            )
        }
        LiveFilter::All => {}
    }

    // Dynamic mode: any shared-library import → emit a dynamic executable.
    let imports: Vec<Vec<u8>> = {
        let _t = peony_prof::trace("reloc-postproc:imports");
        let mut imports: Vec<Vec<u8>> = symbols
            .iter()
            .filter(|(_, r)| r.import)
            .map(|(n, _)| n.to_vec())
            .collect();
        imports.sort();
        imports
    };
    peony_prof::count("dynamic_imports", imports.len() as u64);
    // Per-import version requirement, parallel to the sorted `imports`.
    let import_versions: Vec<Option<Vec<u8>>> = imports
        .iter()
        .map(|n| symbols.lookup(n).and_then(|r| r.version.clone()))
        .collect();
    let import_sonames: Vec<Option<String>> = imports
        .iter()
        .map(|n| symbols.lookup(n).and_then(|r| r.soname.clone()))
        .collect();
    for (i, name) in imports.iter().enumerate() {
        if let Some(r) = symbols.lookup_mut(name) {
            r.dynsym_index = (i + 1) as u32;
        }
    }
    // A PIE needs R_X86_64_RELATIVE dynamic relocations for absolute pointers,
    // even with no imports. Emit dynamic sections whenever there are imports OR
    // the output is a PIE (rustc/cc default).
    // Both PIE and shared objects are ET_DYN and need R_X86_64_RELATIVE dynamic
    // relocations for absolute pointers the loader must bias.
    let et_dyn = args.pie || args.shared;
    let dynamic_counts = if et_dyn || args.shared {
        let _t = peony_prof::trace("reloc-postproc:dynamic-counts");
        peony_reloc::count_dynamic_relocs(&objects, &symbols, &got_syms, &tls_got, args.shared)
    } else if !imports.is_empty() {
        peony_reloc::DynamicRelocCounts {
            tls: peony_reloc::count_tls_relocs(&objects, &symbols, &tls_got, args.shared),
            ..Default::default()
        }
    } else {
        peony_reloc::DynamicRelocCounts::default()
    };
    let n_relative = dynamic_counts.relative_total;
    let n_irelative = dynamic_counts.irelative;
    // A shared object exports its defined, non-hidden global/weak symbols so
    // `dlsym` can find them. When rustc supplies a `--version-script`, it lists
    // exactly the symbols to export (`global:`) and localizes the rest
    // (`local: *`), so we honour it as an allowlist — otherwise std's thousands
    // of internal globals would all leak into `.dynsym`.
    let version_script = match &args.version_script {
        Some(p) => Some(parse_version_script(p)?),
        None => None,
    };
    let export_requested =
        args.shared || args.export_dynamic || !args.export_dynamic_patterns.is_empty();
    let exports: Vec<peony_layout::ExportSym> = if export_requested {
        let _t = peony_prof::trace("reloc-postproc:exports");
        let mut e: Vec<peony_layout::ExportSym> = symbols
            .iter()
            .filter(|(_, r)| r.is_export())
            .filter(|(_, r)| !excluded_by_exclude_libs(&args.exclude_libs, r, &symbols))
            .filter(|(name, _)| {
                if args.shared {
                    version_script.as_ref().is_none_or(|vs| vs.exports(name))
                } else {
                    args.export_dynamic
                        || export_pattern_matches(&args.export_dynamic_patterns, name)
                }
            })
            .map(|(name, r)| {
                let bind = match r.binding {
                    peony_object::Binding::Weak => peony_object::elf::STB_WEAK,
                    _ => peony_object::elf::STB_GLOBAL,
                };
                peony_layout::ExportSym {
                    name: name.to_vec(),
                    info: peony_object::elf::st_info(bind, r.st_type),
                    other: r.visibility,
                }
            })
            .collect();
        e.sort_by(|a, b| a.name.cmp(&b.name));
        peony_prof::count("dynamic_exports", e.len() as u64);
        e
    } else {
        Vec::new()
    };
    // A shared object's SONAME: explicit -soname, else the output file name.
    let soname = if args.shared {
        Some(args.soname.clone().unwrap_or_else(|| {
            args.output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "a.out".to_string())
        }))
    } else {
        None
    };
    // TLS dynamic relocations the TLS GOT will need (sizes `.rela.dyn`). A shared
    // object needs GD/LD/IE relocs; an executable relaxes GD/LD to Local-Exec but
    // imported IE slots are still loader-filled TPOFF64 entries.
    let n_tls_reloc = dynamic_counts.tls;
    // Symbolic R_X86_64_64 dynamic relocs for data sites referencing an imported
    // symbol (gcc's `.data.rel.local.DW.ref.*` EH slots). Sized here, collected
    // post-layout. Meaningful for any ET_DYN (PIE or shared).
    let n_symbolic_data = dynamic_counts.symbolic_data;
    let rpath = (!args.rpaths.is_empty()).then(|| args.rpaths.join(":"));
    let interp = args.dynamic_linker.as_ref().map(|s| {
        let mut bytes = s.as_bytes().to_vec();
        if !bytes.ends_with(&[0]) {
            bytes.push(0);
        }
        bytes
    });
    // Dynamic sections are needed for any import/needed DSO, any PIE/shared
    // object, or executable dynsym exports.
    let dynamic_needed = !imports.is_empty()
        || !needed.is_empty()
        || et_dyn
        || export_requested
        || rpath.is_some()
        || interp.is_some();
    let dynamic = dynamic_needed.then(|| peony_layout::DynamicInfo {
        imports,
        import_versions,
        import_sonames,
        needed: needed.clone(),
        interp,
        rpath,
        enable_new_dtags: args.enable_new_dtags,
        hash_style: args.hash_style,
        pie: args.pie,
        n_relative,
        n_irelative,
        shared: args.shared,
        soname,
        exports,
        n_tls_reloc,
        copy_relocs,
        n_symbolic_data,
    });
    if dynamic.is_some() {
        tracing::info!(
            needed = needed.len(),
            n_relative,
            shared = args.shared,
            "dynamic object"
        );
    }
    peony_prof::record_rss("after-reloc-postproc");

    // Predefine linker-provided symbols and `--defsym`s BEFORE layout so they are
    // included in `.symtab`; the layout-dependent addresses are filled in after.
    let provided = predefine_linker_symbols(&mut symbols);
    apply_defsym(&mut symbols, &args.defsym)?;

    let config = LayoutConfig {
        base_address,
        entry_symbol: args.entry.clone(),
        build_id: args.build_id,
        strip: args.strip_all,
        strip_debug: args.strip_debug || args.strip_all,
        pie: args.pie,
        shared: args.shared,
        emit_relocs: args.emit_relocs && !args.strip_all,
        script: (!script_controls.layout.output_sections.is_empty())
            .then_some(script_controls.layout.clone()),
        ..Default::default()
    };
    tracing::info!("computing layout");
    // --icf: fold byte+reloc-identical, non-address-significant .text sections.
    let fold_map = if args.icf {
        let fm = peony_layout::icf::compute_fold_map(&arena, &objects);
        if !fm.is_empty() {
            tracing::info!(folded = fm.len(), "ICF: folded identical sections");
        }
        Some(fm)
    } else {
        None
    };
    drop(_postscan);
    let layout_span = peony_prof::phase("layout");
    let layout_trace = peony_prof::trace_fields(
        "layout",
        [
            peony_prof::TraceField::count("objects", objects.len() as u64),
            peony_prof::TraceField::count("symbols", symbols.len() as u64),
            peony_prof::TraceField::count("got", got_syms.len() as u64),
            peony_prof::TraceField::count("plt", plt_syms.len() as u64),
            peony_prof::TraceField::count("dynamic", u64::from(dynamic.is_some())),
        ],
    );
    // Incremental layout-reuse fast path (blueprint Phase 2). When the cached
    // front-end snapshot's drivers reproduce the current front-end exactly, the
    // cached `Layout` is byte-identical to a fresh one, so substitute it and
    // skip `compute_layout`. Hard-gated: `--gc-sections`/`--icf`, a changed
    // non-object input, a drivers mismatch, or a corrupt blob all fall back to a
    // full layout, so reuse is only taken when provably pure.
    let fe_eligible = args.incremental && !args.gc_sections && !args.icf;
    let mut reused_snapshot: Option<peony_cache::FrontEndSnapshot> = None;
    let mut reused_changed_objects: Option<FxHashSet<usize>> = None;
    let mut layout_reused = false;
    let mut layout = match try_reuse_layout(
        &args.output,
        &input_paths,
        args_hash,
        fe_eligible,
        &arena,
        &objects,
        &symbols,
        &got_syms,
        &plt_syms,
        live.as_filter(),
        &config,
        &tls_got,
        fold_map.as_ref(),
    ) {
        Some((cached_layout, snapshot, changed_objects)) => {
            tracing::info!("incremental: reusing cached layout (drivers match)");
            peony_prof::count("layout_reused", 1);
            reused_snapshot = Some(snapshot);
            reused_changed_objects = Some(changed_objects);
            layout_reused = true;
            cached_layout
        }
        None => peony_layout::compute_layout_icf(
            &arena,
            &objects,
            &symbols,
            &got_syms,
            &plt_syms,
            live.as_filter(),
            dynamic.as_ref(),
            &config,
            &tls_got,
            fold_map.as_ref(),
        )
        .context("layout computation failed")?,
    };
    drop(layout_trace);
    drop(layout_span);
    peony_prof::count("layout_sections", layout.output_sections.len() as u64);
    peony_prof::count("layout_segments", layout.segments.len() as u64);
    peony_prof::record_items("layout", layout.output_sections.len() as u64);
    peony_prof::record_bytes("layout", layout.file_size);
    peony_prof::record_rss("after-layout");
    tracing::info!(
        sections = layout.output_sections.len(),
        segments = layout.segments.len(),
        file_size = layout.file_size,
        entry = format_args!("{:#x}", layout.entry),
        "layout complete"
    );

    // Post-layout symbol finalization + dynamic-reloc assembly was previously
    // untimed ("other"): patch linker-symbol addresses, finalize the symtab,
    // and (for ET_DYN) collect RELATIVE/IRELATIVE/TLS/symbolic dynamic relocs.
    let _finalize = peony_prof::phase("finalize-syms");
    set_linker_addresses(&mut symbols, &layout, &provided);
    finalize_symbols(&mut symbols, &layout);
    // Fill `.rela.plt` IRELATIVE addends for direct-call IFUNCs now that resolver
    // VAs are final (see `patch_ifunc_plt_relocs`).
    patch_ifunc_plt_relocs(&mut layout, &symbols);
    peony_prof::record_items("finalize-syms", symbols.len() as u64);
    // A shared object may legitimately reference symbols it does not define;
    // they are resolved at load time against the process image. Only enforce
    // full resolution for executables.
    if !args.shared || args.no_undefined {
        check_undefined(&symbols).context("unresolved symbols")?;
    }
    check_required_defined(&symbols, &args.require_defined)?;

    // Assemble `.rela.dyn` now that symbol VAs are final: the R_X86_64_RELATIVE
    // entries (ET_DYN only) come first, then the GLOB_DATs. For a non-PIE dynamic
    // executable there are no relatives, so this just materialises the GLOB_DATs.
    //
    // SKIP on a layout-reuse relink: the cached `layout.bin` is the FINAL,
    // post-append layout, so `dyn_blobs.rela_dyn` + `tls_got_writes` are already
    // assembled — re-appending would double the dynamic relocs.
    if dynamic.is_some() && !layout_reused {
        // Partition data relocations into RELATIVE (normal), IRELATIVE (IFUNC,
        // resolver run at startup), and symbolic R64 (imported DSO data) in a
        // single post-layout walk. Meaningful for any ET_DYN (PIE or shared); a
        // non-PIE dynamic exe has no base-relative data relocs.
        let (relative, irelative, symbolic) = if et_dyn {
            let _t = peony_prof::trace("finalize:collect-dyn-relocs");
            peony_reloc::collect_data_and_symbolic_relocs(&objects, &symbols, &layout)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        if !irelative.is_empty() {
            tracing::info!(
                ifuncs = irelative.len(),
                "emitting R_X86_64_IRELATIVE relocations"
            );
        }
        // TLS GOT contents: a shared object emits DTPMOD64/DTPOFF64/TPOFF64
        // dynamic relocs (+ static DTPOFF in GD/LDM slot1); an executable writes
        // local IE slots statically but keeps imported IE as loader TPOFF64
        // relocs. Always run when there are TLS GOT slots so exe IE slots get
        // filled or relocated.
        let tls_dyn: Vec<(u64, u32, u32, i64)>;
        if !tls_got.is_empty() {
            let contents =
                peony_reloc::collect_tls_got(&objects, &symbols, &layout, &tls_got, args.shared);
            tracing::info!(
                tls_relocs = contents.relocs.len(),
                tls_static = contents.static_writes.len(),
                shared = args.shared,
                "emitting TLS GOT (GD/LD/IE)"
            );
            layout.tls_got_writes = contents.static_writes;
            tls_dyn = contents.relocs;
        } else {
            tls_dyn = Vec::new();
        }
        // Symbolic R_X86_64_64 dynamic relocs (imported-symbol data sites) were
        // collected above alongside the RELATIVE/IRELATIVE lists.
        if !symbolic.is_empty() {
            tracing::info!(
                symbolic = symbolic.len(),
                "emitting symbolic R_X86_64_64 dynamic relocations"
            );
        }
        layout.append_all_dynamic_relocs(&relative, &irelative, &tls_dyn, &symbolic);
    }
    drop(_finalize);
    peony_prof::record_rss("after-finalize");

    // Snapshot the FINAL (post-append) layout + drivers fingerprint for the next
    // relink's fast path. Captured HERE, after dynamic-reloc assembly, so the
    // cached `layout.bin` already contains the assembled `.rela.dyn`/TLS state —
    // a reuse relink (which SKIPS the append above) emits the correct synthetic
    // sections. On a reuse link `reused_snapshot` is `Some`, so the cached
    // snapshot is reused verbatim and `layout.bin` is not rewritten.
    let (front_end_snapshot, layout_blob) = build_front_end_snapshot(
        fe_eligible,
        &arena,
        &objects,
        &symbols,
        &got_syms,
        &plt_syms,
        live.as_filter(),
        &config,
        &tls_got,
        fold_map.as_ref(),
        reused_snapshot,
        &layout,
    );

    let _emit_span = peony_prof::phase("emit");
    let emit_config = EmitConfig::default();
    let incremental_plan = if args.incremental {
        let _t = peony_prof::trace("incremental:plan");
        incremental_emit_plan(
            &args.output,
            &input_paths,
            args_hash,
            &objects,
            &symbols,
            &layout,
            layout_reused,
        )
        .context("incremental patch planning")?
    } else {
        IncrementalEmitPlan::Disabled
    };
    let mut emitted = false;
    if let IncrementalEmitPlan::Patch(plan) = &incremental_plan {
        tracing::info!(
            red = plan.red_count(),
            green = plan.green_count(),
            "incremental red/green patch plan accepted"
        );
        emitted = if let Some(changed) = &reused_changed_objects {
            // Layout was reused: a drivers match proves every address, synthetic
            // section, header, and unchanged object's bytes are byte-identical to
            // the prior link on disk, so patch ONLY the changed objects'
            // contributions (object-granular, not all of reddened `.text`).
            let changed_std: HashSet<usize> = changed.iter().copied().collect();
            emit_partial_objects(
                &args.output,
                &arena,
                &objects,
                &symbols,
                &layout,
                &emit_config,
                &changed_std,
            )
            .context("incremental object-patch emission failed")?
        } else {
            emit_partial(
                &args.output,
                &arena,
                &objects,
                &symbols,
                &layout,
                &emit_config,
                plan.red_sections(),
            )
            .context("incremental patch emission failed")?
        };
        if emitted {
            tracing::info!(layout_reused, "incremental patch emitted");
            cache_report.record(&args.output, CacheOutcome::PartialRelink { plan })?;
        }
    }
    if !emitted {
        let full_emit_reason = match &incremental_plan {
            IncrementalEmitPlan::Disabled => FullEmitReason::IncrementalDisabled,
            IncrementalEmitPlan::CacheUnavailable => FullEmitReason::CacheStateUnavailable,
            IncrementalEmitPlan::Fallback(reason) => FullEmitReason::PlannerFallback(reason),
            IncrementalEmitPlan::Patch(_) => FullEmitReason::PartialEmitDeclined,
        };
        if args.incremental {
            tracing::info!(
                reason = full_emit_reason.code(),
                "incremental red/green patch unavailable; using full emit"
            );
        }
        emit_full(
            &args.output,
            &arena,
            &objects,
            &symbols,
            &layout,
            &emit_config,
        )
        .context("binary emission failed")?;
        cache_report.record(
            &args.output,
            CacheOutcome::FullEmit {
                reason: full_emit_reason,
            },
        )?;
    }

    if args.incremental {
        let _t = peony_prof::trace("incremental:record");
        let sections = {
            let _t = peony_prof::trace("record:section-records");
            section_records(&layout).unwrap_or_default()
        };
        let cached_symbols = symbol_records(&symbols);
        let _t2 = peony_prof::trace("record:write-manifest");
        peony_cache::record_link_with_sections(
            &args.output,
            &input_paths,
            args_hash,
            &sections,
            &cached_symbols,
            front_end_snapshot.as_ref(),
            layout_blob.as_deref(),
        )
        .context("incremental cache record")?;
    }

    drop(_emit_span);
    peony_prof::record_rss("after-emit");
    tracing::info!(output = %args.output.display(), "link complete");
    peony_prof::report();
    Ok(())
}

/// Capture per-output-section records (offset, size, capacity, content
/// fingerprint, vaddr) from a finished link for the incremental manifest. This
/// is the data an in-place relink needs to decide which sections it may patch
/// without relaying out. Capacity currently equals size (no incremental padding
/// is reserved yet); a future padding pass can widen it. Reads the emitted
/// output once and slices each allocatable section by its file range.
fn section_records(layout: &peony_layout::Layout) -> Option<Vec<peony_cache::SectionRecord>> {
    // Pure walk of the laid-out sections — no output read. The active patch
    // planner (`plan_partial_relink`) consumes only offset/size/capacity/vaddr;
    // the per-section content `fingerprint` fed the now-dead `compute_red_green`
    // path, so we no longer read+hash the whole output to compute it (that was
    // ~1ms of wasted work on every incremental link).
    let mut records = Vec::new();
    for sec in &layout.output_sections {
        // Only file-backed allocatable sections have stable, patchable bytes.
        if sec.sh_type == peony_object::elf::SHT_NOBITS
            || sec.sh_flags & peony_object::elf::SHF_ALLOC == 0
        {
            continue;
        }
        records.push(peony_cache::SectionRecord {
            name: sec.name.clone(),
            fingerprint: peony_cache::Fingerprint::default(),
            file_offset: sec.sh_offset,
            size: sec.sh_size,
            capacity: sec.sh_size, // no padding reserved yet
            virtual_address: sec.sh_addr,
        });
    }
    Some(records)
}

/// Relocation kinds the incremental fast path can re-apply from the MINIMAL
/// cached symbol view (`{virtual_address, got_address, plt_address, size}` +
/// the reused layout). All are PC-relative / GOT / PLT code relocations whose
/// value is a pure function of those fields — no dynamic relocation, no TLS, no
/// IFUNC/COPY. Anything else (absolute `R64`/`32`/`32S` → dynamic RELATIVE in a
/// PIE, all TLS models, SIZE, IRELATIVE, COPY) is descoped to a full link.
fn reloc_apply_simple(r_type: u32) -> bool {
    use peony_reloc::r_x86_64 as r;
    matches!(
        r_type,
        r::PC32
            | r::PLT32
            | r::GOT32
            | r::GOTPCREL
            | r::GOTPCRELX
            | r::REX_GOTPCRELX
            | r::PC16
            | r::PC8
            | r::PC64
            | r::GOTOFF64
            | r::GOTPC32
    )
}

/// An empty placeholder object for the sparse `objects` vec on the parse-only
/// fast path: the changed objects sit at their original ids and everything else
/// is empty. Only the changed objects' contributions are emitted, so these are
/// never read for bytes; `SymIndex::build` iterates their (empty) symbol lists.
fn empty_object(path: String) -> InputObject {
    InputObject {
        path,
        sections: Vec::new(),
        symbols: Vec::new(),
        section_map: IndexLookup::default(),
        symbol_map: IndexLookup::default(),
        comdat_groups: Vec::new(),
    }
}

/// The incremental fast path that parses ONLY the changed object(s) — blueprint
/// Phase 3-4. Runs BEFORE the full parse+resolve. When the cached layout +
/// symbol manifest let us reuse everything and re-apply only the changed
/// object's relocations, it emits in place and returns `Ok(true)` (the caller
/// returns). On ANY ineligibility (no snapshot, object-set drift, digest
/// mismatch, a non-simple relocation, a missing cached symbol, size drift) it
/// returns `Ok(false)` and the caller falls through to the full pipeline (which
/// still reuses the layout via [`try_reuse_layout`]). Never serves stale bytes:
/// the digest match proves the changed object's layout/symbol/reloc demand is
/// unchanged, and the reloc whitelist proves the minimal view can apply it.
fn try_parse_only_changed(
    output: &Path,
    input_paths: &[PathBuf],
    args_hash: u64,
    emit_config: &EmitConfig,
    cache_report: &CacheReportSink,
) -> Result<bool> {
    let Some(cached) = peony_cache::load_changed_state(output, input_paths, args_hash)? else {
        return Ok(false);
    };
    let Some(snap) = cached.front_end.as_ref() else {
        return Ok(false);
    };
    if cached.changed_inputs.is_empty() {
        return Ok(false);
    }
    // Each changed input must be a plain object already in the cached object set.
    let mut changed_ids: Vec<usize> = Vec::new();
    for ci in &cached.changed_inputs {
        match snap.object_paths.iter().position(|p| p == ci) {
            Some(idx) => changed_ids.push(idx),
            None => return Ok(false), // archive/.so/script changed → full link
        }
    }

    // Minimal symbol view from the cached manifest (name → VA/GOT/PLT/size).
    let symbols = build_cached_symbol_view(&cached.symbols);

    // Reuse the cached layout (verify the blob matches the manifest).
    let blob = match peony_cache::read_layout_blob(output)?
        .filter(|b| peony_cache::blob_hash(b) == snap.blob_hash)
    {
        Some(b) => b,
        None => return Ok(false),
    };
    let Some(layout) = peony_layout::deserialize_layout(&blob) else {
        return Ok(false);
    };

    // Shared core: parse the changed objects, verify, and emit them in place.
    let Some(changed_set) = emit_parse_only_changed(
        output,
        &layout,
        &symbols,
        &snap.object_paths,
        &snap.object_digests,
        &changed_ids,
        emit_config,
    )?
    else {
        return Ok(false);
    };

    // Refresh the manifest (the layout blob + symbols are unchanged → reuse the
    // cached snapshot, do not rewrite layout.bin).
    let sections = section_records(&layout).unwrap_or_default();
    peony_cache::record_link_with_sections(
        output,
        input_paths,
        args_hash,
        &sections,
        &cached.symbols,
        cached.front_end.as_ref(),
        None,
    )
    .context("incremental cache record")?;

    tracing::info!(
        objects = changed_set.len(),
        "incremental: parse-only-changed fast relink (skipped full parse+resolve)"
    );
    let patch_sections = patch_sections_for_changed(&layout, &changed_set);
    if let Ok(plan) = peony_cache::plan_partial_relink(
        &cached,
        &patch_sections,
        &[],
        &peony_cache::RelocReverseIndex::new(0, 0),
        &[],
    ) {
        cache_report.record(output, CacheOutcome::PartialRelink { plan: &plan })?;
    }
    Ok(true)
}

/// Build the MINIMAL cached symbol view (name → fabricated resolution carrying
/// VA/GOT/PLT/size) used to re-apply the changed object's relocations without a
/// full re-resolve. Shared by the one-shot disk fast path and the daemon.
pub(crate) fn build_cached_symbol_view(entries: &[peony_cache::CachedSymbolEntry]) -> SymbolTable {
    let mut symbols = SymbolTable::with_capacity(entries.len());
    for e in entries {
        symbols.insert_cached(
            &e.name,
            SymbolResolution::cached_defined(
                e.virtual_address,
                e.got_address,
                e.plt_address,
                e.size,
            ),
        );
    }
    symbols
}

/// The shared parse-only-changed CORE: parse each changed object, verify it is
/// reuse-safe (reloc-complete digest match + every relocation in the simple
/// whitelist with its target resolvable from `symbols`), and emit ONLY its
/// contributions against the reused `layout`. Returns `Some(changed_object_ids)`
/// when emitted, `None` on any ineligibility (caller falls back / full-links).
///
/// Used by BOTH the one-shot disk fast path (`try_parse_only_changed`, which
/// deserializes `layout`/`symbols` from disk each call) and the resident daemon
/// (which supplies them from RAM — the only structural difference, and the whole
/// point of the daemon: skip the per-relink deserialize + symbol-view rebuild).
fn emit_parse_only_changed(
    output: &Path,
    layout: &peony_layout::Layout,
    symbols: &SymbolTable,
    object_paths: &[String],
    object_digests: &[u64],
    changed_ids: &[usize],
    emit_config: &EmitConfig,
) -> Result<Option<HashSet<usize>>> {
    let mut arena = InputArena::new();
    let mut parsed: Vec<(usize, InputObject)> = Vec::new();
    for &idx in changed_ids {
        let Some(path) = object_paths.get(idx) else {
            return Ok(None);
        };
        let obj = match parse_object(&mut arena, Path::new(path)) {
            Ok(o) => o,
            Err(_) => return Ok(None),
        };
        if peony_layout::object_reuse_digest(&arena, &obj, idx)
            != object_digests.get(idx).copied().unwrap_or(u64::MAX)
        {
            return Ok(None); // layout/symbol/reloc demand changed → full link
        }
        for sec in &obj.sections {
            for r in &sec.relocs {
                if !reloc_apply_simple(r.r_type) {
                    return Ok(None);
                }
                if let Some(s) = obj.symbol_by_index(r.symbol.0) {
                    if s.binding != Binding::Local
                        && !s.name.is_empty()
                        && symbols.lookup(&s.name).is_none()
                    {
                        return Ok(None);
                    }
                }
            }
        }
        parsed.push((idx, obj));
    }

    // Sparse object set: changed objects at their original ids, empties else.
    let mut objects: Vec<InputObject> = object_paths.iter().cloned().map(empty_object).collect();
    let mut changed_set: HashSet<usize> = HashSet::with_capacity(parsed.len());
    for (idx, obj) in parsed {
        objects[idx] = obj;
        changed_set.insert(idx);
    }

    let emitted = emit_partial_objects(
        output,
        &arena,
        &objects,
        symbols,
        layout,
        emit_config,
        &changed_set,
    )?;
    if !emitted {
        return Ok(None);
    }
    Ok(Some(changed_set))
}

/// Red/green section records for the `--cache-report`, computed from the changed
/// object-id set alone (no objects vec needed): an Input output section is "red"
/// iff a changed object contributes to it; synthetics are byte-identical.
fn patch_sections_for_changed(
    layout: &peony_layout::Layout,
    changed: &HashSet<usize>,
) -> Vec<peony_cache::PatchSectionRecord> {
    layout
        .output_sections
        .iter()
        .filter(|sec| {
            sec.sh_type != peony_object::elf::SHT_NOBITS
                && sec.sh_flags & peony_object::elf::SHF_ALLOC != 0
        })
        .map(|sec| {
            let input_changed = matches!(sec.source, peony_layout::SecSource::Input)
                && sec
                    .contributions
                    .iter()
                    .any(|c| changed.contains(&c.object_id));
            peony_cache::PatchSectionRecord {
                name: sec.name.clone(),
                file_offset: sec.sh_offset,
                size: sec.sh_size,
                virtual_address: sec.sh_addr,
                input_changed,
            }
        })
        .collect()
}

/// Attempt the incremental layout-reuse fast path (blueprint Phase 2).
///
/// Returns the cached `Layout` plus the freshly-computed fingerprint when the
/// reuse is provably safe; `None` (→ full `compute_layout`) on any of: not
/// eligible, no/version-mismatched cached snapshot, object-set drift, a changed
/// non-object input (archive/shared-lib/script), a drivers mismatch, or a
/// corrupt layout blob. The byte-identity gate is the freshly-recomputed
/// `drivers_hash`: it folds every input `compute_layout` reads, so a match
/// proves the cached layout equals a fresh one.
#[allow(clippy::too_many_arguments)]
fn try_reuse_layout(
    output: &Path,
    input_paths: &[PathBuf],
    args_hash: u64,
    eligible: bool,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    got_syms: &[SymbolId],
    plt_syms: &[SymbolId],
    live: peony_layout::SectionFilter<'_>,
    config: &LayoutConfig,
    tls_got: &peony_layout::TlsGotInfo,
    fold_map: Option<&peony_layout::icf::FoldMap>,
) -> Option<(
    peony_layout::Layout,
    peony_cache::FrontEndSnapshot,
    FxHashSet<usize>,
)> {
    if !eligible {
        return None;
    }
    let Some(cached) = peony_cache::load_changed_state(output, input_paths, args_hash)
        .ok()
        .flatten()
    else {
        tracing::debug!("reuse-gate: no cached changed-state");
        return None;
    };
    let Some(snap) = cached.front_end.as_ref() else {
        tracing::debug!("reuse-gate: no front_end snapshot in manifest");
        return None;
    };
    // Object set must match exactly (paths + order); object ids index `objects`.
    if snap.object_paths.len() != objects.len()
        || snap
            .object_paths
            .iter()
            .zip(objects)
            .any(|(p, o)| p != &o.path)
    {
        tracing::debug!(
            cached = snap.object_paths.len(),
            current = objects.len(),
            "reuse-gate: object set mismatch"
        );
        return None;
    }
    // Every changed input must be a plain object already present in `objects`.
    // A changed archive/shared-lib/script is descoped (it could pull new members
    // or alter dynamic state) → full-link.
    let mut changed_object_ids: FxHashSet<usize> = FxHashSet::default();
    for changed in &cached.changed_inputs {
        match objects.iter().position(|o| &o.path == changed) {
            Some(idx) => {
                changed_object_ids.insert(idx);
            }
            None => {
                tracing::debug!(input = %changed, "reuse-gate: changed input is not a plain object");
                return None;
            }
        }
    }
    let fp = {
        let _t = peony_prof::trace("reuse:fingerprint");
        peony_layout::compute_layout_fingerprint(
            arena,
            objects,
            symbols,
            got_syms,
            plt_syms,
            live,
            config,
            tls_got,
            fold_map,
            &changed_object_ids,
            Some(&snap.object_digests),
        )
    };
    if fp.drivers_hash != snap.drivers_hash {
        tracing::debug!(
            current = fp.drivers_hash,
            cached = snap.drivers_hash,
            "reuse-gate: drivers hash mismatch"
        );
        return None;
    }
    // Read the cached layout from its own file and verify it matches the
    // manifest's `blob_hash` (guards against a stale blob from a crash between
    // writing `layout.bin` and the manifest).
    let blob = {
        let _t = peony_prof::trace("reuse:read-blob");
        peony_cache::read_layout_blob(output).ok().flatten()?
    };
    {
        let _t = peony_prof::trace("reuse:blob-hash");
        if peony_cache::blob_hash(&blob) != snap.blob_hash {
            tracing::debug!("reuse-gate: layout.bin hash mismatch (stale blob)");
            return None;
        }
    }
    let layout = {
        let _t = peony_prof::trace("reuse:deserialize");
        match peony_layout::deserialize_layout(&blob) {
            Some(l) => l,
            None => {
                tracing::debug!("reuse-gate: layout blob failed to deserialize");
                return None;
            }
        }
    };
    // The drivers matched, so the cached snapshot is still valid for the NEXT
    // relink — hand it back so recording persists the small manifest verbatim
    // and skips rewriting `layout.bin` entirely. `changed_object_ids` drives the
    // object-granular fast emit (only these objects' bytes need rewriting).
    Some((layout, cached.front_end.unwrap(), changed_object_ids))
}

/// Build the persisted front-end snapshot for the next relink, or `None` when
/// reuse is not in scope. Reuses the fingerprint a successful fast path already
/// computed; otherwise digests every object from scratch.
#[allow(clippy::too_many_arguments)]
fn build_front_end_snapshot(
    eligible: bool,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    got_syms: &[SymbolId],
    plt_syms: &[SymbolId],
    live: peony_layout::SectionFilter<'_>,
    config: &LayoutConfig,
    tls_got: &peony_layout::TlsGotInfo,
    fold_map: Option<&peony_layout::icf::FoldMap>,
    reused_snapshot: Option<peony_cache::FrontEndSnapshot>,
    layout: &peony_layout::Layout,
) -> (Option<peony_cache::FrontEndSnapshot>, Option<Vec<u8>>) {
    if !eligible {
        return (None, None);
    }
    // Reuse path: the cached snapshot is still valid (drivers matched), so
    // persist its metadata verbatim and DON'T rewrite `layout.bin` (`None` blob)
    // — the cached blob is byte-identical.
    if let Some(snapshot) = reused_snapshot {
        return (Some(snapshot), None);
    }
    // Full/first link: digest every object and serialize the fresh layout into
    // its own blob file.
    let fp = peony_layout::compute_layout_fingerprint(
        arena,
        objects,
        symbols,
        got_syms,
        plt_syms,
        live,
        config,
        tls_got,
        fold_map,
        &FxHashSet::default(),
        None,
    );
    let blob = peony_layout::serialize_layout(layout);
    let meta = peony_cache::FrontEndSnapshot {
        drivers_hash: fp.drivers_hash,
        blob_hash: peony_cache::blob_hash(&blob),
        object_digests: fp.object_digests,
        object_paths: objects.iter().map(|o| o.path.clone()).collect(),
    };
    (Some(meta), Some(blob))
}

enum IncrementalEmitPlan {
    Disabled,
    CacheUnavailable,
    Fallback(peony_cache::PartialRelinkFallback),
    Patch(peony_cache::PartialRelinkPlan),
}

fn incremental_emit_plan(
    output: &Path,
    input_paths: &[PathBuf],
    args_hash: u64,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &peony_layout::Layout,
    layout_reused: bool,
) -> Result<IncrementalEmitPlan> {
    let Some(cached) = peony_cache::load_changed_state(output, input_paths, args_hash)? else {
        return Ok(IncrementalEmitPlan::CacheUnavailable);
    };
    let patch_sections = patch_section_records(layout, objects, &cached.changed_inputs);
    // When the layout was reused, the drivers fingerprint proved every address
    // (and hence every symbol VA) is byte-identical to the cached link, so NO
    // symbol can have moved. Skip the 32k-reloc reverse-index build entirely
    // (~5ms) and color purely by which output sections a changed input feeds.
    let (moved_symbols, rev_index, reloc_sections);
    if layout_reused {
        moved_symbols = Vec::new();
        rev_index = peony_cache::RelocReverseIndex::new(0, 0);
        reloc_sections = Vec::new();
    } else {
        moved_symbols = moved_symbol_ids(&cached.symbols, symbols);
        let (idx, secs) = relocation_reverse_index(objects, symbols, layout);
        rev_index = idx;
        reloc_sections = secs;
    }
    let reloc_section_refs: Vec<&str> = reloc_sections.iter().map(String::as_str).collect();
    Ok(
        match peony_cache::plan_partial_relink(
            &cached,
            &patch_sections,
            &moved_symbols,
            &rev_index,
            &reloc_section_refs,
        ) {
            Ok(plan) => IncrementalEmitPlan::Patch(plan),
            Err(reason) => IncrementalEmitPlan::Fallback(reason),
        },
    )
}

fn patch_section_records(
    layout: &peony_layout::Layout,
    objects: &[InputObject],
    changed_inputs: &[String],
) -> Vec<peony_cache::PatchSectionRecord> {
    let changed_inputs: HashSet<&str> = changed_inputs.iter().map(String::as_str).collect();
    let mut records = Vec::new();
    for sec in &layout.output_sections {
        if sec.sh_type == peony_object::elf::SHT_NOBITS
            || sec.sh_flags & peony_object::elf::SHF_ALLOC == 0
        {
            continue;
        }
        let input_changed = match sec.source {
            peony_layout::SecSource::Input => sec.contributions.iter().any(|c| {
                objects
                    .get(c.object_id)
                    .is_some_and(|obj| object_from_changed_input(&obj.path, &changed_inputs))
            }),
            _ => true,
        };
        records.push(peony_cache::PatchSectionRecord {
            name: sec.name.clone(),
            file_offset: sec.sh_offset,
            size: sec.sh_size,
            virtual_address: sec.sh_addr,
            input_changed,
        });
    }
    records
}

fn object_from_changed_input(path: &str, changed_inputs: &HashSet<&str>) -> bool {
    changed_inputs.iter().any(|input| {
        path == *input
            || path
                .strip_prefix(input)
                .is_some_and(|rest| rest.starts_with('('))
    })
}

fn moved_symbol_ids(cached: &[peony_cache::CachedSymbolEntry], symbols: &SymbolTable) -> Vec<u32> {
    cached
        .iter()
        .filter_map(|entry| {
            let current = symbols.lookup(&entry.name)?;
            (current.virtual_address != entry.virtual_address
                || current.got_address != entry.got_address)
                .then_some(current.id.0)
        })
        .collect()
}

fn relocation_reverse_index(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &peony_layout::Layout,
) -> (peony_cache::RelocReverseIndex, Vec<String>) {
    let reloc_count: usize = layout
        .output_sections
        .iter()
        .filter(|sec| sec.source == peony_layout::SecSource::Input)
        .flat_map(|sec| &sec.contributions)
        .filter_map(|c| {
            objects
                .get(c.object_id)
                .and_then(|obj| obj.sections.get(c.section_pos))
        })
        .map(|sec| sec.relocs.len())
        .sum();
    let index = peony_cache::RelocReverseIndex::new(symbols.len(), reloc_count);
    let mut reloc_sections = Vec::with_capacity(reloc_count);
    for sec in &layout.output_sections {
        if sec.source != peony_layout::SecSource::Input {
            continue;
        }
        for c in &sec.contributions {
            let Some(obj) = objects.get(c.object_id) else {
                continue;
            };
            let Some(input_sec) = obj.sections.get(c.section_pos) else {
                continue;
            };
            for reloc in &input_sec.relocs {
                let reloc_id = reloc_sections.len() as u32;
                reloc_sections.push(sec.name.clone());
                if let Some(sym) = obj.symbol_by_index(reloc.symbol.0) {
                    if sym.binding != Binding::Local {
                        if let Some(res) = symbols.lookup(&sym.name) {
                            index.insert(res.id.0, reloc_id);
                        }
                    }
                }
            }
        }
    }
    (index, reloc_sections)
}

fn symbol_records(symbols: &SymbolTable) -> Vec<peony_cache::CachedSymbolEntry> {
    symbols
        .iter()
        .filter(|(_, res)| res.id.0 != u32::MAX)
        .map(|(name, res)| peony_cache::CachedSymbolEntry {
            name: name.to_vec(),
            virtual_address: res.virtual_address,
            got_address: res.got_address,
            plt_address: res.plt_address,
            size: res.size,
        })
        .collect()
}

// ── Loading + resolution ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct ParseStats {
    sections: u64,
    alloc_sections: u64,
    debug_sections: u64,
    symbols: u64,
    relocs: u64,
    input_bytes: u64,
    owned_buffers: u64,
    owned_bytes: u64,
}

impl ParseStats {
    fn from_parsed(obj: &InputObject, input_bytes: usize, owned: &[Vec<u8>]) -> Self {
        let mut stats = Self {
            sections: usize_to_trace_count(obj.sections.len()),
            symbols: usize_to_trace_count(obj.symbols.len()),
            input_bytes: usize_to_trace_count(input_bytes),
            owned_buffers: usize_to_trace_count(owned.len()),
            owned_bytes: owned
                .iter()
                .map(|buf| usize_to_trace_count(buf.len()))
                .sum(),
            ..Self::default()
        };
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC != 0 {
                stats.alloc_sections += 1;
            }
            if sec.kind == peony_object::SectionKind::Debug {
                stats.debug_sections += 1;
            }
            stats.relocs = stats
                .relocs
                .saturating_add(usize_to_trace_count(sec.relocs.len()));
        }
        stats
    }

    fn add(&mut self, other: Self) {
        self.sections = self.sections.saturating_add(other.sections);
        self.alloc_sections = self.alloc_sections.saturating_add(other.alloc_sections);
        self.debug_sections = self.debug_sections.saturating_add(other.debug_sections);
        self.symbols = self.symbols.saturating_add(other.symbols);
        self.relocs = self.relocs.saturating_add(other.relocs);
        self.input_bytes = self.input_bytes.saturating_add(other.input_bytes);
        self.owned_buffers = self.owned_buffers.saturating_add(other.owned_buffers);
        self.owned_bytes = self.owned_bytes.saturating_add(other.owned_bytes);
    }

    fn trace_fields(&self, objects: usize) -> [peony_prof::TraceField; 9] {
        [
            peony_prof::TraceField::count("objects", usize_to_trace_count(objects)),
            peony_prof::TraceField::count("sections", self.sections),
            peony_prof::TraceField::count("alloc_sections", self.alloc_sections),
            peony_prof::TraceField::count("debug_sections", self.debug_sections),
            peony_prof::TraceField::count("symbols", self.symbols),
            peony_prof::TraceField::count("relocs", self.relocs),
            peony_prof::TraceField::bytes("mapped", self.input_bytes),
            peony_prof::TraceField::count("owned_buffers", self.owned_buffers),
            peony_prof::TraceField::bytes("owned", self.owned_bytes),
        ]
    }
}

fn usize_to_trace_count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Run [`load_and_resolve`]; if it fails only because the inputs supply their own
/// `_start` (colliding with the auto-injected CRT `Scrt1.o`), re-link without the
/// injected CRT objects. This is the freestanding-PIE case (e.g. a hand-written
/// `_start`). Injecting CRT optimistically and retrying on the rare conflict
/// avoids pre-scanning every input for `_start` before parse — a per-object cost
/// that dominated startup on large `cc`-driven links. The common case (no user
/// `_start`) never retries; the freestanding case re-parses a handful of objects.
fn load_and_resolve_with_crt_fallback(
    inputs: &[ResolvedInput],
    injected_crt: &[PathBuf],
    forced_undefined: &[String],
) -> Result<Resolved> {
    let first = load_and_resolve(inputs, forced_undefined);
    let Err(err) = first else {
        return first;
    };
    if injected_crt.is_empty() || !is_user_start_vs_crt_conflict(&err) {
        return Err(err);
    }
    tracing::info!(
        "inputs define their own `_start`; relinking without auto-injected CRT startup objects"
    );
    let user_only: Vec<ResolvedInput> = inputs
        .iter()
        .filter(|i| !injected_crt.contains(&i.path))
        .cloned()
        .collect();
    load_and_resolve(&user_only, forced_undefined)
}

/// True if `err` is a duplicate-`_start` resolution error. With CRT injected, the
/// only `_start` provider among the injected objects is `Scrt1.o`, so a `_start`
/// duplicate means the user provided its own — the signal to drop CRT. (A genuine
/// user/user `_start` clash re-surfaces on the CRT-free retry and propagates.)
fn is_user_start_vs_crt_conflict(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<peony_symbols::SymbolError>(),
        Some(peony_symbols::SymbolError::DuplicateSymbol { name, .. }) if name == "_start"
    )
}

/// Parse all bare objects (in parallel) and pull in archive members lazily,
/// returning the object list and the resolved global symbol table.
///
/// Object indices in the returned `Vec` match the [`peony_symbols::ObjectId`]s
/// assigned during resolution (lock-step `add_object` + `push`).
fn load_and_resolve(inputs: &[ResolvedInput], forced_undefined: &[String]) -> Result<Resolved> {
    let _t = peony_prof::trace("load_and_resolve");
    let mut r = Resolver::default();

    // Classify each input ONCE by reading only its leading magic bytes. The old
    // code called `is_archive` + `is_shared_object` in three separate filters,
    // re-opening (and `is_shared_object` formerly re-reading whole) every input
    // up to 3× — on a 419-object link that was ~3950 openat calls. One pass.
    use peony_object::{FileKind as Kind, MappedInput};
    // mmap each DISTINCT input ONCE, then reuse the mapping for classification
    // AND parsing — instead of opening each file 2× (classify header-read +
    // parse mmap). Bare objects are parsed straight from the mapped bytes via
    // parse_bytes (no second open). Archives/shared keep their own readers (they
    // need member iteration / whole-file reads). cc/rustc repeat archives like
    // `-lgcc` 4×, so the map is keyed by distinct path.
    let mut mmap_cache: FxHashMap<&Path, Option<MappedInput>> = FxHashMap::default();
    for input in inputs {
        let p = input.path.as_path();
        mmap_cache.entry(p).or_insert_with(|| MappedInput::open(p));
    }
    let kinds: Vec<Kind> = {
        let _t = peony_prof::trace("classify-inputs");
        let kinds: Vec<Kind> = inputs
            .iter()
            .map(|input| {
                match mmap_cache
                    .get(input.path.as_path())
                    .and_then(|m| m.as_ref())
                {
                    // Classify from the already-mapped bytes — zero extra syscalls.
                    Some(m) => peony_object::classify_bytes(m.bytes()),
                    // Unmappable (e.g. empty file): fall back to a header read.
                    None => peony_object::classify_file(&input.path),
                }
            })
            .collect();
        peony_prof::event(
            "classified",
            format!("{} inputs, {} distinct", inputs.len(), mmap_cache.len()),
        );
        kinds
    };

    // ── Bare objects: parallel parse, then serial resolve in input order ─────
    let bare: Vec<&PathBuf> = inputs
        .iter()
        .zip(&kinds)
        .filter(|(input, k)| **k == Kind::Bare && !input.start_lib_member)
        .map(|(input, _)| &input.path)
        .collect();
    tracing::info!(objects = bare.len(), "parsing input objects");
    // Small links (the common `cc`/incremental case) parse faster serially:
    // touching rayon's global pool spins up a worker per core that then idles
    // on `sched_yield`/`futex` for longer than the handful of parses take. Only
    // fan out once there are enough objects to amortize the thread management.
    // Raised well above the small-link regime: rayon's global pool spawns one
    // worker per core, and once touched those workers idle-spin on
    // futex/sched_yield for the REST of the link (profiling a 22-object Rust
    // link showed 85% of syscall time in futex+sched_yield from the pool, for
    // work that runs faster serially). Only fan out for links big enough that
    // the parse time dominates the thread-management cost.
    // Zero-copy parallel parse, in two stages so the shared arena is never
    // written from a worker thread (which would race / be nondeterministic):
    //   1. SERIAL pre-map — move every bare object's mmap into the arena and
    //      record its file_id. mmap is a cheap syscall; page faults happen
    //      lazily inside parse, so the real work still parallelizes.
    //   2. PARALLEL parse — each worker reads its object's bytes straight from
    //      the (now stable, immutable) arena mmap and produces an InputObject
    //      plus any transformed-bytes buffers (compressed .debug_*), with
    //      object-local Owned indices.
    //   3. SERIAL merge — append each object's owned buffers in object order and
    //      rebase its Owned handles, so the final layout is deterministic
    //      regardless of which thread finished first.
    const PARALLEL_PARSE_THRESHOLD: usize = 64;
    // Stage 1: pre-map. `mapped[i]` = (file_id, label) for bare[i]; bytes are
    // fetched from the arena by file_id inside the parse closure.
    let mut mapped: Vec<(u32, String)> = Vec::with_capacity(bare.len());
    let mut mapped_bytes = 0u64;
    {
        let _t = peony_prof::trace("parse-bare:map-inputs");
        for p in &bare {
            let label = p.display().to_string();
            // An empty/unmappable file can't be mmap'd; it is not a valid object and
            // `parse_bare_parallel` would fail it like the old serial path did. Map
            // it (reusing the classification map if present) or surface the error.
            let mmap = match mmap_cache.remove(p.as_path()).flatten() {
                Some(m) => m.into_mmap(),
                None => peony_object::MappedInput::open(p)
                    .ok_or_else(|| anyhow::anyhow!("failed to open/map `{}`", p.display()))?
                    .into_mmap(),
            };
            mapped_bytes = mapped_bytes.saturating_add(usize_to_trace_count(mmap.len()));
            let file_id = r.arena.push_mmap(mmap);
            mapped.push((file_id, label));
        }
    }
    peony_prof::event_fields(
        "parse-bare:mapped",
        [
            peony_prof::TraceField::count("objects", usize_to_trace_count(mapped.len())),
            peony_prof::TraceField::bytes("bytes", mapped_bytes),
        ],
    );
    // Stage 2: parallel parse from the stable arena mmaps.
    let parsed: Vec<InputObject> = {
        let _t = peony_prof::trace("parse-bare");
        peony_prof::event(
            "parse",
            format!(
                "{} bare objects, {}",
                mapped.len(),
                if mapped.len() >= PARALLEL_PARSE_THRESHOLD {
                    "parallel zero-copy"
                } else {
                    "serial zero-copy"
                }
            ),
        );
        // SAFETY: `arena` is only read here (mmap bytes); no thread mutates it.
        // The borrow is shared + the mmaps are stable for the arena's lifetime.
        let arena_ref = &r.arena;
        let collect_parse_stats = peony_prof::is_enabled();
        let parse_one =
            |&(file_id, ref label): &(u32, String)| -> Result<(InputObject, Vec<Vec<u8>>, Option<ParseStats>)> {
                let bytes = arena_ref.mmap_bytes(file_id);
                // owned_base = 0: handles are object-local, rebased at merge.
                let (obj, owned) =
                    peony_object::parse_bare_parallel(file_id, 0, label.clone(), bytes)
                        .with_context(|| format!("failed to parse `{label}`"))?;
                let stats = collect_parse_stats.then(|| ParseStats::from_parsed(&obj, bytes.len(), &owned));
                if peony_prof::trace_detail_enabled() {
                    let stats = stats.unwrap_or_default();
                    peony_prof::detail_event_fields(
                        "parse:object",
                        [
                            peony_prof::TraceField::text("object", label.as_str()),
                            peony_prof::TraceField::bytes("mapped", stats.input_bytes),
                            peony_prof::TraceField::count("sections", stats.sections),
                            peony_prof::TraceField::count("alloc_sections", stats.alloc_sections),
                            peony_prof::TraceField::count("debug_sections", stats.debug_sections),
                            peony_prof::TraceField::count("symbols", stats.symbols),
                            peony_prof::TraceField::count("relocs", stats.relocs),
                            peony_prof::TraceField::count("owned_buffers", stats.owned_buffers),
                            peony_prof::TraceField::bytes("owned", stats.owned_bytes),
                        ],
                    );
                }
                Ok((obj, owned, stats))
            };
        let results: Vec<(InputObject, Vec<Vec<u8>>, Option<ParseStats>)> = {
            let _t = peony_prof::trace("parse-bare:workers");
            if mapped.len() >= PARALLEL_PARSE_THRESHOLD {
                mapped.par_iter().map(parse_one).collect::<Result<_>>()?
            } else {
                mapped.iter().map(parse_one).collect::<Result<_>>()?
            }
        };
        if collect_parse_stats {
            let mut parse_stats = ParseStats::default();
            for (_, _, stats) in &results {
                if let Some(stats) = stats {
                    parse_stats.add(*stats);
                }
            }
            peony_prof::event_fields("parse-bare:result", parse_stats.trace_fields(results.len()));
        }
        // Stage 3: serial merge in object order (deterministic owned rebase).
        let mut parsed = Vec::with_capacity(results.len());
        {
            let _t = peony_prof::trace("parse-bare:merge-owned");
            for (mut obj, local_owned, _) in results {
                r.arena.merge_parsed_owned(&mut obj, local_owned);
                parsed.push(obj);
            }
        }
        parsed
    };
    // Now that the objects are parsed we know roughly how many distinct symbols
    // the link will define; pre-size the resolution map to avoid repeated
    // grow+rehash as ~10k+ symbols are inserted on a large link.
    {
        let est_symbols: usize = parsed.iter().map(|o| o.symbols.len()).sum();
        let _t = peony_prof::trace_fields(
            "resolve-bare:reserve-symbols",
            [
                peony_prof::TraceField::count("symbols", usize_to_trace_count(est_symbols)),
                peony_prof::TraceField::count("existing", r.symbols.len() as u64),
            ],
        );
        if est_symbols > r.symbols.len() {
            r.symbols.reserve(est_symbols);
        }
    }
    {
        let _t = peony_prof::trace("resolve-bare");
        for obj in parsed {
            r.resolve(obj)?;
        }
    }

    for name in forced_undefined {
        r.symbols.force_undefined(name.as_bytes());
    }

    // ── Whole archives: include every object member unconditionally ──────────
    let whole_archives: Vec<&PathBuf> = inputs
        .iter()
        .zip(&kinds)
        .filter(|(input, k)| **k == Kind::Archive && input.whole_archive)
        .map(|(input, _)| &input.path)
        .collect();
    if !whole_archives.is_empty() {
        include_whole_archive_members(&whole_archives, &mut r)?;
    }

    // ── --start-lib: object files with archive-like lazy semantics ───────────
    let start_lib_objects: Vec<&PathBuf> = inputs
        .iter()
        .zip(&kinds)
        .filter(|(input, k)| **k == Kind::Bare && input.start_lib_member)
        .map(|(input, _)| &input.path)
        .collect();
    if !start_lib_objects.is_empty() {
        include_start_lib_members(&start_lib_objects, &mut r)?;
    }

    // ── Archives: lazily include members that satisfy undefined references ────
    let archives: Vec<&PathBuf> = inputs
        .iter()
        .zip(&kinds)
        .filter(|(input, k)| **k == Kind::Archive && !input.whole_archive)
        .map(|(input, _)| &input.path)
        .collect();
    if !archives.is_empty() {
        include_archive_members(&archives, &mut r)?;
    }

    // ── Shared objects: their exports satisfy remaining undefined refs ────────
    // Parse the DSOs in parallel (each is an independent mmap + dynsym scan),
    // then register them SERIALLY in input order so `--as-needed` gating observes
    // a consistent symbol-table state (whether a DSO satisfied a still-undefined
    // ref depends on the registers that ran before it).
    let shared_inputs: Vec<&ResolvedInput> = inputs
        .iter()
        .zip(&kinds)
        .filter(|(_, k)| **k == Kind::Shared)
        .map(|(input, _)| input)
        .collect();
    let parse_shared = |input: &&ResolvedInput| -> Result<peony_object::SharedObject> {
        peony_object::parse_shared_object(&input.path)
            .with_context(|| format!("reading shared object `{}`", input.path.display()))
    };
    let parsed_shared: Vec<Result<peony_object::SharedObject>> = {
        let _t = peony_prof::trace("parse-shared");
        const PARALLEL_SHARED_THRESHOLD: usize = 4;
        if shared_inputs.len() >= PARALLEL_SHARED_THRESHOLD {
            shared_inputs.par_iter().map(parse_shared).collect()
        } else {
            shared_inputs.iter().map(parse_shared).collect()
        }
    };
    let mut needed = Vec::new();
    for (input, lib) in shared_inputs.iter().zip(parsed_shared) {
        let lib = lib?;
        let satisfied = r
            .symbols
            .register_shared_export_symbols(&lib.export_symbols, &lib.soname);
        if !input.as_needed || satisfied > 0 {
            needed.push(lib.soname);
        }
    }

    tracing::info!(
        objects = r.objects.len(),
        symbols = r.symbols.len(),
        needed = needed.len(),
        "symbol table built"
    );
    Ok(Resolved {
        arena: r.arena,
        objects: r.objects,
        symbols: r.symbols,
        comdat_excluded: r.excluded,
        needed,
    })
}

/// Result of loading + resolving the inputs.
struct Resolved {
    /// Backing store for all section bytes (input mmaps). Sections borrow into
    /// this; it must outlive layout + emit, so it travels with the objects.
    arena: peony_object::InputArena,
    objects: Vec<InputObject>,
    symbols: SymbolTable,
    /// `(object_id, section_index)` of sections discarded by COMDAT dedup.
    comdat_excluded: FxHashSet<(usize, usize)>,
    /// `DT_NEEDED` shared-library names.
    needed: Vec<String>,
}

/// Threads symbol resolution and COMDAT-group deduplication across objects, in
/// the order they are added (object index == `ObjectId`).
#[derive(Default)]
struct Resolver {
    /// All input mmaps; section bytes are borrowed from here (zero-copy).
    arena: peony_object::InputArena,
    objects: Vec<InputObject>,
    symbols: SymbolTable,
    seen_comdat: FxHashSet<Name>,
    excluded: FxHashSet<(usize, usize)>,
}

impl Resolver {
    fn resolve(&mut self, obj: InputObject) -> Result<()> {
        let oid = self.symbols.add_object(obj.path.clone());
        let obj_id = oid.0 as usize;
        // Discard COMDAT members whose signature was already seen.
        let mut obj_excluded: FxHashSet<usize> = FxHashSet::default();
        for g in &obj.comdat_groups {
            if g.signature.is_empty() {
                continue;
            }
            if !self.seen_comdat.insert(g.signature.clone()) {
                for &m in &g.members {
                    obj_excluded.insert(m);
                    self.excluded.insert((obj_id, m));
                }
            }
        }
        self.symbols
            .process_object_excluding(oid, &obj, &obj_excluded)
            .with_context(|| format!("symbol resolution failed for `{}`", obj.path))?;
        self.objects.push(obj);
        Ok(())
    }
}

struct Member {
    obj: Option<InputObject>,
    /// Global symbol names this member *defines*.
    defines: HashSet<Name>,
}

fn current_global_undefined(symbols: &SymbolTable) -> HashSet<Name> {
    symbols
        .iter()
        .filter(|(_, res)| !res.is_defined() && res.binding == Binding::Global)
        .map(|(n, _)| Name::from_slice(n))
        .collect()
}

fn member_defines(obj: &InputObject) -> HashSet<Name> {
    obj.symbols
        .iter()
        .filter(|s| !s.is_undefined && s.binding != Binding::Local && !s.name.is_empty())
        .map(|s| s.name.clone())
        .collect()
}

fn pull_lazy_members(
    members: &mut [Member],
    r: &mut Resolver,
    undefined: &mut HashSet<Name>,
) -> Result<u32> {
    let mut pulled = 0u32;
    for m in members.iter_mut() {
        if m.obj.is_none() {
            continue;
        }
        // Only pull this member for symbols STILL undefined right now.
        if m.defines.is_disjoint(undefined) {
            continue;
        }
        let obj = m.obj.take().unwrap();
        // Snapshot the names this member touches (its claimed defines plus
        // its own undefined refs) BEFORE resolving, then reconcile each
        // against the ACTUAL table state AFTER resolve. Reconciling against
        // the table — rather than trusting `m.defines` — is what keeps the
        // incremental set correct under COMDAT exclusion: a member can be
        // pulled yet have a claimed define dropped (its COMDAT group already
        // seen), in which case that name stays undefined and must remain in
        // the set. Bounded by the member's own symbol count.
        let touched: Vec<Name> = m
            .defines
            .iter()
            .cloned()
            .chain(
                obj.symbols
                    .iter()
                    .filter(|s| s.is_undefined && !s.name.is_empty())
                    .map(|s| s.name.clone()),
            )
            .collect();
        r.resolve(obj)?;
        for nm in touched {
            match r.symbols.lookup(nm.as_bytes()) {
                Some(res) if !res.is_defined() && res.binding == Binding::Global => {
                    undefined.insert(nm);
                }
                _ => {
                    undefined.remove(nm.as_bytes());
                }
            }
        }
        pulled += 1;
    }
    Ok(pulled)
}

fn include_whole_archive_members(archives: &[&PathBuf], r: &mut Resolver) -> Result<()> {
    let _t = peony_prof::trace("include_whole_archive_members");
    for ar in archives {
        let archive_members = iter_archive_members(ar)
            .with_context(|| format!("reading archive `{}`", ar.display()))?;
        peony_prof::count("whole_archive_members_seen", archive_members.len() as u64);
        for m in archive_members {
            let label = format!("{}({})", ar.display(), m.name);
            let owned_id = r.arena.push_owned(m.data);
            let data: &[u8] = {
                let b = r.arena.owned_bytes(owned_id);
                unsafe { std::slice::from_raw_parts(b.as_ptr(), b.len()) }
            };
            let Ok(obj) = parse_owned_member(&mut r.arena, owned_id, label, data) else {
                continue;
            };
            peony_prof::count("whole_archive_members_parsed", 1);
            r.resolve(obj)?;
        }
    }
    Ok(())
}

fn include_start_lib_members(paths: &[&PathBuf], r: &mut Resolver) -> Result<()> {
    let _t = peony_prof::trace("include_start_lib_members");
    let mut members = Vec::with_capacity(paths.len());
    for path in paths {
        let obj = parse_object(&mut r.arena, path)
            .with_context(|| format!("failed to parse start-lib member `{}`", path.display()))?;
        members.push(Member {
            defines: member_defines(&obj),
            obj: Some(obj),
        });
    }
    let mut undefined = current_global_undefined(&r.symbols);
    peony_prof::count("start_lib_strong_undefs", undefined.len() as u64);
    while !undefined.is_empty() {
        let pulled = pull_lazy_members(&mut members, r, &mut undefined)?;
        peony_prof::count("start_lib_members_pulled", pulled as u64);
        if pulled == 0 {
            break;
        }
    }
    Ok(())
}

fn include_archive_members(archives: &[&PathBuf], r: &mut Resolver) -> Result<()> {
    let _t = peony_prof::trace("include_archive_members");
    let mut members: Vec<Member> = Vec::new();
    let mut undefined: HashSet<Name> = current_global_undefined(&r.symbols);
    peony_prof::count("archive_strong_undefs", undefined.len() as u64);
    // Deduplicate archive paths, keeping first-seen order. `cc`/`rustc` pass the
    // same archive repeatedly (e.g. `-lgcc … -lgcc_s … -lgcc`, four `-lgcc`s) to
    // paper over link-order cycles between libgcc and libc. Reading each archive
    // 4× cost ~13MB of redundant I/O on a hello-world link. This is sound because
    // the member-inclusion loop below is a GLOBAL fixpoint over ALL collected
    // members — it re-checks the still-undefined set each round, so a member is
    // pulled exactly when it first resolves a live undef regardless of how many
    // times its archive was named. Collecting each archive's members once is thus
    // equivalent to collecting them N times (duplicates would only be dropped by
    // the fixpoint anyway).
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let unique: Vec<&PathBuf> = archives
        .iter()
        .filter(|ar| seen.insert((**ar).clone()))
        .copied()
        .collect();

    // Fixpoint: include any member that satisfies a currently-undefined symbol.
    // Archive semantics: a member is pulled only to resolve a symbol STILL
    // undefined at that moment. Two archives may define the same symbol (e.g.
    // `__mulsc3` in compiler_builtins.rlib and libgcc.a); only the first is
    // pulled, else a spurious duplicate.
    //
    // Pending archives are member-parsed lazily when their symbol index matches
    // the current undefined set. A no-hit link pays only symbol-index reads, but
    // cross-archive cycles still work because skipped archives are rechecked after
    // each pulled member adds new undefined refs.
    let mut pending_archives = unique;
    let mut parsed_index_offsets: FxHashMap<PathBuf, FxHashSet<u64>> = FxHashMap::default();
    let mut round = 0u32;
    loop {
        if undefined.is_empty() {
            break;
        }
        round += 1;
        let mut checked_archives = 0u32;
        let mut skipped_archives = 0u32;
        let mut parsed_archives = 0u32;
        let mut next_pending = Vec::new();
        let mut parsed_indexed = Vec::new();
        for ar in pending_archives {
            checked_archives += 1;
            let seen_offsets = parsed_index_offsets.entry((*ar).clone()).or_default();
            let (archive_members, used_index) = match iter_archive_members_matching(
                ar,
                |name| undefined.contains(name),
                |offset| seen_offsets.insert(offset),
            )
            .with_context(|| format!("reading archive index `{}`", ar.display()))?
            {
                Some(members) if members.is_empty() => {
                    skipped_archives += 1;
                    next_pending.push(ar);
                    continue;
                }
                Some(members) => (members, true),
                None => (
                    iter_archive_members(ar)
                        .with_context(|| format!("reading archive `{}`", ar.display()))?,
                    false,
                ),
            };
            parsed_archives += 1;
            if used_index {
                parsed_indexed.push(ar);
            }
            peony_prof::count("archive_members_seen", archive_members.len() as u64);
            for m in archive_members {
                let label = format!("{}({})", ar.display(), m.name);
                // Archive members are parsed from a buffer copied into the arena's
                // owned store (archives are not zero-copy from the `.a`, a small
                // non-hot cost; bare objects — the bulk — are zero-copy from mmap).
                let owned_id = r.arena.push_owned(m.data);
                // SAFETY: the buffer borrows arena.owned[owned_id], stable for the
                // arena's life; parse copies nothing out of it.
                let data: &[u8] = {
                    let b = r.arena.owned_bytes(owned_id);
                    unsafe { std::slice::from_raw_parts(b.as_ptr(), b.len()) }
                };
                let Ok(obj) = parse_owned_member(&mut r.arena, owned_id, label, data) else {
                    continue;
                };
                peony_prof::count("archive_members_parsed", 1);
                let defines = member_defines(&obj);
                members.push(Member {
                    obj: Some(obj),
                    defines,
                });
            }
        }
        let pulled = pull_lazy_members(&mut members, r, &mut undefined)?;
        let included_any = pulled > 0;
        peony_prof::event(
            "archive-round",
            format!(
                "round {round}: checked {checked_archives}, skipped {skipped_archives}, parsed {parsed_archives}, pulled {pulled}, {} undef left",
                undefined.len()
            ),
        );
        peony_prof::count("archive_index_checks", checked_archives as u64);
        peony_prof::count("archive_index_skips", skipped_archives as u64);
        peony_prof::count("archive_members_pulled", pulled as u64);
        if included_any {
            next_pending.extend(parsed_indexed);
        }
        pending_archives = next_pending;
        if !included_any && parsed_archives == 0 {
            break;
        }
    }
    Ok(())
}

/// Apply `--defsym SYM=VALUE` definitions as absolute symbols.
fn apply_defsym(symbols: &mut SymbolTable, defs: &[String]) -> Result<()> {
    for d in defs {
        let (name, val) = d
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid --defsym `{d}` (expected SYM=VALUE)"))?;
        let value = parse_hex_or_dec(val.trim())
            .with_context(|| format!("invalid --defsym value in `{d}`"))?;
        symbols.define_absolute(name.trim().as_bytes(), value);
    }
    Ok(())
}

/// Determine which input sections the layout should emit, combining
/// `--gc-sections` reachability with COMDAT deduplication. Returns `None` (emit
/// everything) only when neither applies.
fn compute_live(
    objects: &[InputObject],
    gc: bool,
    symbols: &SymbolTable,
    entry: &str,
    comdat_excluded: &FxHashSet<(usize, usize)>,
    export_roots: bool,
    script: Option<&ScriptLayout>,
) -> LiveFilter {
    if !gc {
        // No GC: emit everything except the COMDAT-discarded dups. This is a
        // small DENY-list — avoids materialising a set of all ~37k (obj,sec)
        // pairs just to remove a handful (measured ~5ms of pure hashset build).
        if comdat_excluded.is_empty() {
            return LiveFilter::All;
        }
        return LiveFilter::Except(comdat_excluded.clone());
    }
    // --gc-sections: an allow-list of reachable sections, minus COMDAT dups.
    let gc_out = peony_layout::gc_sections_rooted_with_stats(objects, symbols, entry, export_roots);
    let stats = gc_out.stats;
    let mut live = gc_out.live;
    if let Some(script) = script {
        add_script_kept_sections(&mut live, objects, script);
    }
    for key in comdat_excluded {
        live.remove(*key);
    }
    peony_prof::event_fields(
        "reloc-postproc:live-sections-result",
        [
            peony_prof::TraceField::count("roots", stats.roots),
            peony_prof::TraceField::count("visited", stats.traversed_sections),
            peony_prof::TraceField::count("relocs", stats.scanned_relocs),
            peony_prof::TraceField::count("raw_live", stats.live_sections),
            peony_prof::TraceField::count("final_live", live.len() as u64),
            peony_prof::TraceField::count("target_symbols", stats.target_symbols),
            peony_prof::TraceField::count("dense_maps", stats.dense_target_objects),
            peony_prof::TraceField::count("sparse_maps", stats.sparse_target_objects),
        ],
    );
    LiveFilter::Only(live)
}

fn add_script_kept_sections(
    live: &mut peony_layout::LiveSections,
    objects: &[InputObject],
    script: &ScriptLayout,
) {
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            if script.keeps_input(&sec.name) {
                live.insert((obj_id, sec.index.0));
            }
        }
    }
}

/// Owned section-emission filter produced by [`compute_live`]; borrowed as a
/// [`peony_layout::SectionFilter`] when passed to the layout.
enum LiveFilter {
    All,
    Only(peony_layout::LiveSections),
    Except(FxHashSet<(usize, usize)>),
}

impl LiveFilter {
    fn as_filter(&self) -> peony_layout::SectionFilter<'_> {
        match self {
            LiveFilter::All => peony_layout::SectionFilter::All,
            LiveFilter::Only(s) => peony_layout::SectionFilter::OnlyLive(s),
            LiveFilter::Except(s) => peony_layout::SectionFilter::Except(s),
        }
    }
}

/// The export allowlist parsed from a `--version-script`.
///
/// peony supports the subset rustc emits for a cdylib: a single anonymous
/// version node with a `global:` list of exact symbol names and a `local: *;`
/// catch-all. `exact` holds the named globals; `global_wildcard` is true if
/// `global: *` appears (export everything not explicitly localized).
#[derive(Debug, Default)]
struct VersionScript {
    exact: std::collections::HashSet<Vec<u8>>,
    global_wildcard: bool,
}

impl VersionScript {
    /// Whether a symbol named `name` should be exported under this script.
    fn exports(&self, name: &[u8]) -> bool {
        self.global_wildcard || self.exact.contains(name)
    }
}

/// Parse the subset of GNU version-script syntax rustc emits: `global:`/`local:`
/// sections with `;`-terminated bare symbol names and `*` wildcards. Comments
/// (`#`/`//`) and version-node names are ignored; we treat the file as one flat
/// global/local partition (sufficient for cdylib export control).
fn parse_version_script(path: &Path) -> Result<VersionScript> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading version script `{}`", path.display()))?;
    let mut vs = VersionScript::default();
    let mut in_global = false;
    for raw in text.lines() {
        // Strip comments and braces/version-node tokens.
        let line = raw.split('#').next().unwrap_or("");
        let line = line.split("//").next().unwrap_or("");
        for tok in line.split([' ', '\t', '{', '}', '(', ')']) {
            let tok = tok.trim();
            if tok.is_empty() {
                continue;
            }
            match tok {
                "global:" => in_global = true,
                "local:" => in_global = false,
                _ => {
                    // A name list entry, possibly several `name;` on one line.
                    for name in tok.split(';') {
                        let name = name.trim();
                        if name.is_empty() || name == ";" {
                            continue;
                        }
                        if in_global {
                            if name == "*" {
                                vs.global_wildcard = true;
                            } else {
                                vs.exact.insert(name.as_bytes().to_vec());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(vs)
}

fn read_dynamic_symbol_patterns(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading dynamic symbol list `{}`", path.display()))?;
    let text = strip_block_comments(&text);
    let mut patterns = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("");
        let line = line.split("//").next().unwrap_or("");
        let cleaned: String = line
            .chars()
            .map(|c| if "{}();,:\t\r\n".contains(c) { ' ' } else { c })
            .collect();
        for tok in cleaned.split_whitespace() {
            if matches!(
                tok,
                "global"
                    | "local"
                    | "extern"
                    | "C"
                    | "C++"
                    | "VERSION"
                    | "Base"
                    | "global:"
                    | "local:"
            ) {
                continue;
            }
            if tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && tok.ends_with("_VER") {
                continue;
            }
            if !patterns.iter().any(|p| p == tok) {
                patterns.push(tok.to_string());
            }
        }
    }
    Ok(patterns)
}

fn export_pattern_matches(patterns: &[String], name: &[u8]) -> bool {
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };
    patterns.iter().any(|pat| wildcard_matches(pat, name))
}

fn excluded_by_exclude_libs(
    exclude_libs: &[String],
    res: &peony_symbols::SymbolResolution,
    symbols: &SymbolTable,
) -> bool {
    if exclude_libs.is_empty() {
        return false;
    }
    let Some(obj_id) = res.defined_in else {
        return false;
    };
    let Some(path) = symbols.object_path(obj_id) else {
        return false;
    };
    let Some((archive, _member)) = path.split_once('(') else {
        return false;
    };
    let archive_name = Path::new(archive)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(archive);
    let stem = archive_name
        .strip_prefix("lib")
        .and_then(|s| s.strip_suffix(".a"))
        .unwrap_or(archive_name);
    exclude_libs.iter().any(|pat| {
        pat == "ALL"
            || pat == archive_name
            || pat == stem
            || wildcard_matches(pat, archive_name)
            || wildcard_matches(pat, stem)
    })
}

fn wildcard_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    if !pattern.starts_with('*') && !name.starts_with(parts[0]) {
        return false;
    }
    if !pattern.ends_with('*') && !name.ends_with(parts[parts.len() - 1]) {
        return false;
    }
    let mut rest = name;
    for part in parts.into_iter().filter(|p| !p.is_empty()) {
        let Some(pos) = rest.find(part) else {
            return false;
        };
        rest = &rest[pos + part.len()..];
    }
    true
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_thread_pool(threads: usize, n_inputs: usize) -> Result<()> {
    // `--threads 0` (the default) does NOT mean "all cores": profiling a
    // 423-object link showed peony's parallel phases stop scaling at ~4 workers
    // and get SLOWER beyond that (thread-management + allocator contention
    // outweigh the embarrassingly-parallel parse/scan once the serial layout and
    // per-reloc symbol lookups dominate — Amdahl). Letting rayon default to all
    // 24 cores cost ~25% over a 4-worker pool. Cap the default at a modest count;
    // an explicit `--threads N` still overrides.
    //
    // CRUCIAL for small links: rayon's global pool spawns N worker threads at
    // build time, and on a tiny link (a `cc hello.c` is ~10 inputs) those
    // workers spawn, find no work past the serial thresholds, and busy-spin on
    // sched_yield — measured at ~13ms of pure overhead and 264 sched_yield calls
    // on a 1-object link, making peony 3x slower than mold purely in thread
    // churn. Below the parallel thresholds there is nothing to parallelise, so
    // use a SINGLE thread: no worker pool, no spin. The PARALLEL_*_THRESHOLD
    // constants (256 objects) gate the actual fan-out, so a 1-thread pool here
    // loses nothing for small links and the big-link path is unaffected.
    const SMALL_LINK_INPUTS: usize = 64;
    let n = if threads > 0 {
        threads
    } else if n_inputs < SMALL_LINK_INPUTS {
        1
    } else {
        std::thread::available_parallelism()
            .map(|c| c.get())
            .unwrap_or(1)
            .min(8)
    };
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build_global()
        .context("failed to configure rayon thread pool")?;
    Ok(())
}

fn parse_hex_or_dec(s: &str) -> std::result::Result<u64, std::num::ParseIntError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse()
    }
}
