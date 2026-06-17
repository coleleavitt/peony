//! `peony` — driver binary for the incremental parallel ELF linker.
//!
//! ## Usage (ld-compatible subset)
//!
//! ```text
//! peony [OPTIONS] <inputs>...
//!   -o <file>          Output file (default: a.out)
//!   --incremental      Enable the incremental cache
//!   --threads <N>      rayon worker threads (0 = all cores)
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

use anyhow::{Context, Result};
use peony_emit::{EmitConfig, emit_full};
use peony_layout::{
    LayoutConfig,
    ScriptLayout,
    ScriptOutputSection,
    check_undefined,
    compute_layout,
    finalize_symbols,
};
use peony_object::{Binding, InputObject, iter_archive_members, parse_bytes, parse_object};
use peony_reloc::scan_relocations;
use peony_symbols::SymbolTable;
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use tracing_subscriber::EnvFilter;

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Linker options. Parsed by a permissive, `ld`-compatible hand-rolled parser
/// (see [`parse_args`]) so peony can be invoked directly by `cc`/`gcc` as the
/// linker — it accepts the standard `ld` flags and ignores the ones it doesn't
/// act on, rather than erroring.
#[derive(Debug)]
struct Args {
    raw_args: Vec<String>,
    inputs: Vec<PathBuf>,
    library_paths: Vec<PathBuf>,
    libraries: Vec<String>,
    output: PathBuf,
    incremental: bool,
    threads: usize,
    base_address: String,
    base_address_explicit: bool,
    entry: String,
    entry_explicit: bool,
    gc_sections: bool,
    defsym: Vec<String>,
    linker_scripts: Vec<PathBuf>,
    plugin: Option<PathBuf>,
    build_id: bool,
    strip_all: bool,
    strip_debug: bool,
    emit_relocs: bool,
    pie: bool,
    /// Produce a shared object (`-shared`): ET_DYN with exported `.dynsym`, no
    /// crt startup objects, no `PT_INTERP`, no mandatory entry point.
    shared: bool,
    /// `DT_SONAME` value (`-soname`/`-h`). Defaults to the output file name when
    /// producing a shared object.
    soname: Option<String>,
    /// `--version-script` path. rustc emits one for a cdylib listing exactly the
    /// symbols to export (`global:`) and hiding the rest (`local: *`).
    version_script: Option<PathBuf>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            raw_args: Vec::new(),
            inputs: Vec::new(),
            library_paths: Vec::new(),
            libraries: Vec::new(),
            output: PathBuf::from("a.out"),
            incremental: false,
            threads: 0,
            base_address: "0x400000".to_string(),
            base_address_explicit: false,
            entry: "_start".to_string(),
            entry_explicit: false,
            gc_sections: false,
            defsym: Vec::new(),
            linker_scripts: Vec::new(),
            plugin: None,
            build_id: false,
            strip_all: false,
            strip_debug: false,
            emit_relocs: false,
            pie: false,
            shared: false,
            soname: None,
            version_script: None,
        }
    }
}

/// Parse a (permissive, `ld`-compatible) command line. Unknown flags are ignored;
/// flags that take a separate value argument consume it so it isn't mistaken for
/// an input file.
fn parse_args() -> Result<Args> {
    // ld flags whose value is a *separate* following argument that we ignore.
    const IGNORE_WITH_VALUE: &[&str] = &[
        "-z",
        "-m",
        "-plugin",
        "-plugin-opt",
        "-rpath",
        "-rpath-link",
        "-y",
        "-Y",
        "-R",
        "-a",
        "-A",
        "--hash-style",
        "--sysroot",
        "-dynamic-linker",
        "--dynamic-linker",
        "--exclude-libs",
    ];
    fn take(argv: &[String], i: &mut usize, flag: &str) -> Result<String> {
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing value for `{flag}`"))
    }
    let mut a = Args::default();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    a.raw_args = argv.clone();
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].clone();
        let arg = arg.as_str();
        match arg {
            "-o" | "--output" => a.output = PathBuf::from(take(&argv, &mut i, arg)?),
            "-e" | "--entry" => {
                a.entry = take(&argv, &mut i, arg)?;
                a.entry_explicit = true;
            }
            "-L" | "--library-path" => a
                .library_paths
                .push(PathBuf::from(take(&argv, &mut i, arg)?)),
            "-T" | "--script" => a
                .linker_scripts
                .push(PathBuf::from(take(&argv, &mut i, arg)?)),
            "-l" | "--library" => a.libraries.push(take(&argv, &mut i, arg)?),
            "--defsym" => a.defsym.push(take(&argv, &mut i, arg)?),
            "-plugin" | "--plugin" => a.plugin = Some(PathBuf::from(take(&argv, &mut i, arg)?)),
            "--threads" => a.threads = take(&argv, &mut i, arg)?.parse().unwrap_or(0),
            "--base-address" => {
                a.base_address = take(&argv, &mut i, arg)?;
                a.base_address_explicit = true;
            }
            "--gc-sections" => a.gc_sections = true,
            "--no-gc-sections" => a.gc_sections = false,
            "--build-id" => a.build_id = true,
            "--incremental" => a.incremental = true,
            "-s" | "--strip-all" => {
                a.strip_all = true;
                a.strip_debug = true;
            }
            "-S" | "--strip-debug" => a.strip_debug = true,
            "--emit-relocs" | "-q" => a.emit_relocs = true,
            "-pie" | "--pie" => a.pie = true,
            "-no-pie" | "--no-pie" => a.pie = false,
            "-shared" | "--shared" | "-Bshareable" => a.shared = true,
            "-soname" | "-h" | "--soname" => a.soname = Some(take(&argv, &mut i, arg)?),
            "--version-script" => a.version_script = Some(PathBuf::from(take(&argv, &mut i, arg)?)),
            _ if IGNORE_WITH_VALUE.contains(&arg) => {
                let _ = take(&argv, &mut i, arg); // consume and ignore the value
            }
            _ if arg.starts_with("--entry=") => {
                a.entry = arg[8..].to_string();
                a.entry_explicit = true;
            }
            _ if arg.starts_with("--defsym=") => a.defsym.push(arg[9..].to_string()),
            _ if arg.starts_with("--plugin=") => {
                a.plugin = Some(PathBuf::from(&arg["--plugin=".len()..]))
            }
            _ if arg.starts_with("--threads=") => a.threads = arg[10..].parse().unwrap_or(0),
            _ if arg.starts_with("--base-address=") => {
                a.base_address = arg[15..].to_string();
                a.base_address_explicit = true;
            }
            _ if arg.starts_with("--script=") => a
                .linker_scripts
                .push(PathBuf::from(&arg["--script=".len()..])),
            _ if arg.starts_with("--build-id") => a.build_id = true, // --build-id=<style>
            _ if arg.starts_with("--version-script=") => {
                a.version_script = Some(PathBuf::from(&arg["--version-script=".len()..]))
            }
            _ if arg.starts_with("-soname=") => a.soname = Some(arg[8..].to_string()),
            _ if arg.starts_with("-h") && arg.len() > 2 => a.soname = Some(arg[2..].to_string()),
            _ if arg.starts_with("-o") => a.output = PathBuf::from(&arg[2..]),
            _ if arg.starts_with("-L") => a.library_paths.push(PathBuf::from(&arg[2..])),
            _ if arg.starts_with("-T") && arg.len() > 2 => {
                a.linker_scripts.push(PathBuf::from(&arg[2..]))
            }
            _ if arg.starts_with("-plugin=") => a.plugin = Some(PathBuf::from(&arg[8..])),
            _ if arg.starts_with("-l") => a.libraries.push(arg[2..].to_string()),
            _ if arg.starts_with("-e") => {
                a.entry = arg[2..].to_string();
                a.entry_explicit = true;
            }
            _ if arg.starts_with('-') => {} // unknown ld flag → ignore
            _ => a.inputs.push(PathBuf::from(arg)), // positional input file
        }
        i += 1;
    }
    Ok(a)
}

/// `cc`/`g++`/`rustc` always pass `-plugin .../liblto_plugin.so` even for a
/// plain (non-LTO) link, so handing the whole link off to `ld.bfd` whenever a
/// plugin is present meant peony was silently never doing the link. Instead we
/// strip the plugin args (already kept out of `inputs` by the parser) and link
/// the objects ourselves — they are real ELF `.o` files, not LTO IR. Any actual
/// LTO IR object simply fails `parse_object` and is skipped as non-ELF.
///
/// Kept as a no-op returning `Ok(false)` so the call site and `args.plugin`
/// (used for logging/LTO detection) are preserved.
fn maybe_handoff_lto_plugin(args: &Args) -> Result<bool> {
    if let Some(plugin) = &args.plugin {
        tracing::info!(
            plugin = %plugin.display(),
            "ignoring LTO plugin; linking objects directly (no bfd handoff)"
        );
    }
    Ok(false)
}

#[allow(dead_code)] // retained for diagnostics; no longer used for LTO handoff
fn system_gnu_linker() -> Option<PathBuf> {
    ["/usr/bin/ld.bfd", "/usr/bin/ld"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
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
    init_thread_pool(args.threads)?;

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
    let inputs = resolve_inputs(&args)?;
    let inputs = expand_inputs(inputs, &library_search_paths(&args.library_paths))?;
    // When invoked directly as the linker (e.g. by rustc), the C-runtime startup
    // objects that provide `_start`/`_init` are not passed; inject them as `cc`
    // would for a dynamic/PIE executable. A shared object has no `_start`/crt1.
    let inputs = if args.shared {
        inputs
    } else {
        inject_crt_objects(inputs, &args)
    };

    // Incremental fast-path: if every input and the previous output are
    // byte-identical to the last link, the existing output is already correct.
    if args.incremental
        && peony_cache::try_reuse(&args.output, &inputs).context("incremental cache")?
    {
        tracing::info!(output = %args.output.display(), "incremental: inputs unchanged, reused cached output");
        return Ok(());
    }

    let Resolved {
        objects,
        mut symbols,
        comdat_excluded,
        needed,
    } = load_and_resolve(&inputs)?;

    // Weak-undefined symbols referenced through the GOT (e.g. `__gmon_start__`)
    // need a real SymbolId so their GOT slot gets a recorded address (holding 0).
    // Assign ids before the scan so the slots are tracked.
    peony_reloc::assign_weak_got_ids(&objects, &mut symbols);

    tracing::info!("scanning relocations");
    let scan = scan_relocations(&objects, &symbols, args.shared);
    let got_syms = scan.got_symbols();
    let plt_syms = scan.plt_symbols();
    let tls_got = peony_layout::TlsGotInfo {
        gd: scan.tls_gd_refs(),
        ie: scan.tls_ie_refs(),
        desc: scan.tls_desc_refs(),
        ldm: scan.needs_tls_ldm(),
    };
    let copy_relocs = if args.shared {
        Vec::new()
    } else {
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

    // Combine the GC live-set with COMDAT deduplication into the set of sections
    // the layout will actually emit.
    let live = compute_live(
        &objects,
        args.gc_sections,
        &symbols,
        &args.entry,
        &comdat_excluded,
        args.shared, // shared-object exports are GC roots
    );
    if let Some(l) = &live {
        tracing::info!(live_sections = l.len(), "section selection complete");
    }

    // Dynamic mode: any shared-library import → emit a dynamic executable.
    let mut imports: Vec<Vec<u8>> = symbols
        .iter()
        .filter(|(_, r)| r.import)
        .map(|(n, _)| n.to_vec())
        .collect();
    imports.sort();
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
    let (n_relative, n_irelative) = if et_dyn {
        let total = peony_reloc::count_relative(&objects, &symbols)
            + peony_reloc::count_got_relative(&got_syms, &symbols);
        let irel = peony_reloc::count_irelative(&objects, &symbols, &got_syms);
        (total, irel)
    } else {
        (0, 0)
    };
    // A shared object exports its defined, non-hidden global/weak symbols so
    // `dlsym` can find them. When rustc supplies a `--version-script`, it lists
    // exactly the symbols to export (`global:`) and localizes the rest
    // (`local: *`), so we honour it as an allowlist — otherwise std's thousands
    // of internal globals would all leak into `.dynsym`.
    let version_script = match &args.version_script {
        Some(p) => Some(parse_version_script(p)?),
        None => None,
    };
    let exports: Vec<peony_layout::ExportSym> = if args.shared {
        let mut e: Vec<peony_layout::ExportSym> = symbols
            .iter()
            .filter(|(_, r)| r.is_export())
            .filter(|(name, _)| version_script.as_ref().is_none_or(|vs| vs.exports(name)))
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
    // object needs GD/LD/IE relocs; an executable relaxes GD/LD to Local-Exec and
    // fills IE slots statically, so it needs none.
    let n_tls_reloc = if args.shared {
        peony_reloc::count_tls_relocs(&objects, &symbols, &tls_got)
    } else {
        0
    };
    // Symbolic R_X86_64_64 dynamic relocs for data sites referencing an imported
    // symbol (gcc's `.data.rel.local.DW.ref.*` EH slots). Sized here, collected
    // post-layout. Meaningful for any ET_DYN (PIE or shared).
    let n_symbolic_data = if et_dyn {
        peony_reloc::count_symbolic_data_relocs(&objects, &symbols)
    } else {
        0
    };
    // Dynamic sections are needed for any import, any PIE, or a shared object.
    let dynamic = (!imports.is_empty() || et_dyn).then(|| peony_layout::DynamicInfo {
        imports,
        import_versions,
        import_sonames,
        needed: needed.clone(),
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
    let mut layout = compute_layout(
        &objects,
        &symbols,
        &got_syms,
        &plt_syms,
        live.as_ref(),
        dynamic.as_ref(),
        &config,
        &tls_got,
    )
    .context("layout computation failed")?;
    tracing::info!(
        sections = layout.output_sections.len(),
        segments = layout.segments.len(),
        file_size = layout.file_size,
        entry = format_args!("{:#x}", layout.entry),
        "layout complete"
    );

    set_linker_addresses(&mut symbols, &layout, &provided);
    finalize_symbols(&mut symbols, &layout);
    // A shared object may legitimately reference symbols it does not define;
    // they are resolved at load time against the process image. Only enforce
    // full resolution for executables.
    if !args.shared {
        check_undefined(&symbols).context("unresolved symbols")?;
    }

    // Assemble `.rela.dyn` now that symbol VAs are final: the R_X86_64_RELATIVE
    // entries (ET_DYN only) come first, then the GLOB_DATs. For a non-PIE dynamic
    // executable there are no relatives, so this just materialises the GLOB_DATs.
    if dynamic.is_some() {
        // Partition data relocations into RELATIVE (normal) and IRELATIVE (IFUNC,
        // resolver run at startup). Meaningful for any ET_DYN (PIE or shared); a
        // non-PIE dynamic exe has no base-relative data relocs.
        let (relative, irelative) = if et_dyn {
            peony_reloc::collect_dynamic_data_relocs(&objects, &symbols, &layout)
        } else {
            (Vec::new(), Vec::new())
        };
        if !irelative.is_empty() {
            tracing::info!(
                ifuncs = irelative.len(),
                "emitting R_X86_64_IRELATIVE relocations"
            );
        }
        // TLS GOT contents: a shared object emits DTPMOD64/DTPOFF64/TPOFF64
        // dynamic relocs (+ static DTPOFF in GD/LDM slot1); an executable writes
        // its Initial-Exec slots statically (no relocs). Always run when there
        // are TLS GOT slots so exe IE slots get filled.
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
        // Symbolic R_X86_64_64 dynamic relocs for imported-symbol data sites.
        let symbolic = if et_dyn {
            peony_reloc::collect_symbolic_data_relocs(&objects, &symbols, &layout)
        } else {
            Vec::new()
        };
        if !symbolic.is_empty() {
            tracing::info!(
                symbolic = symbolic.len(),
                "emitting symbolic R_X86_64_64 dynamic relocations"
            );
        }
        layout.append_all_dynamic_relocs(&relative, &irelative, &tls_dyn, &symbolic);
    }

    emit_full(
        &args.output,
        &objects,
        &symbols,
        &layout,
        &EmitConfig::default(),
    )
    .context("binary emission failed")?;

    if args.incremental {
        peony_cache::record_link(&args.output, &inputs).context("incremental cache record")?;
    }

    tracing::info!(output = %args.output.display(), "link complete");
    Ok(())
}

// ── Loading + resolution ───────────────────────────────────────────────────────

/// Parse all bare objects (in parallel) and pull in archive members lazily,
/// returning the object list and the resolved global symbol table.
///
/// Object indices in the returned `Vec` match the [`peony_symbols::ObjectId`]s
/// assigned during resolution (lock-step `add_object` + `push`).
fn load_and_resolve(inputs: &[PathBuf]) -> Result<Resolved> {
    let mut r = Resolver::default();

    // ── Bare objects: parallel parse, then serial resolve in input order ─────
    let bare: Vec<&PathBuf> = inputs
        .iter()
        .filter(|p| !is_archive(p) && !peony_object::is_shared_object(p))
        .collect();
    tracing::info!(objects = bare.len(), "parsing input objects");
    // Small links (the common `cc`/incremental case) parse faster serially:
    // touching rayon's global pool spins up a worker per core that then idles
    // on `sched_yield`/`futex` for longer than the handful of parses take. Only
    // fan out once there are enough objects to amortize the thread management.
    const PARALLEL_PARSE_THRESHOLD: usize = 16;
    let parse_one = |p: &&PathBuf| {
        parse_object(p).with_context(|| format!("failed to parse `{}`", p.display()))
    };
    let parsed: Vec<InputObject> = if bare.len() >= PARALLEL_PARSE_THRESHOLD {
        bare.par_iter().map(parse_one).collect::<Result<_>>()?
    } else {
        bare.iter().map(parse_one).collect::<Result<_>>()?
    };
    for obj in parsed {
        r.resolve(obj)?;
    }

    // ── Archives: lazily include members that satisfy undefined references ────
    let archives: Vec<&PathBuf> = inputs.iter().filter(|p| is_archive(p)).collect();
    if !archives.is_empty() {
        include_archive_members(&archives, &mut r)?;
    }

    // ── Shared objects: their exports satisfy remaining undefined refs ────────
    let mut needed = Vec::new();
    for so in inputs.iter().filter(|p| peony_object::is_shared_object(p)) {
        let lib = peony_object::parse_shared_object(so)
            .with_context(|| format!("reading shared object `{}`", so.display()))?;
        if r.symbols
            .register_shared_export_symbols(&lib.export_symbols, &lib.soname)
            > 0
        {
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
        objects: r.objects,
        symbols: r.symbols,
        comdat_excluded: r.excluded,
        needed,
    })
}

/// Result of loading + resolving the inputs.
struct Resolved {
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
    objects: Vec<InputObject>,
    symbols: SymbolTable,
    seen_comdat: FxHashSet<Vec<u8>>,
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
    defines: HashSet<Vec<u8>>,
}

fn include_archive_members(archives: &[&PathBuf], r: &mut Resolver) -> Result<()> {
    let mut members: Vec<Member> = Vec::new();
    for ar in archives {
        for m in iter_archive_members(ar)
            .with_context(|| format!("reading archive `{}`", ar.display()))?
        {
            let label = format!("{}({})", ar.display(), m.name);
            // Skip members that are not ELF objects (e.g. rustc metadata).
            let Ok(obj) = parse_bytes(label, &m.data) else {
                continue;
            };
            let defines = obj
                .symbols
                .iter()
                .filter(|s| !s.is_undefined && s.binding != Binding::Local && !s.name.is_empty())
                .map(|s| s.name.clone())
                .collect();
            members.push(Member {
                obj: Some(obj),
                defines,
            });
        }
    }

    // Fixpoint: include any member that satisfies a currently-undefined symbol.
    // Crucially, re-check undefinedness *per member* and update it as members are
    // pulled in: archive semantics say a member is included only to resolve a
    // symbol still undefined at that moment. Two archives may both define the
    // same symbol (e.g. `__mulsc3` in compiler_builtins.rlib and libgcc.a); only
    // the first should be pulled, or peony would report a spurious duplicate.
    loop {
        let mut undefined: HashSet<Vec<u8>> = r
            .symbols
            .iter()
            .filter(|(_, res)| !res.is_defined())
            .map(|(n, _)| n.to_vec())
            .collect();
        if undefined.is_empty() {
            break;
        }
        let mut included_any = false;
        for m in members.iter_mut() {
            if m.obj.is_none() {
                continue;
            }
            // Only pull this member for symbols STILL undefined right now.
            if m.defines.is_disjoint(&undefined) {
                continue;
            }
            let obj = m.obj.take().unwrap();
            // The symbols this member newly provides are no longer undefined, so
            // a later member defining the same name is not pulled for them.
            for name in &m.defines {
                undefined.remove(name);
            }
            r.resolve(obj)?;
            included_any = true;
        }
        if !included_any {
            break;
        }
    }
    Ok(())
}

/// Standard linker-provided symbols (PROVIDE semantics: only define a name an
/// input *referenced but left undefined*; never override a real definition).
const LINKER_SYMS: &[&str] = &[
    "_GLOBAL_OFFSET_TABLE_",
    "__executable_start",
    "__ehdr_start",
    "__bss_start",
    "_edata",
    "edata",
    "_end",
    "end",
    // `__dso_handle` identifies this DSO for `__cxa_atexit`/`__cxa_finalize`.
    // The runtime only uses its ADDRESS as an opaque handle, so the image base
    // is a valid, stable definition. crtbegin normally provides it, but when
    // linking without crt (e.g. a cdylib) `libc_nonshared.a` references it
    // undefined, so the linker must synthesise it.
    "__dso_handle",
];

fn linker_symbol_addr(name: &str, layout: &peony_layout::Layout) -> u64 {
    match name {
        "_GLOBAL_OFFSET_TABLE_" => layout.got_base,
        "__executable_start" | "__ehdr_start" | "__dso_handle" => layout.image_base,
        "__bss_start" => layout.bss_start,
        "_edata" | "edata" => layout.edata,
        "_end" | "end" => layout.end,
        _ => 0,
    }
}

/// Pre-define well-known linker-synthesized symbols (e.g. `_end`, `_start`)
/// as absolute symbols at address 0; their real addresses are patched by the
/// layout pass. Returns the list of names registered.
fn predefine_linker_symbols(symbols: &mut SymbolTable) -> Vec<&'static str> {
    let mut provided = Vec::new();
    for &name in LINKER_SYMS {
        if let Some(r) = symbols.lookup(name.as_bytes()) {
            if r.defined_in.is_none() {
                symbols.define_absolute(name.as_bytes(), 0);
                provided.push(name);
            }
        }
    }
    provided
}

/// Fill in the real addresses of the provided linker symbols after layout.
fn set_linker_addresses(
    symbols: &mut SymbolTable,
    layout: &peony_layout::Layout,
    provided: &[&'static str],
) {
    for &name in provided {
        let addr = linker_symbol_addr(name, layout);
        if let Some(r) = symbols.lookup_mut(name.as_bytes()) {
            r.value = addr;
            r.virtual_address = addr;
        }
    }
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
) -> Option<FxHashSet<(usize, usize)>> {
    if !gc && comdat_excluded.is_empty() {
        return None;
    }
    let mut live = if gc {
        peony_layout::gc_sections_rooted(objects, symbols, entry, export_roots)
    } else {
        all_sections(objects)
    };
    for key in comdat_excluded {
        live.remove(key);
    }
    Some(live)
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

fn all_sections(objects: &[InputObject]) -> FxHashSet<(usize, usize)> {
    objects
        .iter()
        .enumerate()
        .flat_map(|(i, o)| o.sections.iter().map(move |s| (i, s.index.0)))
        .collect()
}

/// Expand any GNU linker-script inputs (e.g. the system `libc.so`, which is a
/// `GROUP(...)` script) into the real object/library files they reference,
/// recursively.
fn expand_inputs(inputs: Vec<PathBuf>, search: &[PathBuf]) -> Result<Vec<PathBuf>> {
    use std::collections::VecDeque;
    let mut out = Vec::new();
    let mut work: VecDeque<PathBuf> = inputs.into();
    while let Some(p) = work.pop_front() {
        if !is_linker_script(&p) {
            out.push(p);
            continue;
        }
        let dir = p.parent().map(Path::to_path_buf);
        for r in parse_linker_script(&p)? {
            match resolve_script_ref(&r, dir.as_deref(), search) {
                Some(rp) => work.push_back(rp),
                None => tracing::warn!("linker script `{}`: cannot resolve `{r}`", p.display()),
            }
        }
    }
    Ok(out)
}

/// A text file referencing GROUP/INPUT (and not an ELF/archive) is a linker script.
fn is_linker_script(path: &Path) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    if data.starts_with(&peony_object::elf::ELFMAG) || data.starts_with(b"!<arch>\n") {
        return false;
    }
    let text = String::from_utf8_lossy(&data);
    text.contains("GROUP")
        || text.contains("INPUT")
        || text.contains("AS_NEEDED")
        || text.contains("SECTIONS")
        || text.contains("ENTRY")
}

/// Extract the file/`-l` references from a linker script (GROUP/INPUT/AS_NEEDED).
fn parse_linker_script(path: &Path) -> Result<Vec<String>> {
    let data = std::fs::read(path)
        .with_context(|| format!("reading linker script `{}`", path.display()))?;
    let text = strip_block_comments(&String::from_utf8_lossy(&data));
    Ok(extract_script_refs(&text))
}

#[derive(Default)]
struct ScriptControls {
    entry: Option<String>,
    base_address: Option<String>,
    layout: ScriptLayout,
}

fn parse_linker_script_controls(paths: &[PathBuf]) -> Result<ScriptControls> {
    let mut merged = ScriptControls::default();
    for path in paths {
        let data = std::fs::read(path)
            .with_context(|| format!("reading linker script `{}`", path.display()))?;
        let text = strip_block_comments(&String::from_utf8_lossy(&data));
        if let Some(entry) = directive_arg(&text, "ENTRY") {
            merged.entry = Some(entry);
        }
        if let Some(base) = script_base_address(&text) {
            merged.base_address = Some(base);
        }
        let layout = parse_sections_layout(&text);
        merged.layout.output_sections.extend(layout.output_sections);
    }
    Ok(merged)
}

fn extract_script_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for keyword in ["GROUP", "INPUT", "AS_NEEDED"] {
        let mut pos = 0;
        while let Some((body, end)) = directive_body(text, keyword, pos) {
            for r in parse_ref_tokens(body) {
                if !refs.contains(&r) {
                    refs.push(r);
                }
            }
            pos = end;
        }
    }
    refs
}

fn parse_ref_tokens(text: &str) -> Vec<String> {
    let cleaned: String = text
        .chars()
        .map(|c| if "(),".contains(c) { ' ' } else { c })
        .collect();
    cleaned
        .split_whitespace()
        .map(|t| t.trim_matches('"').trim_matches('\''))
        .filter(|t| {
            *t != "GROUP"
                && *t != "INPUT"
                && *t != "AS_NEEDED"
                && *t != "/DISCARD/"
                && (t.starts_with("-l")
                    || t.contains('/')
                    || t.ends_with(".a")
                    || t.contains(".so"))
        })
        .map(str::to_string)
        .collect()
}

fn parse_sections_layout(text: &str) -> ScriptLayout {
    let Some((body, _)) = directive_body(text, "SECTIONS", 0) else {
        return ScriptLayout::default();
    };
    let mut layout = ScriptLayout::default();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b';') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'.' && bytes[i] != b'/' {
            i += 1;
            continue;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b':' | b'{' | b';')
        {
            i += 1;
        }
        let name = body[name_start..i].trim();
        let j = skip_ws(body, i);
        if name == "." && j < bytes.len() && bytes[j] == b'=' {
            i = body[j..].find(';').map_or(bytes.len(), |off| j + off + 1);
            continue;
        }
        let Some(colon) = find_byte_before(body, j, b':', b';') else {
            i += 1;
            continue;
        };
        let Some(open_rel) = body[colon + 1..].find('{') else {
            i = colon + 1;
            continue;
        };
        let open = colon + 1 + open_rel;
        let Some(close) = matching_brace(body, open) else {
            break;
        };
        if !name.is_empty() && name != "/DISCARD/" {
            let mut patterns = section_patterns(&body[open + 1..close]);
            if patterns.is_empty() {
                patterns.push(name.to_string());
            }
            layout.output_sections.push(ScriptOutputSection {
                name: name.to_string(),
                patterns,
            });
        }
        i = close + 1;
    }
    layout
}

fn section_patterns(body: &str) -> Vec<String> {
    let cleaned: String = body
        .chars()
        .map(|c| if "(){};,\":".contains(c) { ' ' } else { c })
        .collect();
    let mut out = Vec::new();
    for tok in cleaned.split_whitespace() {
        if tok.starts_with('.') && !out.iter().any(|p| p == tok) {
            out.push(tok.to_string());
        }
    }
    out
}

fn script_base_address(text: &str) -> Option<String> {
    let (body, _) = directive_body(text, "SECTIONS", 0)?;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.' {
            let j = skip_ws(body, i + 1);
            if j < bytes.len() && bytes[j] == b'=' {
                let expr_start = skip_ws(body, j + 1);
                let expr_end = body[expr_start..]
                    .find(';')
                    .map_or(bytes.len(), |off| expr_start + off);
                let expr = body[expr_start..expr_end].trim();
                if let Some(num) = first_number_token(expr) {
                    return Some(num.to_string());
                }
            }
        }
        i += 1;
    }
    None
}

fn first_number_token(expr: &str) -> Option<&str> {
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let is_start = bytes[i].is_ascii_digit();
        if is_start {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_hexdigit() || matches!(bytes[i], b'x' | b'X'))
            {
                i += 1;
            }
            return Some(&expr[start..i]);
        }
        i += 1;
    }
    None
}

fn directive_arg(text: &str, keyword: &str) -> Option<String> {
    directive_body(text, keyword, 0).map(|(body, _)| body.trim().to_string())
}

fn directive_body<'a>(text: &'a str, keyword: &str, start: usize) -> Option<(&'a str, usize)> {
    let mut search = start;
    while let Some(pos) = text[search..].find(keyword) {
        let kw = search + pos;
        let before_ok = kw == 0
            || !text.as_bytes()[kw - 1].is_ascii_alphanumeric() && text.as_bytes()[kw - 1] != b'_';
        let after = kw + keyword.len();
        let after_ok = after >= text.len()
            || !text.as_bytes()[after].is_ascii_alphanumeric() && text.as_bytes()[after] != b'_';
        if before_ok && after_ok {
            let open = skip_ws(text, after);
            if open < text.len() && text.as_bytes()[open] == b'(' {
                if let Some(close) = matching_paren(text, open) {
                    return Some((&text[open + 1..close], close + 1));
                }
            } else if open < text.len() && text.as_bytes()[open] == b'{' {
                if let Some(close) = matching_brace(text, open) {
                    return Some((&text[open + 1..close], close + 1));
                }
            }
        }
        search = after;
    }
    None
}

fn find_byte_before(text: &str, start: usize, needle: u8, stop: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == needle {
            return Some(i);
        }
        if bytes[i] == stop {
            return None;
        }
        i += 1;
    }
    None
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    matching_delim(text, open, b'(', b')')
}

fn matching_brace(text: &str, open: usize) -> Option<usize> {
    matching_delim(text, open, b'{', b'}')
}

fn matching_delim(text: &str, open: usize, left: u8, right: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        if bytes[i] == left {
            depth += 1;
        } else if bytes[i] == right {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn skip_ws(text: &str, mut i: usize) -> usize {
    let bytes = text.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn strip_block_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn resolve_script_ref(r: &str, script_dir: Option<&Path>, search: &[PathBuf]) -> Option<PathBuf> {
    if let Some(name) = r.strip_prefix("-l") {
        for d in search {
            for ext in ["so", "a"] {
                let p = d.join(format!("lib{name}.{ext}"));
                if p.exists() {
                    return Some(p);
                }
            }
        }
        return None;
    }
    let p = Path::new(r);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    if let Some(dir) = script_dir {
        let q = dir.join(r);
        if q.exists() {
            return Some(q);
        }
    }
    search.iter().map(|d| d.join(r)).find(|p| p.exists())
}

/// Resolve positional inputs plus `-l<name>` libraries (searched as `lib<name>.a`
/// on the `-L` paths) into the final ordered input list.
///
/// The search list is the explicit `-L` directories followed by the system
/// library directories discovered from the host C toolchain (`gcc
/// -print-search-dirs`) plus the standard multiarch locations. This lets
/// `-lgcc_s`, `-lc`, etc. resolve without the caller passing every `-L`, exactly
/// as GNU ld / lld do when driven by `cc`.
fn resolve_inputs(args: &Args) -> Result<Vec<PathBuf>> {
    let mut inputs = args.inputs.clone();
    inputs.extend(args.linker_scripts.iter().cloned());
    let search = library_search_paths(&args.library_paths);
    for name in &args.libraries {
        // Search each dir for the shared library first, then the archive.
        let found = search
            .iter()
            .flat_map(|d| {
                [
                    d.join(format!("lib{name}.so")),
                    d.join(format!("lib{name}.a")),
                ]
            })
            .find(|p| p.exists())
            .ok_or_else(|| {
                anyhow::anyhow!("cannot find -l{name} (lib{name}.so/.a) on the library path")
            })?;
        inputs.push(found);
    }
    if inputs.is_empty() {
        anyhow::bail!("no input files");
    }
    Ok(inputs)
}

/// Inject C-runtime startup objects (`Scrt1.o crti.o crtbeginS.o … crtendS.o
/// crtn.o`) around the user inputs, as the `cc` driver does, when none is already
/// present. Only applied for executables (not `-shared`/`-r`). If the toolchain
/// objects can't be located, the inputs are returned unchanged (a static link
/// with its own `_start`, e.g. the test-suite's hand-written objects, still works).
fn inject_crt_objects(inputs: Vec<PathBuf>, args: &Args) -> Vec<PathBuf> {
    // Heuristic: skip if a Scrt1/crt1 object is already on the command line.
    let already = inputs.iter().any(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == "Scrt1.o" || n == "crt1.o" || n == "rcrt1.o")
    });
    // Only auto-inject for a PIE (the case rustc/cc drive without crt objects).
    if already || !args.pie {
        return inputs;
    }
    // Skip if the inputs already provide `_start` (a freestanding PIE that does
    // not need the C runtime). crt's Scrt1.o also defines `_start`, so injecting
    // would cause a duplicate-symbol error.
    if inputs.iter().any(|p| object_defines_start(p)) {
        return inputs;
    }

    // Locate the crt objects via the C toolchain. `crtbeginS.o`/`crtendS.o` are
    // the PIC variants used for PIE and dynamic executables.
    let find = |name: &str| -> Option<PathBuf> {
        let out = std::process::Command::new("gcc")
            .arg(format!("-print-file-name={name}"))
            .output()
            .ok()?;
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        // gcc returns the bare name unchanged when it can't find the file.
        (p.is_absolute() && p.exists()).then_some(p)
    };

    let scrt1 = if args.pie { "Scrt1.o" } else { "crt1.o" };
    let (begin, end) = if args.pie {
        ("crtbeginS.o", "crtendS.o")
    } else {
        ("crtbegin.o", "crtend.o")
    };
    let (Some(c1), Some(ci), Some(cb), Some(ce), Some(cn)) = (
        find(scrt1),
        find("crti.o"),
        find(begin),
        find(end),
        find("crtn.o"),
    ) else {
        return inputs; // toolchain crt unavailable — leave inputs as-is
    };

    let mut out = Vec::with_capacity(inputs.len() + 5);
    out.push(c1);
    out.push(ci);
    out.push(cb);
    out.extend(inputs);
    out.push(ce);
    out.push(cn);
    out
}

/// True if `path` is a relocatable object that defines a global `_start`.
/// Used to decide whether the C-runtime startup objects are needed.
fn object_defines_start(path: &Path) -> bool {
    if is_archive(path) || peony_object::is_shared_object(path) {
        return false;
    }
    match peony_object::parse_object(path) {
        Ok(obj) => obj.symbols.iter().any(|s| {
            s.name == b"_start" && !s.is_undefined && s.binding != peony_object::Binding::Local
        }),
        Err(_) => false,
    }
}

/// The full library search path: explicit `-L` dirs first (highest priority),
/// then GCC's own library directories, then standard system locations.
fn library_search_paths(explicit: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = explicit.to_vec();
    let push = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    for p in gcc_library_dirs() {
        push(p.clone(), &mut out);
    }
    for p in [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/lib/x86_64-linux-gnu",
        "/lib64",
        "/usr/lib",
        "/lib",
    ] {
        push(PathBuf::from(p), &mut out);
    }
    out
}

/// Parse `gcc -print-search-dirs` and return its `libraries:` entries.
/// Returns an empty vector if `gcc` is unavailable, so peony still works with
/// explicit `-L` paths on systems without a C compiler.
fn gcc_library_dirs() -> &'static [PathBuf] {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE.get_or_init(|| {
        // Try known locations before a PATH search to avoid the 40+ failed
        // `execve()`s the OS performs probing every PATH entry for `gcc`.
        let gcc = ["/usr/bin/gcc", "/usr/local/bin/gcc", "/usr/bin/cc"]
            .iter()
            .find(|p| std::fs::metadata(p).is_ok())
            .copied()
            .unwrap_or("gcc");
        let output = match std::process::Command::new(gcc)
            .arg("-print-search-dirs")
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("libraries:") {
                // Format: "libraries: =/path/a:/path/b:..."; entries are ':'-joined.
                let rest = rest.trim_start().trim_start_matches('=');
                return rest
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    // Canonicalize to collapse the ../.. segments gcc emits.
                    .map(|p| p.canonicalize().unwrap_or(p))
                    .collect();
            }
        }
        Vec::new()
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_thread_pool(threads: usize) -> Result<()> {
    if threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .context("failed to configure rayon thread pool")?;
    }
    Ok(())
}

/// An archive iff the file begins with the `ar` magic.
fn is_archive(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map(|_| &magic == b"!<arch>\n")
        .unwrap_or(false)
}

fn parse_hex_or_dec(s: &str) -> std::result::Result<u64, std::num::ParseIntError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse()
    }
}
