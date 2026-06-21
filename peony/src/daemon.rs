//! Resident linker daemon — break the one-shot-CLI floor toward a sub-5ms relink.
//!
//! A one-shot `--incremental` relink still pays, every invocation, for things a
//! resident process could keep in RAM: the library search + archive scan, the
//! ~MB `layout.bin` deserialize, and rebuilding the cached symbol view. The
//! daemon loads all of that ONCE and serves relinks from memory, so a relink
//! costs only re-parse-the-changed-object + emit — the genuine incremental core.
//!
//! Model: `peony --daemon -o app @args` loads the cache a prior
//! `peony --incremental` link wrote, then serves on a Unix socket at
//! `app.incr/daemon.sock`. A normal `peony --incremental` invocation (the
//! client — e.g. spawned fresh by the build system) delegates to a live daemon
//! and exits; with no daemon it runs the usual one-shot path.
//!
//! Protocol: the client writes the 8-byte `args_hash`; the server replies with
//! one status byte — 0 relinked, 1 no change, 2 fall back to a one-shot link.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use peony_cache::{CachedSymbolEntry, FastFingerprint, FrontEndSnapshot};
use peony_emit::EmitConfig;
use peony_layout::Layout;
use peony_symbols::SymbolTable;

const STATUS_RELINKED: u8 = 0;
const STATUS_NOCHANGE: u8 = 1;
const STATUS_FALLBACK: u8 = 2;

fn socket_path(output: &Path) -> PathBuf {
    peony_cache::cache_dir(output).join("daemon.sock")
}

fn blob_mtime(output: &Path) -> Option<SystemTime> {
    std::fs::metadata(peony_cache::layout_blob_path(output))
        .and_then(|m| m.modified())
        .ok()
}

/// Client side: if a daemon is serving `output`, ask it to relink. Returns
/// `Some(true)` if it handled the link (relinked or no-op), `Some(false)` if it
/// asked us to fall back to a one-shot link, `None` if no daemon is reachable.
pub(crate) fn try_delegate(output: &Path, args_hash: u64) -> Option<bool> {
    let mut stream = UnixStream::connect(socket_path(output)).ok()?;
    stream.write_all(&args_hash.to_le_bytes()).ok()?;
    stream.flush().ok()?;
    let mut status = [0u8; 1];
    stream.read_exact(&mut status).ok()?;
    match status[0] {
        STATUS_RELINKED | STATUS_NOCHANGE => Some(true),
        _ => Some(false),
    }
}

/// Auto-spawn a background daemon for `output` when `PEONY_DAEMON=1` is set, no
/// daemon is already serving it, and a cache exists to load — then wait briefly
/// for it to come up. This makes the sub-5ms path automatic inside a dev shell
/// (`export PEONY_DAEMON=1`) without affecting clean/CI builds or the test
/// suite. The spawned daemon replicates our argv + `--daemon`, detaches its IO
/// to `<output>.incr/daemon.log`, and idle-times-out on its own.
pub(crate) fn ensure_autospawn(output: &Path) {
    if std::env::var("PEONY_DAEMON").as_deref() != Ok("1") {
        return;
    }
    // Already serving, or no cache to load yet (first link) → nothing to do.
    if UnixStream::connect(socket_path(output)).is_ok() {
        return;
    }
    if !peony_cache::layout_blob_path(output).exists() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&argv)
        .arg("--daemon")
        .stdin(std::process::Stdio::null());
    // Detach the daemon's output to a log file in the cache dir (so it does not
    // clutter the client's terminal); fall back to /dev/null.
    match std::fs::File::create(peony_cache::cache_dir(output).join("daemon.log")) {
        Ok(out) => {
            let err = out.try_clone();
            cmd.stdout(std::process::Stdio::from(out));
            cmd.stderr(match err {
                Ok(e) => std::process::Stdio::from(e),
                Err(_) => std::process::Stdio::null(),
            });
        }
        Err(_) => {
            cmd.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
    }
    if cmd.spawn().is_err() {
        return;
    }
    // Wait up to ~1s for the daemon to bind its socket.
    let sock = socket_path(output);
    let deadline = Instant::now() + Duration::from_secs(1);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Server side: load the incremental state into RAM and serve relinks until
/// killed or idle. Requires a prior `--incremental` link to have written the
/// cache. Idle timeout: `PEONY_DAEMON_IDLE_SECS` (default 300s).
pub(crate) fn serve(output: &Path, input_paths: &[PathBuf], args_hash: u64) -> Result<()> {
    // If another daemon already bound the socket (a race between two clients
    // auto-spawning), defer to it and exit quietly.
    if UnixStream::connect(socket_path(output)).is_ok() {
        return Ok(());
    }
    let mut state = DaemonState::load(output, input_paths, args_hash)?.context(
        "no incremental cache for this output; run `peony --incremental` once before `--daemon`",
    )?;
    let sock = socket_path(output);
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)
        .with_context(|| format!("bind daemon socket {}", sock.display()))?;
    listener
        .set_nonblocking(true)
        .context("set daemon socket non-blocking")?;
    let idle_timeout = Duration::from_secs(
        std::env::var("PEONY_DAEMON_IDLE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300),
    );
    eprintln!(
        "peony daemon: serving {} ({} objects resident) on {} (idle timeout {}s)",
        output.display(),
        state.snapshot.object_paths.len(),
        sock.display(),
        idle_timeout.as_secs()
    );
    let mut last_active = Instant::now();
    loop {
        match listener.accept() {
            Ok((mut conn, _)) => {
                let _ = conn.set_nonblocking(false);
                let mut buf = [0u8; 8];
                if conn.read_exact(&mut buf).is_ok() {
                    let status = state.handle(u64::from_le_bytes(buf));
                    let _ = conn.write_all(&[status]);
                    let _ = conn.flush();
                }
                last_active = Instant::now();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if last_active.elapsed() >= idle_timeout {
                    let _ = std::fs::remove_file(&sock);
                    eprintln!("peony daemon: idle, shutting down");
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// The expensive incremental state, held resident in RAM.
struct DaemonState {
    output: PathBuf,
    input_paths: Vec<PathBuf>,
    args_hash: u64,
    layout: Layout,
    symbols: SymbolTable,
    snapshot: FrontEndSnapshot,
    cached_symbols: Vec<CachedSymbolEntry>,
    /// Cheap stat fingerprints per input, to detect what changed since the last
    /// relink without re-reading any file.
    input_fps: Vec<FastFingerprint>,
    /// mtime of `layout.bin` when loaded; if it changes, a client fell back to a
    /// full link and rewrote the layout, so we reload before serving.
    blob_mtime: Option<SystemTime>,
    emit_config: EmitConfig,
}

impl DaemonState {
    fn load(output: &Path, input_paths: &[PathBuf], args_hash: u64) -> Result<Option<Self>> {
        let Some(cached) = peony_cache::load_changed_state(output, input_paths, args_hash)? else {
            return Ok(None);
        };
        let Some(snapshot) = cached.front_end else {
            return Ok(None);
        };
        let blob = match peony_cache::read_layout_blob(output)?
            .filter(|b| peony_cache::blob_hash(b) == snapshot.blob_hash)
        {
            Some(b) => b,
            None => return Ok(None),
        };
        let Some(layout) = peony_layout::deserialize_layout(&blob) else {
            return Ok(None);
        };
        let symbols = crate::build_cached_symbol_view(&cached.symbols);
        // Baseline = the fingerprints the manifest was LAST LINKED against, NOT
        // a re-stat of the current files. This lets the daemon notice an edit
        // that landed before it loaded (e.g. when auto-spawned mid-relink) — the
        // first relink request then applies that pending change instead of
        // seeing "no change" against an already-edited file.
        let baseline = peony_cache::manifest_fast_inputs(output)?.unwrap_or_default();
        let input_fps = input_paths
            .iter()
            .map(|p| {
                let s = p.display().to_string();
                baseline
                    .iter()
                    .find(|(bp, _)| bp == &s)
                    .map(|(_, fp)| *fp)
                    .unwrap_or_default()
            })
            .collect();
        Ok(Some(Self {
            output: output.to_path_buf(),
            input_paths: input_paths.to_vec(),
            args_hash,
            layout,
            symbols,
            snapshot,
            cached_symbols: cached.symbols,
            input_fps,
            blob_mtime: blob_mtime(output),
            emit_config: EmitConfig::default(),
        }))
    }

    fn handle(&mut self, req_hash: u64) -> u8 {
        if req_hash != self.args_hash {
            return STATUS_FALLBACK;
        }
        // A client that fell back to a one-shot full link rewrote `layout.bin`;
        // reload our resident state before serving.
        if blob_mtime(&self.output) != self.blob_mtime {
            match DaemonState::load(&self.output, &self.input_paths, self.args_hash) {
                Ok(Some(fresh)) => *self = fresh,
                _ => return STATUS_FALLBACK,
            }
        }
        // Find changed objects by a single stat() per input — no reads.
        let mut changed_ids: Vec<usize> = Vec::new();
        for (i, p) in self.input_paths.iter().enumerate() {
            let fp = FastFingerprint::of_file(p).unwrap_or_default();
            if fp == self.input_fps[i] {
                continue;
            }
            let s = p.display().to_string();
            match self.snapshot.object_paths.iter().position(|op| op == &s) {
                Some(idx) => changed_ids.push(idx),
                None => return STATUS_FALLBACK, // a non-object input changed
            }
        }
        if changed_ids.is_empty() {
            return STATUS_NOCHANGE;
        }
        let started = std::time::Instant::now();
        // In-RAM parse-only relink: the layout + symbol view never leave memory.
        let emitted = crate::emit_parse_only_changed(
            &self.output,
            &self.layout,
            &self.symbols,
            &self.snapshot.object_paths,
            &self.snapshot.object_digests,
            &changed_ids,
            &self.emit_config,
        );
        match emitted {
            Ok(Some(_)) => {
                // Refresh resident fingerprints + the on-disk manifest (small;
                // layout.bin unchanged) so a later one-shot or restart is in sync.
                for (i, p) in self.input_paths.iter().enumerate() {
                    self.input_fps[i] = FastFingerprint::of_file(p).unwrap_or_default();
                }
                let sections = crate::section_records(&self.layout).unwrap_or_default();
                let _ = peony_cache::record_link_with_sections(
                    &self.output,
                    &self.input_paths,
                    self.args_hash,
                    &sections,
                    &self.cached_symbols,
                    Some(&self.snapshot),
                    None,
                );
                self.blob_mtime = blob_mtime(&self.output);
                eprintln!(
                    "peony daemon: relinked {} ({} object(s)) in {:.2}ms (in-RAM)",
                    self.output.display(),
                    changed_ids.len(),
                    started.elapsed().as_secs_f64() * 1e3
                );
                STATUS_RELINKED
            }
            _ => STATUS_FALLBACK,
        }
    }
}
