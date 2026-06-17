//! `peony-prof` — an in-linker phase profiler.
//!
//! The recurring pain when tuning peony was *guessing* where time went via
//! external `strace`/`perf`. This crate measures it from the inside: wrap each
//! link phase in a [`phase`] guard and, when profiling is enabled, peony prints
//! a breakdown table (phase, wall-clock, % of total, and optional byte/item
//! counters). When disabled (the default) every operation is a cheap atomic
//! load that short-circuits — no timers, no allocation, no table.
//!
//! ```ignore
//! peony_prof::enable();                 // driver does this on `--stats`
//! {
//!     let _g = peony_prof::phase("parse");
//!     // ... parse work ...
//!     peony_prof::record_bytes("parse", bytes_read);
//! }
//! peony_prof::report();                 // prints the table to stderr
//! ```
//!
//! Phases are recorded in first-`phase()`-call order so the table reads like the
//! pipeline. Re-entering a phase name accumulates (so a phase split across calls
//! sums correctly). Designed to be safe to call from any thread.

use std::cell::RefCell;
use std::panic::Location;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Global on/off switch. When false, every public entry point is a single
/// relaxed atomic load and an early return — so leaving profiling calls in the
/// hot path costs essentially nothing in a normal link.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Process start, captured the first time profiling is enabled, so the report
/// can show each phase's share of the whole run.
static T0: Mutex<Option<Instant>> = Mutex::new(None);

/// One accumulated phase: total nanoseconds, number of spans, and optional
/// user counters (bytes processed, items processed).
#[derive(Default, Clone)]
struct PhaseStat {
    nanos: u128,
    spans: u64,
    bytes: u64,
    items: u64,
}

/// Insertion-ordered phase table. A Vec keeps first-seen order (the pipeline
/// order); lookups are linear but the phase count is tiny (≈8).
static PHASES: Mutex<Vec<(String, PhaseStat)>> = Mutex::new(Vec::new());

/// Enable profiling. Idempotent; resets the clock and clears prior data so a
/// process that links several times reports the most recent run.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    *T0.lock().unwrap() = Some(Instant::now());
    PHASES.lock().unwrap().clear();
}

/// Is profiling on? Hot-path callers can gate expensive counter computation.
#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// An RAII span: times from creation to drop and accumulates into the named
/// phase. No-op (and stores no name) when profiling is disabled.
pub struct Span {
    name: &'static str,
    start: Instant,
    active: bool,
}

/// Begin timing a phase. The returned guard records the elapsed time into
/// `name` when dropped. Cheap no-op when disabled.
#[inline]
pub fn phase(name: &'static str) -> Span {
    let active = is_enabled();
    Span {
        name,
        start: Instant::now(),
        active,
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let elapsed = self.start.elapsed().as_nanos();
        let mut phases = PHASES.lock().unwrap();
        match phases.iter_mut().find(|(n, _)| n == self.name) {
            Some((_, s)) => {
                s.nanos += elapsed;
                s.spans += 1;
            }
            None => phases.push((
                self.name.to_string(),
                PhaseStat {
                    nanos: elapsed,
                    spans: 1,
                    bytes: 0,
                    items: 0,
                },
            )),
        }
    }
}

/// Add to a phase's byte counter (e.g. bytes parsed, bytes written). No-op when
/// disabled. Creates the phase row if it does not yet exist.
#[inline]
pub fn record_bytes(name: &'static str, bytes: u64) {
    add_counter(name, bytes, 0);
}

/// Add to a phase's item counter (e.g. objects, symbols, relocations).
#[inline]
pub fn record_items(name: &'static str, items: u64) {
    add_counter(name, 0, items);
}

fn add_counter(name: &'static str, bytes: u64, items: u64) {
    if !is_enabled() {
        return;
    }
    let mut phases = PHASES.lock().unwrap();
    match phases.iter_mut().find(|(n, _)| n == name) {
        Some((_, s)) => {
            s.bytes += bytes;
            s.items += items;
        }
        None => phases.push((
            name.to_string(),
            PhaseStat {
                nanos: 0,
                spans: 0,
                bytes,
                items,
            },
        )),
    }
}

// ── Call-flow tracing ────────────────────────────────────────────────────────
//
// The phase table above tells you WHICH phase is slow; the trace tree below
// tells you the FLOW — who called what, in order, at which source line — so a
// bug can be followed through the pipeline instead of guessed. Wrap a suspect
// function body in `let _t = peony_prof::trace("name");` and the captured tree
// shows the nested caller→callee path with file:line and per-frame timing.
//
// Tracing is gated behind `trace_enable()` (separate from `enable()`) because
// fine-grained per-call tracing is heavier than phase timing; you turn it on
// only when hunting a specific bug. Tree state is thread-local: each worker
// thread builds its own tree, avoiding cross-thread lock contention, and
// `trace_tree()` prints whichever thread asks (typically the main driver).

static TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

/// One node in the call-flow tree: a label, where it was entered (file:line),
/// nesting depth, elapsed nanos, and the order it was ENTERED (so the printed
/// tree preserves execution order even though nodes finish out of order). A node
/// with `is_event = true` is a zero-duration point marker (e.g. "symbol
/// conflict", "comdat excluded") logged AT a source line inside a frame, with an
/// optional `detail` string — this is what lets you see what happened per line,
/// not just which function ran.
#[derive(Clone)]
struct TraceNode {
    label: &'static str,
    loc: &'static Location<'static>,
    depth: usize,
    nanos: u128,
    enter_seq: u64,
    is_event: bool,
    detail: String,
}

thread_local! {
    /// Current nesting depth on this thread.
    static TRACE_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    /// Completed trace nodes for this thread, in finish order.
    static TRACE_NODES: RefCell<Vec<TraceNode>> = const { RefCell::new(Vec::new()) };
}

/// Monotonic enter sequence so the tree can be re-sorted into execution order.
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Turn on call-flow tracing. Implies [`enable`] so the report runs too.
pub fn trace_enable() {
    enable();
    TRACE_ENABLED.store(true, Ordering::Relaxed);
    TRACE_SEQ.store(0, Ordering::Relaxed);
    TRACE_NODES.with(|n| n.borrow_mut().clear());
    TRACE_DEPTH.with(|d| *d.borrow_mut() = 0);
}

/// An RAII trace frame: records `label` + the call site + nesting depth + time.
pub struct TraceFrame {
    label: &'static str,
    loc: &'static Location<'static>,
    start: Instant,
    depth: usize,
    enter_seq: u64,
    active: bool,
}

/// Enter a trace frame. `#[track_caller]` captures the CALLER's file:line, so the
/// tree shows where in the source the flow went. Cheap no-op when tracing is off.
#[inline]
#[track_caller]
pub fn trace(label: &'static str) -> TraceFrame {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return TraceFrame {
            label,
            loc: Location::caller(),
            start: Instant::now(),
            depth: 0,
            enter_seq: 0,
            active: false,
        };
    }
    let depth = TRACE_DEPTH.with(|d| {
        let mut d = d.borrow_mut();
        let cur = *d;
        *d += 1;
        cur
    });
    TraceFrame {
        label,
        loc: Location::caller(),
        start: Instant::now(),
        depth,
        enter_seq: TRACE_SEQ.fetch_add(1, Ordering::Relaxed),
        active: true,
    }
}

impl Drop for TraceFrame {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let nanos = self.start.elapsed().as_nanos();
        TRACE_DEPTH.with(|d| *d.borrow_mut() = self.depth);
        TRACE_NODES.with(|n| {
            n.borrow_mut().push(TraceNode {
                label: self.label,
                loc: self.loc,
                depth: self.depth,
                nanos,
                enter_seq: self.enter_seq,
                is_event: false,
                detail: String::new(),
            })
        });
    }
}

/// Log a point-in-flow EVENT at the current nesting depth: a labelled marker
/// captured at the calling source line, with an optional detail string. Use it
/// to see what happened *inside* a frame, line by line — e.g.
/// `event!("comdat-excluded", sig)`, `event!("symbol-conflict", name)`,
/// `event!("archive-round", format!("round {r}: pulled {n}"))`. Renders in the
/// trace tree at the line it was logged. Cheap no-op when tracing is off.
#[inline]
#[track_caller]
pub fn event(label: &'static str, detail: impl Into<String>) {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let depth = TRACE_DEPTH.with(|d| *d.borrow());
    let seq = TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let loc = Location::caller();
    let detail = detail.into();
    TRACE_NODES.with(|n| {
        n.borrow_mut().push(TraceNode {
            label,
            loc,
            depth,
            nanos: 0,
            enter_seq: seq,
            is_event: true,
            detail,
        })
    });
}

/// Print the call-flow tree for the CURRENT thread to stderr: nested
/// caller→callee frames in execution order, each with file:line and timing. Use
/// after a link to follow how the flow actually executed. No-op when tracing off.
pub fn trace_tree() {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut nodes = TRACE_NODES.with(|n| n.borrow().clone());
    if nodes.is_empty() {
        return;
    }
    // Re-order into execution (enter) order; depth then renders the nesting.
    nodes.sort_by_key(|n| n.enter_seq);
    eprintln!("\n── peony --trace (call flow) ──────────────────────────────");
    for n in &nodes {
        let indent = "  ".repeat(n.depth);
        let file = n
            .loc
            .file()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| n.loc.file());
        if n.is_event {
            // Point event: "• label: detail (file:line)" — no duration.
            let detail = if n.detail.is_empty() {
                String::new()
            } else {
                format!(": {}", n.detail)
            };
            eprintln!(
                "{indent}• {}{}  ({}:{})",
                n.label,
                detail,
                file,
                n.loc.line()
            );
        } else {
            eprintln!(
                "{indent}{} {:>9}  ({}:{})",
                n.label,
                fmt_ns(n.nanos),
                file,
                n.loc.line()
            );
        }
    }
    eprintln!("───────────────────────────────────────────────────────────");
}

/// A free-running counter not tied to a timed phase (e.g. syscall-ish events the
/// linker itself can count, like `file_opens`). Stored as an item counter under
/// the given name. Cheap no-op when disabled.
static COUNTERS: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Named global counters, parallel to `COUNTERS` by index.
const COUNTER_NAMES: [&str; 8] = [
    "file_opens",
    "file_reads",
    "bytes_read",
    "objects_parsed",
    "symbols_resolved",
    "relocs_applied",
    "sections_emitted",
    "spare",
];

/// Bump a named global counter (matched against [`COUNTER_NAMES`]). Always a
/// cheap relaxed add when enabled; no-op otherwise.
#[inline]
pub fn count(name: &str, n: u64) {
    if !is_enabled() {
        return;
    }
    if let Some(i) = COUNTER_NAMES.iter().position(|&c| c == name) {
        COUNTERS[i].fetch_add(n, Ordering::Relaxed);
    }
}

/// Print the phase + counter breakdown to stderr. No-op when disabled.
pub fn report() {
    if !is_enabled() {
        return;
    }
    let total_ns = T0
        .lock()
        .unwrap()
        .map(|t0| t0.elapsed().as_nanos())
        .unwrap_or(0)
        .max(1);
    let phases = PHASES.lock().unwrap();

    eprintln!("\n── peony --stats ──────────────────────────────────────────");
    eprintln!(
        "{:<18} {:>10} {:>7}  {:>12} {:>10}",
        "phase", "wall", "%", "bytes", "items"
    );
    let mut timed_ns: u128 = 0;
    for (name, s) in phases.iter() {
        timed_ns += s.nanos;
        let pct = (s.nanos as f64 / total_ns as f64) * 100.0;
        eprintln!(
            "{:<18} {:>10} {:>6.1}% {:>12} {:>10}",
            name,
            fmt_ns(s.nanos),
            pct,
            human(s.bytes),
            s.items,
        );
    }
    let other = total_ns.saturating_sub(timed_ns);
    eprintln!(
        "{:<18} {:>10} {:>6.1}%  (startup/teardown/untimed)",
        "other",
        fmt_ns(other),
        (other as f64 / total_ns as f64) * 100.0
    );
    eprintln!("{:<18} {:>10}", "TOTAL", fmt_ns(total_ns));

    // Global counters (only print non-zero ones).
    let mut printed_header = false;
    for (i, name) in COUNTER_NAMES.iter().enumerate() {
        let v = COUNTERS[i].load(Ordering::Relaxed);
        if v == 0 {
            continue;
        }
        if !printed_header {
            eprintln!("── counters ──");
            printed_header = true;
        }
        eprintln!("  {name:<18} {v}");
    }
    eprintln!("───────────────────────────────────────────────────────────");

    // If call-flow tracing was on, follow it with the flow tree.
    trace_tree();
}

fn fmt_ns(ns: u128) -> String {
    if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

fn human(bytes: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut b = bytes as f64;
    let mut i = 0;
    while b >= 1024.0 && i < U.len() - 1 {
        b /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes}B")
    } else {
        format!("{b:.1}{}", U[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests mutate process-global profiler state, so they must not run
    // concurrently. Serialise them on one mutex.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disabled_is_noop() {
        let _t = TEST_LOCK.lock().unwrap();
        // With profiling off, phases record nothing and report is silent.
        ENABLED.store(false, Ordering::Relaxed);
        PHASES.lock().unwrap().clear();
        let before = COUNTERS[0].load(Ordering::Relaxed);
        {
            let _g = phase("noop_phase");
            record_bytes("noop_phase", 100);
            count("file_opens", 5);
        }
        // Disabled ⇒ nothing recorded: no phase row, counter unchanged.
        assert!(
            !PHASES
                .lock()
                .unwrap()
                .iter()
                .any(|(n, _)| n == "noop_phase"),
            "disabled profiling must not create phase rows"
        );
        assert_eq!(
            COUNTERS[0].load(Ordering::Relaxed),
            before,
            "disabled profiling must not bump counters"
        );
        report(); // must not panic when disabled
    }

    #[test]
    fn enabled_accumulates_phase_time_and_counters() {
        let _t = TEST_LOCK.lock().unwrap();
        enable();
        COUNTERS[0].store(0, Ordering::Relaxed);
        {
            let _g = phase("parse");
            std::thread::sleep(std::time::Duration::from_millis(2));
            record_bytes("parse", 4096);
            record_items("parse", 3);
        }
        count("file_opens", 7);
        let phases = PHASES.lock().unwrap();
        let (_, s) = phases
            .iter()
            .find(|(n, _)| n == "parse")
            .expect("phase present");
        assert!(s.nanos >= 1_000_000, "≥1ms recorded, got {}ns", s.nanos);
        assert_eq!(s.spans, 1);
        assert_eq!(s.bytes, 4096);
        assert_eq!(s.items, 3);
        drop(phases);
        assert_eq!(COUNTERS[0].load(Ordering::Relaxed), 7);
        // reset for other tests
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn reentering_a_phase_accumulates() {
        let _t = TEST_LOCK.lock().unwrap();
        enable();
        for _ in 0..3 {
            let _g = phase("loop");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let phases = PHASES.lock().unwrap();
        let (_, s) = phases.iter().find(|(n, _)| n == "loop").unwrap();
        assert_eq!(s.spans, 3, "three spans summed under one phase");
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_builds_nested_caller_callee_tree() {
        let _t = TEST_LOCK.lock().unwrap();
        trace_enable();
        {
            let _outer = trace("outer");
            {
                let _inner = trace("inner");
                let _leaf = trace("leaf");
            } // leaf, inner drop here
        } // outer drops here
        let nodes = TRACE_NODES.with(|n| n.borrow().clone());
        // Three frames captured, with increasing depth following the call nesting.
        let by_label = |l: &str| nodes.iter().find(|n| n.label == l).expect("frame present");
        assert_eq!(by_label("outer").depth, 0);
        assert_eq!(by_label("inner").depth, 1);
        assert_eq!(by_label("leaf").depth, 2);
        // Enter order is preserved: outer < inner < leaf.
        assert!(by_label("outer").enter_seq < by_label("inner").enter_seq);
        assert!(by_label("inner").enter_seq < by_label("leaf").enter_seq);
        // Each frame captured a real source line.
        assert!(by_label("outer").loc.line() > 0);
        TRACE_ENABLED.store(false, Ordering::Relaxed);
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_disabled_is_noop() {
        let _t = TEST_LOCK.lock().unwrap();
        TRACE_ENABLED.store(false, Ordering::Relaxed);
        TRACE_NODES.with(|n| n.borrow_mut().clear());
        {
            let _f = trace("x");
            event("e", "d");
        }
        assert!(
            TRACE_NODES.with(|n| n.borrow().is_empty()),
            "disabled tracing records no nodes or events"
        );
    }

    #[test]
    fn events_logged_at_frame_depth_with_detail() {
        let _t = TEST_LOCK.lock().unwrap();
        trace_enable();
        {
            let _f = trace("phase");
            event("conflict", "sym_foo");
            event("excluded", String::from("comdat_bar"));
        }
        let nodes = TRACE_NODES.with(|n| n.borrow().clone());
        let ev: Vec<_> = nodes.iter().filter(|n| n.is_event).collect();
        assert_eq!(ev.len(), 2, "two events recorded");
        // Events sit at depth 1 (inside the frame) and carry their detail.
        assert!(ev.iter().all(|e| e.depth == 1));
        assert!(
            ev.iter()
                .any(|e| e.label == "conflict" && e.detail == "sym_foo")
        );
        assert!(ev.iter().any(|e| e.detail == "comdat_bar"));
        // Events have zero duration.
        assert!(ev.iter().all(|e| e.nanos == 0));
        TRACE_ENABLED.store(false, Ordering::Relaxed);
        ENABLED.store(false, Ordering::Relaxed);
    }
}
