//! `peony-prof` — an in-linker phase profiler.

mod counters;
mod fmt;
mod phase;
mod rss;
mod trace;
mod trace_detail;
mod trace_fields;
mod trace_render;
#[cfg(test)]
mod trace_tests;

pub use counters::count;
pub use phase::{Span, enable, is_enabled, phase, record_bytes, record_items};
pub use rss::record_rss;
pub use trace::{
    TraceFrame,
    detail_event_fields,
    event,
    event_fields,
    trace,
    trace_detail_enable,
    trace_detail_enabled,
    trace_enable,
    trace_fields,
    trace_stack_detail_enable,
    trace_stack_enable,
};
pub use trace_fields::TraceField;
pub use trace_render::trace_tree;

pub fn report() {
    if !is_enabled() {
        return;
    }
    let (total_ns, phases) = phase::snapshot();

    eprintln!("\n── peony --stats ──────────────────────────────────────────");
    eprintln!(
        "{:<18} {:>10} {:>7} {:>7} {:>12} {:>10} {:>12}",
        "phase", "wall", "%", "spans", "bytes", "items", "rate"
    );

    let mut timed_ns: u128 = 0;
    for row in &phases {
        timed_ns += row.nanos;
        let pct = (row.nanos as f64 / total_ns as f64) * 100.0;
        eprintln!(
            "{:<18} {:>10} {:>6.1}% {:>7} {:>12} {:>10} {:>12}",
            row.name,
            fmt::fmt_ns(row.nanos),
            pct,
            row.spans,
            fmt::human(row.bytes),
            row.items,
            fmt::rate(row.bytes, row.items, row.nanos),
        );
    }

    let other = total_ns.saturating_sub(timed_ns);
    eprintln!(
        "{:<18} {:>10} {:>6.1}%  (startup/teardown/untimed)",
        "other",
        fmt::fmt_ns(other),
        (other as f64 / total_ns as f64) * 100.0
    );
    eprintln!("{:<18} {:>10}", "TOTAL", fmt::fmt_ns(total_ns));

    counters::report();
    rss::report();
    eprintln!("───────────────────────────────────────────────────────────");
    trace_tree();
}

pub(crate) fn reset_all() {
    counters::reset();
    rss::reset();
    trace::reset();
}

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
