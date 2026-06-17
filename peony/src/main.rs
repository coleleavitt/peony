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
use peony_layout::{LayoutConfig, check_undefined, compute_layout, finalize_symbols};
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
    inputs: Vec<PathBuf>,
    library_paths: Vec<PathBuf>,
    libraries: Vec<String>,
    output: PathBuf,
    incremental: bool,
    threads: usize,
    base_address: String,
    entry: String,
    gc_sections: bool,
    defsym: Vec<String>,
    build_id: bool,
    strip: bool,
    pie: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            library_paths: Vec::new(),
            libraries: Vec::new(),
            output: PathBuf::from("a.out"),
            incremental: false,
            threads: 0,
            base_address: "0x400000".to_string(),
            entry: "_start".to_string(),
            gc_sections: false,
            defsym: Vec::new(),
            build_id: false,
            strip: false,
            pie: false,
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
        "-soname",
        "-T",
        "-y",
        "-Y",
        "-R",
        "-a",
        "-A",
        "--hash-style",
        "--version-script",
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
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].clone();
        let arg = arg.as_str();
        match arg {
            "-o" | "--output" => a.output = PathBuf::from(take(&argv, &mut i, arg)?),
            "-e" | "--entry" => a.entry = take(&argv, &mut i, arg)?,
            "-L" | "--library-path" => a
                .library_paths
                .push(PathBuf::from(take(&argv, &mut i, arg)?)),
            "-l" | "--library" => a.libraries.push(take(&argv, &mut i, arg)?),
            "--defsym" => a.defsym.push(take(&argv, &mut i, arg)?),
            "--threads" => a.threads = take(&argv, &mut i, arg)?.parse().unwrap_or(0),
            "--base-address" => a.base_address = take(&argv, &mut i, arg)?,
            "--gc-sections" => a.gc_sections = true,
            "--no-gc-sections" => a.gc_sections = false,
            "--build-id" => a.build_id = true,
            "--incremental" => a.incremental = true,
            "-s" | "-S" | "--strip-all" | "--strip-debug" => a.strip = true,
            "-pie" | "--pie" => a.pie = true,
            "-no-pie" | "--no-pie" => a.pie = false,
            _ if IGNORE_WITH_VALUE.contains(&arg) => {
                let _ = take(&argv, &mut i, arg); // consume and ignore the value
            }
            _ if arg.starts_with("--entry=") => a.entry = arg[8..].to_string(),
            _ if arg.starts_with("--defsym=") => a.defsym.push(arg[9..].to_string()),
            _ if arg.starts_with("--threads=") => a.threads = arg[10..].parse().unwrap_or(0),
            _ if arg.starts_with("--base-address=") => a.base_address = arg[15..].to_string(),
            _ if arg.starts_with("--build-id") => a.build_id = true, // --build-id=<style>
            _ if arg.starts_with("-o") => a.output = PathBuf::from(&arg[2..]),
            _ if arg.starts_with("-L") => a.library_paths.push(PathBuf::from(&arg[2..])),
            _ if arg.starts_with("-l") => a.libraries.push(arg[2..].to_string()),
            _ if arg.starts_with("-e") => a.entry = arg[2..].to_string(),
            _ if arg.starts_with('-') => {} // unknown ld flag → ignore
            _ => a.inputs.push(PathBuf::from(arg)), // positional input file
        }
        i += 1;
    }
    Ok(a)
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_writer(std::io::stderr) // diagnostics on stderr, like a real linker
        .init();

    let args = parse_args()?;
    init_thread_pool(args.threads)?;

    // A PIE is loaded at a kernel-chosen bias, so it is laid out from base 0.
    let base_address = if args.pie {
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
    // would for a dynamic/PIE executable.
    let inputs = inject_crt_objects(inputs, &args);

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
    let scan = scan_relocations(&objects, &symbols);
    let got_syms = scan.got_symbols();
    let plt_syms = scan.plt_symbols();
    tracing::info!(
        got_slots = got_syms.len(),
        plt_slots = plt_syms.len(),
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
    // A PIE needs R_X86_64_RELATIVE dynamic relocations for absolute pointers,
    // even with no imports. Emit dynamic sections whenever there are imports OR
    // the output is a PIE (rustc/cc default).
    let n_relative = if args.pie {
        peony_reloc::count_relative(&objects, &symbols)
            + peony_reloc::count_got_relative(&got_syms, &symbols)
    } else {
        0
    };
    let dynamic = (!imports.is_empty() || args.pie).then(|| peony_layout::DynamicInfo {
        imports,
        import_versions,
        import_sonames,
        needed: needed.clone(),
        pie: args.pie,
        n_relative,
    });
    if dynamic.is_some() {
        tracing::info!(needed = needed.len(), n_relative, "dynamic executable");
    }

    // Predefine linker-provided symbols and `--defsym`s BEFORE layout so they are
    // included in `.symtab`; the layout-dependent addresses are filled in after.
    let provided = predefine_linker_symbols(&mut symbols);
    apply_defsym(&mut symbols, &args.defsym)?;

    let config = LayoutConfig {
        base_address,
        entry_symbol: args.entry.clone(),
        build_id: args.build_id,
        strip: args.strip,
        pie: args.pie,
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
    check_undefined(&symbols).context("unresolved symbols")?;

    // Assemble `.rela.dyn` now that symbol VAs are final: the R_X86_64_RELATIVE
    // entries (PIE only) come first, then the GLOB_DATs. For a non-PIE dynamic
    // executable there are no relatives, so this just materialises the GLOB_DATs.
    if dynamic.is_some() {
        let relative = if args.pie {
            peony_reloc::collect_relative(&objects, &symbols, &layout)
        } else {
            Vec::new()
        };
        layout.append_relative_relocs(&relative);
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
    let parsed: Vec<InputObject> = bare
        .par_iter()
        .map(|p| parse_object(p).with_context(|| format!("failed to parse `{}`", p.display())))
        .collect::<Result<_>>()?;
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
        if r.symbols.register_shared_exports_versioned(
            &lib.exports,
            &lib.export_versions,
            &lib.soname,
        ) > 0
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

    // Fixpoint: include any member that defines a currently-undefined symbol.
    loop {
        let undefined: HashSet<Vec<u8>> = r
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
            if m.defines.is_disjoint(&undefined) {
                continue;
            }
            let obj = m.obj.take().unwrap();
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
];

fn linker_symbol_addr(name: &str, layout: &peony_layout::Layout) -> u64 {
    match name {
        "_GLOBAL_OFFSET_TABLE_" => layout.got_base,
        "__executable_start" | "__ehdr_start" => layout.image_base,
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
) -> Option<FxHashSet<(usize, usize)>> {
    if !gc && comdat_excluded.is_empty() {
        return None;
    }
    let mut live = if gc {
        peony_layout::gc_sections(objects, symbols, entry)
    } else {
        all_sections(objects)
    };
    for key in comdat_excluded {
        live.remove(key);
    }
    Some(live)
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
    text.contains("GROUP") || text.contains("INPUT")
}

/// Extract the file/`-l` references from a linker script (GROUP/INPUT/AS_NEEDED).
fn parse_linker_script(path: &Path) -> Result<Vec<String>> {
    let data = std::fs::read(path)
        .with_context(|| format!("reading linker script `{}`", path.display()))?;
    let text = strip_block_comments(&String::from_utf8_lossy(&data));
    let cleaned: String = text
        .chars()
        .map(|c| if "(),".contains(c) { ' ' } else { c })
        .collect();
    Ok(cleaned
        .split_whitespace()
        .filter(|t| {
            t.starts_with("-l") || t.contains('/') || t.ends_with(".a") || t.contains(".so")
        })
        .map(str::to_string)
        .collect())
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
    let mut push = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    for p in gcc_library_dirs() {
        push(p, &mut out);
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
fn gcc_library_dirs() -> Vec<PathBuf> {
    let output = match std::process::Command::new("gcc")
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
