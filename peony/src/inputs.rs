use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use peony_layout::{ScriptLayout, ScriptOutputSection};

use crate::args::{Args, LibraryMode, LinkSpecKind, ResolvedInput};

/// Expand any GNU linker-script inputs (e.g. the system `libc.so`, which is a
/// `GROUP(...)` script) into the real object/library files they reference,
/// recursively.
pub(crate) fn expand_inputs(
    inputs: Vec<ResolvedInput>,
    search: &[PathBuf],
) -> Result<Vec<ResolvedInput>> {
    let mut out = Vec::new();
    let mut work: VecDeque<ResolvedInput> = inputs.into();
    while let Some(input) = work.pop_front() {
        if peony_object::classify_file(&input.path) == peony_object::FileKind::Archive {
            out.push(input);
            continue;
        }
        if !is_linker_script(&input.path) {
            out.push(input);
            continue;
        }
        let dir = input.path.parent().map(Path::to_path_buf);
        for r in parse_linker_script(&input.path)? {
            match resolve_script_ref(&r, dir.as_deref(), search) {
                Some(rp) => work.push_back(ResolvedInput {
                    path: rp,
                    whole_archive: input.whole_archive,
                    as_needed: input.as_needed,
                    start_lib_member: input.start_lib_member,
                }),
                None => tracing::warn!(
                    "linker script `{}`: cannot resolve `{r}`",
                    input.path.display()
                ),
            }
        }
    }
    Ok(out)
}

pub(crate) fn resolved_input_paths(inputs: &[ResolvedInput]) -> Vec<PathBuf> {
    inputs.iter().map(|i| i.path.clone()).collect()
}

/// A text file referencing GROUP/INPUT (and not an ELF/archive) is a linker script.
///
/// Fast path: peek only the leading magic bytes and reject ELF objects /
/// archives — the overwhelming majority of inputs — WITHOUT reading the whole
/// file. The old code `read`-the-entire-file of every input just to magic-check
/// it (on a 419-object link that re-read ~18MB the parser then mmaps anyway, and
/// on a 1-object link it was ~50% of the link). Only a genuine non-ELF input
/// (the rare linker script) is read in full.
fn is_linker_script(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 8];
    let n = f.read(&mut magic).unwrap_or(0);
    let head = &magic[..n];
    if head.starts_with(&peony_object::elf::ELFMAG)
        || head.starts_with(b"!<arch>\n")
        || head.starts_with(b"!<thin>\n")
    {
        return false;
    }
    // Not ELF/archive: read the rest and scan for linker-script directives.
    let mut data = magic[..n].to_vec();
    if f.read_to_end(&mut data).is_err() {
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
pub(crate) struct ScriptControls {
    pub(crate) entry: Option<String>,
    pub(crate) base_address: Option<String>,
    pub(crate) layout: ScriptLayout,
}

pub(crate) fn parse_linker_script_controls(paths: &[PathBuf]) -> Result<ScriptControls> {
    let mut merged = ScriptControls::default();
    for path in paths {
        let data = std::fs::read(path)
            .with_context(|| format!("reading linker script `{}`", path.display()))?;
        let text = strip_block_comments(&String::from_utf8_lossy(&data));
        reject_unsupported_linker_script(&text, path)?;
        if let Some(entry) = directive_arg(&text, "ENTRY") {
            merged.entry = Some(entry);
        }
        if let Some(base) = script_base_address(&text) {
            merged.base_address = Some(base);
        }
        let layout = parse_sections_layout(&text);
        merged.layout.keep_patterns.extend(layout.keep_patterns);
        merged.layout.output_sections.extend(layout.output_sections);
    }
    Ok(merged)
}

fn reject_unsupported_linker_script(text: &str, path: &Path) -> Result<()> {
    const UNSUPPORTED: &[&str] = &[
        "PROVIDE",
        "ASSERT",
        "PHDRS",
        "MEMORY",
        "SORT",
        "OVERLAY",
        "DATA_SEGMENT_RELRO_END",
        "DATA_SEGMENT_ALIGN",
        "INCLUDE",
        "OUTPUT_FORMAT",
        "OUTPUT_ARCH",
    ];
    for directive in UNSUPPORTED {
        if script_has_token(text, directive) {
            anyhow::bail!(
                "linker script `{}` uses unsupported directive `{directive}`",
                path.display()
            );
        }
    }
    Ok(())
}

fn script_has_token(text: &str, token: &str) -> bool {
    let mut search = 0;
    while let Some(pos) = text[search..].find(token) {
        let idx = search + pos;
        let before_ok = idx == 0
            || !text.as_bytes()[idx - 1].is_ascii_alphanumeric()
                && text.as_bytes()[idx - 1] != b'_';
        let after = idx + token.len();
        let after_ok = after >= text.len()
            || !text.as_bytes()[after].is_ascii_alphanumeric() && text.as_bytes()[after] != b'_';
        if before_ok && after_ok {
            return true;
        }
        search = after;
    }
    false
}

fn extract_script_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut pos = 0;
    while let Some((body, end)) = next_ref_directive_body(text, pos) {
        for r in parse_ref_tokens(body) {
            if !refs.contains(&r) {
                refs.push(r);
            }
        }
        pos = end;
    }
    refs
}

fn next_ref_directive_body<'a>(text: &'a str, start: usize) -> Option<(&'a str, usize)> {
    ["GROUP", "INPUT", "AS_NEEDED"]
        .into_iter()
        .filter_map(|keyword| directive_body_with_pos(text, keyword, start))
        .min_by_key(|(pos, _, _)| *pos)
        .map(|(_, body, end)| (body, end))
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
                    || t.ends_with(".o")
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
            let section_body = &body[open + 1..close];
            layout.keep_patterns.extend(keep_patterns(section_body));
            let mut patterns = section_patterns(section_body);
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

fn keep_patterns(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((keep_body, end)) = directive_body(body, "KEEP", pos) {
        for pat in section_patterns(keep_body) {
            if !out.iter().any(|p| p == &pat) {
                out.push(pat);
            }
        }
        pos = end;
    }
    out
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
    directive_body_with_pos(text, keyword, start).map(|(_, body, end)| (body, end))
}

fn directive_body_with_pos<'a>(
    text: &'a str,
    keyword: &str,
    start: usize,
) -> Option<(usize, &'a str, usize)> {
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
                    return Some((kw, &text[open + 1..close], close + 1));
                }
            } else if open < text.len() && text.as_bytes()[open] == b'{' {
                if let Some(close) = matching_brace(text, open) {
                    return Some((kw, &text[open + 1..close], close + 1));
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

pub(crate) fn strip_block_comments(s: &str) -> String {
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

/// Resolve positional inputs plus `-l<name>` libraries into the final ordered
/// input list. Shared libraries are preferred unless `-Bstatic` is active, but
/// `-Bdynamic` still falls back to an archive if no shared object exists; GNU ld
/// does this for compatibility with system libraries such as glibc's `libutil`.
///
/// The search list is the explicit `-L` directories followed by the system
/// library directories discovered from the host C toolchain (`gcc
/// -print-search-dirs`) plus the standard multiarch locations. This lets
/// `-lgcc_s`, `-lc`, etc. resolve without the caller passing every `-L`, exactly
/// as GNU ld / lld do when driven by `cc`.
pub(crate) fn resolve_inputs(args: &Args) -> Result<Vec<ResolvedInput>> {
    let mut inputs = Vec::new();
    let search = library_search_paths(&args.library_paths);
    for spec in &args.link_specs {
        let path = match &spec.kind {
            LinkSpecKind::Path(path) => path.clone(),
            LinkSpecKind::Library(name) => resolve_library(name, spec.library_mode, &search)?,
        };
        inputs.push(ResolvedInput {
            path,
            whole_archive: spec.whole_archive,
            as_needed: spec.as_needed,
            start_lib_member: spec.start_lib_member,
        });
    }
    if inputs.is_empty() {
        anyhow::bail!("no input files");
    }
    Ok(inputs)
}

fn resolve_library(name: &str, mode: LibraryMode, search: &[PathBuf]) -> Result<PathBuf> {
    let exts: &[&str] = match mode {
        LibraryMode::Any => &["so", "a"],
        LibraryMode::Static => &["a"],
        LibraryMode::Dynamic => &["so", "a"],
    };
    search
        .iter()
        .flat_map(|d| {
            exts.iter()
                .map(move |ext| d.join(format!("lib{name}.{ext}")))
        })
        .find(|p| p.exists())
        .ok_or_else(|| {
            let suffixes = exts
                .iter()
                .map(|ext| format!("lib{name}.{ext}"))
                .collect::<Vec<_>>()
                .join("/");
            anyhow::anyhow!("cannot find -l{name} ({suffixes}) on the library path")
        })
}

/// Inject C-runtime startup objects (`Scrt1.o crti.o crtbeginS.o … crtendS.o
/// crtn.o`) around the user inputs, as the `cc` driver does, when none is already
/// present. Only applied for PIE executables. Freestanding PIE links can pass
/// `--no-crt`/`-nostartfiles` to avoid startup-object injection without forcing
/// Peony to scan every input object before parse.
///
/// Returns the augmented input list plus the paths of any CRT objects that were
/// injected (empty when none were). A freestanding link that defines its own
/// `_start` collides with `Scrt1.o`'s `_start`; rather than pre-scanning every
/// input for `_start` (a per-object cost on large `cc` links), the driver lets
/// resolution surface that duplicate and re-links without these injected objects
/// (see `load_and_resolve_without_crt_on_start_conflict`).
pub(crate) fn inject_crt_objects(
    inputs: Vec<ResolvedInput>,
    args: &Args,
) -> (Vec<ResolvedInput>, Vec<PathBuf>) {
    let _t = peony_prof::trace("inject_crt_objects");
    // Heuristic: skip if a Scrt1/crt1 object is already on the command line.
    let already = inputs.iter().any(|p| {
        p.path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == "Scrt1.o" || n == "crt1.o" || n == "rcrt1.o")
    });
    // Only auto-inject for a PIE (the case rustc/cc drive without crt objects).
    if already || !args.pie || args.no_crt {
        return (inputs, Vec::new());
    }

    // Locate the crt objects. First try the cached GCC library dirs (no
    // subprocess — `gcc_library_dirs()` is memoised); only fall back to
    // `gcc -print-file-name` if a crt isn't on those dirs. The old code spawned
    // gcc 5× and each call PATH-walked (~117 execve on a typical link).
    let find = |name: &str| -> Option<PathBuf> {
        for dir in gcc_library_dirs() {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
        let gcc = ["/usr/bin/gcc", "/usr/local/bin/gcc", "/usr/bin/cc"]
            .into_iter()
            .find(|p| std::fs::metadata(p).is_ok())
            .unwrap_or("gcc");
        let out = std::process::Command::new(gcc)
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
        return (inputs, Vec::new()); // toolchain crt unavailable — leave inputs as-is
    };

    // The exact paths injected, so the driver can re-link without them if the
    // user turns out to provide its own `_start` (freestanding link).
    let injected = vec![c1.clone(), ci.clone(), cb.clone(), ce.clone(), cn.clone()];
    let mut out = Vec::with_capacity(inputs.len() + 5);
    out.push(ResolvedInput {
        path: c1,
        whole_archive: false,
        as_needed: false,
        start_lib_member: false,
    });
    out.push(ResolvedInput {
        path: ci,
        whole_archive: false,
        as_needed: false,
        start_lib_member: false,
    });
    out.push(ResolvedInput {
        path: cb,
        whole_archive: false,
        as_needed: false,
        start_lib_member: false,
    });
    out.extend(inputs);
    out.push(ResolvedInput {
        path: ce,
        whole_archive: false,
        as_needed: false,
        start_lib_member: false,
    });
    out.push(ResolvedInput {
        path: cn,
        whole_archive: false,
        as_needed: false,
        start_lib_member: false,
    });
    (out, injected)
}

/// The full library search path: explicit `-L` dirs first (highest priority),
/// then GCC's own library directories, then standard system locations.
pub(crate) fn library_search_paths(explicit: &[PathBuf]) -> Vec<PathBuf> {
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("peony-inputs-tests")
            .join(format!("{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dynamic_library_mode_falls_back_to_archive() {
        let dir = temp_test_dir("dynamic-archive-fallback");
        let archive = dir.join("libcompat.a");
        std::fs::write(&archive, b"!<arch>\n").unwrap();

        assert_eq!(
            resolve_library("compat", LibraryMode::Dynamic, &[dir.clone()]).unwrap(),
            archive
        );

        let _ = std::fs::remove_file(dir.join("libcompat.a"));
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn script_refs_keep_first_seen_order_across_directives() {
        let refs = extract_script_refs(
            r#"
            GROUP(first.o -lfoo)
            AS_NEEDED(second.so first.o)
            INPUT(third.a "/abs/fourth.o")
            "#,
        );

        assert_eq!(
            refs,
            vec![
                "first.o".to_string(),
                "-lfoo".to_string(),
                "second.so".to_string(),
                "third.a".to_string(),
                "/abs/fourth.o".to_string(),
            ]
        );
    }

    #[test]
    fn script_controls_extract_entry_base_and_sections() {
        let layout = parse_sections_layout(
            r#"
            ENTRY(_custom_start)
            SECTIONS {
                . = 0x500000;
                .text : { KEEP(*(.init)) *(.text .text.*) }
                .rodata : { *(.rodata .rodata.*) }
            }
            "#,
        );

        assert_eq!(
            script_base_address("SECTIONS { . = 0x500000; .text : { *(.text) } }"),
            Some("0x500000".to_string())
        );
        assert_eq!(
            directive_arg("ENTRY(_custom_start)", "ENTRY"),
            Some("_custom_start".to_string())
        );
        assert_eq!(layout.output_sections.len(), 2);
        assert_eq!(layout.output_sections[0].name, ".text");
        assert_eq!(
            layout.output_sections[0].patterns,
            vec![".init", ".text", ".text.*"]
        );
        assert_eq!(layout.keep_patterns, vec![".init"]);
    }
}
