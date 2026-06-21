use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use peony_layout::HashStyle;

use crate::read_dynamic_symbol_patterns;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryMode {
    Any,
    Static,
    Dynamic,
}

#[derive(Debug, Clone)]
pub(crate) enum LinkSpecKind {
    Path(PathBuf),
    Library(String),
}

#[derive(Debug, Clone)]
pub(crate) struct LinkSpec {
    pub(crate) kind: LinkSpecKind,
    pub(crate) whole_archive: bool,
    pub(crate) as_needed: bool,
    pub(crate) library_mode: LibraryMode,
    pub(crate) start_lib_member: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedInput {
    pub(crate) path: PathBuf,
    pub(crate) whole_archive: bool,
    pub(crate) as_needed: bool,
    pub(crate) start_lib_member: bool,
}

/// Linker options. Parsed by a permissive, `ld`-compatible hand-rolled parser
/// (see [`parse_args`]) so peony can be invoked directly by `cc`/`gcc` as the
/// linker — it accepts the standard `ld` flags and ignores the ones it doesn't
/// act on, rather than erroring.
#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) raw_args: Vec<String>,
    pub(crate) inputs: Vec<PathBuf>,
    pub(crate) library_paths: Vec<PathBuf>,
    pub(crate) libraries: Vec<String>,
    pub(crate) link_specs: Vec<LinkSpec>,
    pub(crate) output: PathBuf,
    pub(crate) incremental: bool,
    /// Run as a resident daemon: load the incremental cache into RAM and serve
    /// relinks over a Unix socket (requires a prior `--incremental` link).
    pub(crate) daemon: bool,
    pub(crate) cache_report: Option<PathBuf>,
    pub(crate) threads: usize,
    pub(crate) base_address: String,
    pub(crate) base_address_explicit: bool,
    pub(crate) entry: String,
    pub(crate) entry_explicit: bool,
    pub(crate) gc_sections: bool,
    pub(crate) icf: bool,
    pub(crate) stats: bool,
    pub(crate) trace: bool,
    pub(crate) trace_stack: bool,
    pub(crate) trace_detail: bool,
    pub(crate) help: bool,
    pub(crate) defsym: Vec<String>,
    pub(crate) linker_scripts: Vec<PathBuf>,
    pub(crate) plugin: Option<PathBuf>,
    pub(crate) build_id: bool,
    pub(crate) strip_all: bool,
    pub(crate) strip_debug: bool,
    pub(crate) emit_relocs: bool,
    pub(crate) pie: bool,
    pub(crate) no_crt: bool,
    pub(crate) dynamic_linker: Option<String>,
    pub(crate) rpaths: Vec<String>,
    pub(crate) enable_new_dtags: bool,
    pub(crate) hash_style: HashStyle,
    pub(crate) export_dynamic: bool,
    pub(crate) export_dynamic_patterns: Vec<String>,
    pub(crate) exclude_libs: Vec<String>,
    pub(crate) no_undefined: bool,
    pub(crate) undefined: Vec<String>,
    pub(crate) require_defined: Vec<String>,
    pub(crate) relocatable: bool,
    /// Produce a shared object (`-shared`): ET_DYN with exported `.dynsym`, no
    /// crt startup objects, no `PT_INTERP`, no mandatory entry point.
    pub(crate) shared: bool,
    /// `DT_SONAME` value (`-soname`/`-h`). Defaults to the output file name when
    /// producing a shared object.
    pub(crate) soname: Option<String>,
    /// `--version-script` path. rustc emits one for a cdylib listing exactly the
    /// symbols to export (`global:`) and hiding the rest (`local: *`).
    pub(crate) version_script: Option<PathBuf>,
    /// Known output-affecting flags that Peony does not implement. Unknown
    /// driver-noise flags are still ignored, but these must not silently produce
    /// a binary with different semantics.
    pub(crate) unsupported_flags: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            raw_args: Vec::new(),
            inputs: Vec::new(),
            library_paths: Vec::new(),
            libraries: Vec::new(),
            link_specs: Vec::new(),
            output: PathBuf::from("a.out"),
            // Incremental linking is ON by default: a relink reuses the cache
            // for a fast (and, with a daemon, sub-5ms) byte-identical patch.
            // Opt out with `--no-incremental` or `PEONY_INCREMENTAL=0` (e.g. for
            // a clean/CI build that never relinks and does not want the cache).
            incremental: true,
            daemon: false,
            cache_report: None,
            threads: 0,
            base_address: "0x400000".to_string(),
            base_address_explicit: false,
            entry: "_start".to_string(),
            entry_explicit: false,
            gc_sections: false,
            icf: false,
            stats: false,
            trace: false,
            trace_stack: false,
            trace_detail: false,
            help: false,
            defsym: Vec::new(),
            linker_scripts: Vec::new(),
            plugin: None,
            build_id: false,
            strip_all: false,
            strip_debug: false,
            emit_relocs: false,
            pie: false,
            no_crt: false,
            dynamic_linker: None,
            rpaths: Vec::new(),
            enable_new_dtags: true,
            hash_style: HashStyle::Both,
            export_dynamic: false,
            export_dynamic_patterns: Vec::new(),
            exclude_libs: Vec::new(),
            no_undefined: false,
            undefined: Vec::new(),
            require_defined: Vec::new(),
            relocatable: false,
            shared: false,
            soname: None,
            version_script: None,
            unsupported_flags: Vec::new(),
        }
    }
}

/// Parse a (permissive, `ld`-compatible) command line. Unknown flags are ignored;
/// flags that take a separate value argument consume it so it isn't mistaken for
/// an input file.
pub(crate) fn parse_args() -> Result<Args> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let argv = expand_response_args(argv)?;
    parse_expanded_args(argv)
}

fn parse_expanded_args(argv: Vec<String>) -> Result<Args> {
    // ld flags whose value is a *separate* following argument that we ignore.
    const IGNORE_WITH_VALUE: &[&str] = &[
        "-m",
        "-plugin",
        "-plugin-opt",
        "-rpath-link",
        "-y",
        "-Y",
        "-R",
        "-a",
        "-A",
        "--sysroot",
    ];
    fn take(argv: &[String], i: &mut usize, flag: &str) -> Result<String> {
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing value for `{flag}`"))
    }
    fn push_path(
        a: &mut Args,
        path: PathBuf,
        whole: bool,
        as_needed: bool,
        mode: LibraryMode,
        start_lib: bool,
    ) {
        a.inputs.push(path.clone());
        a.link_specs.push(LinkSpec {
            kind: LinkSpecKind::Path(path),
            whole_archive: whole,
            as_needed,
            library_mode: mode,
            start_lib_member: start_lib,
        });
    }
    fn push_library(
        a: &mut Args,
        name: String,
        whole: bool,
        as_needed: bool,
        mode: LibraryMode,
        start_lib: bool,
    ) {
        a.libraries.push(name.clone());
        a.link_specs.push(LinkSpec {
            kind: LinkSpecKind::Library(name),
            whole_archive: whole,
            as_needed,
            library_mode: mode,
            start_lib_member: start_lib,
        });
    }
    fn parse_hash_style(value: &str) -> Result<HashStyle> {
        match value {
            "sysv" => Ok(HashStyle::Sysv),
            "gnu" => Ok(HashStyle::Gnu),
            "both" => Ok(HashStyle::Both),
            _ => anyhow::bail!("unsupported --hash-style `{value}`"),
        }
    }
    fn parse_exclude_libs(a: &mut Args, value: &str) {
        a.exclude_libs.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    fn parse_z_option(a: &mut Args, value: &str) {
        let value = value.trim_start_matches('=');
        match value {
            // Peony already emits RELRO for dynamic links, uses eager binding for
            // supported dynamic links, and emits a non-executable GNU_STACK.
            "relro" | "now" | "noexecstack" => {}
            "defs" => a.no_undefined = true,
            "undefs" => a.no_undefined = false,
            _ => a.unsupported_flags.push(format!("-z {value}")),
        }
    }

    let mut a = Args::default();
    a.raw_args = argv.clone();
    let mut whole_archive = false;
    let mut as_needed = false;
    let mut library_mode = LibraryMode::Any;
    let mut start_lib = false;
    let mut state_stack: Vec<(bool, bool, LibraryMode)> = Vec::new();
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
            "-l" | "--library" => push_library(
                &mut a,
                take(&argv, &mut i, arg)?,
                whole_archive,
                as_needed,
                library_mode,
                start_lib,
            ),
            "--defsym" => a.defsym.push(take(&argv, &mut i, arg)?),
            "-plugin" | "--plugin" => a.plugin = Some(PathBuf::from(take(&argv, &mut i, arg)?)),
            "--threads" => a.threads = take(&argv, &mut i, arg)?.parse().unwrap_or(0),
            "--base-address" => {
                a.base_address = take(&argv, &mut i, arg)?;
                a.base_address_explicit = true;
            }
            "--gc-sections" => a.gc_sections = true,
            "--no-gc-sections" => a.gc_sections = false,
            "--stats" => a.stats = true,
            "--trace" => a.trace = true,
            "--trace-stack" => {
                a.trace = true;
                a.trace_stack = true;
            }
            "--trace-detail" => {
                a.trace = true;
                a.trace_detail = true;
            }
            "--help" => a.help = true,
            // Identical Code Folding. `=all`/`=safe` accepted; `=none` disables.
            "--icf=all" | "--icf=safe" | "--icf" => a.icf = true,
            "--icf=none" | "--no-icf" => a.icf = false,
            "--build-id" => a.build_id = true,
            "--incremental" => a.incremental = true,
            "--no-incremental" => {
                a.incremental = false;
                a.daemon = false;
            }
            "--daemon" => {
                a.incremental = true;
                a.daemon = true;
            }
            "--cache-report" => a.cache_report = Some(PathBuf::from(take(&argv, &mut i, arg)?)),
            "-s" | "--strip-all" => {
                a.strip_all = true;
                a.strip_debug = true;
            }
            "-S" | "--strip-debug" => a.strip_debug = true,
            "--emit-relocs" | "-q" => a.emit_relocs = true,
            "-pie" | "--pie" => a.pie = true,
            "-no-pie" | "--no-pie" => a.pie = false,
            "-nostartfiles" | "--no-crt" => a.no_crt = true,
            "-static" | "--static" => {
                library_mode = LibraryMode::Static;
                a.pie = false;
            }
            "-static-pie" | "--static-pie" => a.unsupported_flags.push(arg.to_string()),
            "-Bstatic" | "-dn" | "-non_shared" => library_mode = LibraryMode::Static,
            "-Bdynamic" | "-dy" => library_mode = LibraryMode::Dynamic,
            "--as-needed" => as_needed = true,
            "--no-as-needed" => as_needed = false,
            "--push-state" => state_stack.push((whole_archive, as_needed, library_mode)),
            "--pop-state" => {
                let Some((whole, needed, mode)) = state_stack.pop() else {
                    anyhow::bail!("unbalanced --pop-state");
                };
                whole_archive = whole;
                as_needed = needed;
                library_mode = mode;
            }
            "--whole-archive" => whole_archive = true,
            "--no-whole-archive" => whole_archive = false,
            "--start-lib" => {
                if start_lib {
                    anyhow::bail!("nested --start-lib");
                }
                start_lib = true;
            }
            "--end-lib" => {
                if !start_lib {
                    anyhow::bail!("stray --end-lib");
                }
                start_lib = false;
            }
            "-rpath" | "--rpath" => a.rpaths.push(take(&argv, &mut i, arg)?),
            "-dynamic-linker" | "--dynamic-linker" => {
                a.dynamic_linker = Some(take(&argv, &mut i, arg)?);
            }
            "-z" => parse_z_option(&mut a, &take(&argv, &mut i, arg)?),
            "--hash-style" => a.hash_style = parse_hash_style(&take(&argv, &mut i, arg)?)?,
            "--enable-new-dtags" => a.enable_new_dtags = true,
            "--disable-new-dtags" => a.enable_new_dtags = false,
            "--exclude-libs" => parse_exclude_libs(&mut a, &take(&argv, &mut i, arg)?),
            "--no-undefined" | "-zdefs" => a.no_undefined = true,
            "--allow-shlib-undefined" | "--unresolved-symbols=ignore-all" => {
                a.no_undefined = false;
            }
            "--no-allow-shlib-undefined" | "--unresolved-symbols=report-all" => {
                a.no_undefined = true;
            }
            "-shared" | "--shared" | "-Bshareable" => a.shared = true,
            "-soname" | "-h" | "--soname" => a.soname = Some(take(&argv, &mut i, arg)?),
            "--version-script" => a.version_script = Some(PathBuf::from(take(&argv, &mut i, arg)?)),
            "-r" | "--relocatable" => a.relocatable = true,
            "--wrap" | "-wrap" => {
                a.unsupported_flags.push(arg.to_string());
                if matches!(arg, "--wrap" | "-wrap") && i + 1 < argv.len() {
                    i += 1;
                }
            }
            "-u" | "--undefined" => a.undefined.push(take(&argv, &mut i, arg)?),
            "--require-defined" => {
                let name = take(&argv, &mut i, arg)?;
                a.undefined.push(name.clone());
                a.require_defined.push(name);
            }
            "--export-dynamic" => a.export_dynamic = true,
            "--export-dynamic-symbol" => {
                a.export_dynamic_patterns.push(take(&argv, &mut i, arg)?);
            }
            "--export-dynamic-symbol-list" | "--dynamic-list" => {
                let path = take(&argv, &mut i, arg)?;
                a.export_dynamic_patterns
                    .extend(read_dynamic_symbol_patterns(Path::new(&path))?);
            }
            _ if IGNORE_WITH_VALUE.contains(&arg) => {
                // A recognised-but-ignored flag that carries a value (e.g.
                // `-m elf_x86_64`): skip its value argument. A missing value at
                // end-of-argv is harmless here — there is simply nothing to skip
                // — so we advance the index directly rather than erroring.
                i += 1;
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
            _ if arg.starts_with("-rpath=") => a.rpaths.push(arg["-rpath=".len()..].to_string()),
            _ if arg.starts_with("--rpath=") => a.rpaths.push(arg["--rpath=".len()..].to_string()),
            _ if arg.starts_with("-dynamic-linker=") => {
                a.dynamic_linker = Some(arg["-dynamic-linker=".len()..].to_string())
            }
            _ if arg.starts_with("--dynamic-linker=") => {
                a.dynamic_linker = Some(arg["--dynamic-linker=".len()..].to_string())
            }
            _ if arg.starts_with("--hash-style=") => {
                a.hash_style = parse_hash_style(&arg["--hash-style=".len()..])?;
            }
            _ if arg.starts_with("--cache-report=") => {
                a.cache_report = Some(PathBuf::from(&arg["--cache-report=".len()..]));
            }
            _ if arg.starts_with("--exclude-libs=") => {
                parse_exclude_libs(&mut a, &arg["--exclude-libs=".len()..])
            }
            _ if arg.starts_with("-z") && arg.len() > 2 => {
                parse_z_option(&mut a, &arg["-z".len()..]);
            }
            _ if arg == "--start-group" || arg == "--end-group" => {
                // Peony's archive resolver is already a global fixpoint, so
                // GNU group markers do not change behavior for supported input
                // kinds. Accept them as compatibility no-ops.
            }
            _ if arg.starts_with("--wrap=") || arg.starts_with("-wrap=") => {
                a.unsupported_flags.push(arg.to_string());
            }
            _ if arg.starts_with("--require-defined=") => {
                let name = arg["--require-defined=".len()..].to_string();
                a.undefined.push(name.clone());
                a.require_defined.push(name);
            }
            _ if arg.starts_with("--undefined=") => {
                a.undefined.push(arg["--undefined=".len()..].to_string())
            }
            _ if arg.starts_with("--export-dynamic-symbol=") => a
                .export_dynamic_patterns
                .push(arg["--export-dynamic-symbol=".len()..].to_string()),
            _ if arg.starts_with("--export-dynamic-symbol-list=") => {
                let path = &arg["--export-dynamic-symbol-list=".len()..];
                a.export_dynamic_patterns
                    .extend(read_dynamic_symbol_patterns(Path::new(path))?);
            }
            _ if arg.starts_with("--dynamic-list=") => {
                let path = &arg["--dynamic-list=".len()..];
                a.export_dynamic_patterns
                    .extend(read_dynamic_symbol_patterns(Path::new(path))?);
            }
            _ if arg.starts_with("-soname=") => a.soname = Some(arg[8..].to_string()),
            _ if arg.starts_with("-h") && arg.len() > 2 => a.soname = Some(arg[2..].to_string()),
            _ if arg.starts_with("-o") => a.output = PathBuf::from(&arg[2..]),
            _ if arg.starts_with("-L") => a.library_paths.push(PathBuf::from(&arg[2..])),
            _ if arg.starts_with("-T") && arg.len() > 2 => {
                a.linker_scripts.push(PathBuf::from(&arg[2..]))
            }
            _ if arg.starts_with("-plugin=") => a.plugin = Some(PathBuf::from(&arg[8..])),
            _ if arg.starts_with("-l") => push_library(
                &mut a,
                arg[2..].to_string(),
                whole_archive,
                as_needed,
                library_mode,
                start_lib,
            ),
            _ if arg.starts_with("-e") => {
                a.entry = arg[2..].to_string();
                a.entry_explicit = true;
            }
            _ if arg.starts_with('-') => {} // unknown ld flag → ignore
            _ => push_path(
                &mut a,
                PathBuf::from(arg),
                whole_archive,
                as_needed,
                library_mode,
                start_lib,
            ),
        }
        i += 1;
    }
    if start_lib {
        anyhow::bail!("missing --end-lib");
    }
    if !state_stack.is_empty() {
        anyhow::bail!("unbalanced --push-state/--pop-state");
    }
    Ok(a)
}

fn expand_response_args(argv: Vec<String>) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    expand_response_args_inner(argv, &mut seen, 0)
}

fn expand_response_args_inner(
    argv: Vec<String>,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<Vec<String>> {
    if depth > 16 {
        anyhow::bail!("response-file nesting is too deep");
    }
    let mut out = Vec::new();
    for arg in argv {
        let Some(path) = arg.strip_prefix('@') else {
            out.push(arg);
            continue;
        };
        if path.is_empty() {
            out.push(arg);
            continue;
        }
        let p = PathBuf::from(path);
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if !seen.insert(key.clone()) {
            anyhow::bail!("recursive response file `{}`", p.display());
        }
        let text = std::fs::read_to_string(&p)
            .with_context(|| format!("reading response file `{}`", p.display()))?;
        let nested = parse_response_file(&text)?;
        out.extend(expand_response_args_inner(nested, seen, depth + 1)?);
        seen.remove(&key);
    }
    Ok(out)
}

fn parse_response_file(text: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            (None, '"' | '\'') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (_, '\\') => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                } else {
                    cur.push('\\');
                }
            }
            _ => cur.push(ch),
        }
    }
    if let Some(q) = quote {
        anyhow::bail!("unterminated {q} quote in response file");
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    Ok(args)
}

/// Return only command-line arguments that can affect output bytes for the
/// incremental cache key. Diagnostics and scheduler knobs must not make an
/// otherwise reusable output look stale.
pub(crate) fn cache_key_args(raw_args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(raw_args.len());
    let mut skip_value = false;
    for arg in raw_args {
        if skip_value {
            skip_value = false;
            continue;
        }
        match arg.as_str() {
            "--incremental" | "--no-incremental" | "--daemon" | "--stats" | "--trace"
            | "--trace-stack" | "--trace-detail" => {
                continue;
            }
            "--cache-report" | "--threads" => {
                skip_value = true;
                continue;
            }
            _ if arg.starts_with("--cache-report=") || arg.starts_with("--threads=") => continue,
            _ => out.push(arg.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn cache_key_args_ignores_diagnostic_and_scheduler_flags() {
        let raw = strings(&[
            "--incremental",
            "--cache-report",
            "target/peony-cache.json",
            "--stats",
            "--threads",
            "8",
            "-o",
            "app",
            "main.o",
        ]);

        assert_eq!(cache_key_args(&raw), strings(&["-o", "app", "main.o"]));
    }

    #[test]
    fn cache_key_args_ignores_inline_cache_report_path() {
        let raw = strings(&[
            "--cache-report=target/peony-cache.json",
            "--trace",
            "--trace-detail",
            "--threads=4",
            "-shared",
            "lib.o",
        ]);

        assert_eq!(cache_key_args(&raw), strings(&["-shared", "lib.o"]));
    }

    #[test]
    fn parse_crt_suppression_flags() {
        let no_crt = parse_expanded_args(strings(&["--no-crt", "main.o"])).unwrap();
        assert!(no_crt.no_crt);

        let no_startfiles = parse_expanded_args(strings(&["-nostartfiles", "main.o"])).unwrap();
        assert!(no_startfiles.no_crt);
    }
}

pub(crate) fn print_help() {
    println!(
        "peony [OPTIONS] <inputs>...\n\n  -o FILE             Output file (default: a.out)\n  -L DIR              Add library search directory\n  -l NAME             Link libNAME.so or libNAME.a\n  -e, --entry SYM     Entry symbol (default: _start)\n  --threads N         Worker thread count (0 = auto)\n  --stats             Print phase timing table and cache diagnostics\n  --trace             Print phase timing and call-flow trace\n  --trace-detail      Include capped byte/address detail records\n  --trace-stack       Print trace frames with Rust backtraces\n  --incremental       Incremental cache (ON by default)\n  --no-incremental    Disable incremental (also PEONY_INCREMENTAL=0)\n  --daemon            Run a resident daemon serving sub-5ms relinks\n                      (or set PEONY_DAEMON=1 to auto-spawn one)\n  --cache-report FILE Write JSON cache reuse/fallback report\n  --gc-sections       Drop unreachable sections\n  --build-id          Emit .note.gnu.build-id\n  --no-crt            Do not auto-inject C runtime startup objects\n  -shared             Produce a shared object\n  --help              Print this help"
    );
}
