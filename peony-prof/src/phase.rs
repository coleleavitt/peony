use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static T0: Mutex<Option<Instant>> = Mutex::new(None);
static PHASES: Mutex<Vec<(String, PhaseStat)>> = Mutex::new(Vec::new());

#[derive(Default, Clone)]
struct PhaseStat {
    nanos: u128,
    spans: u64,
    bytes: u64,
    items: u64,
}

#[derive(Clone)]
pub(crate) struct PhaseRow {
    pub name: String,
    pub nanos: u128,
    pub spans: u64,
    pub bytes: u64,
    pub items: u64,
}

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    *T0.lock().expect("profiler clock mutex poisoned") = Some(Instant::now());
    PHASES
        .lock()
        .expect("profiler phase mutex poisoned")
        .clear();
    crate::reset_all();
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub struct Span {
    name: &'static str,
    start: Option<Instant>,
}

#[inline]
pub fn phase(name: &'static str) -> Span {
    let active = is_enabled();
    Span {
        name,
        start: active.then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        let elapsed = start.elapsed().as_nanos();
        let mut phases = PHASES.lock().expect("profiler phase mutex poisoned");
        match phases.iter_mut().find(|(name, _)| name == self.name) {
            Some((_, stat)) => {
                stat.nanos += elapsed;
                stat.spans += 1;
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

#[inline]
pub fn record_bytes(name: &'static str, bytes: u64) {
    add_counter(name, bytes, 0);
}

#[inline]
pub fn record_items(name: &'static str, items: u64) {
    add_counter(name, 0, items);
}

pub(crate) fn snapshot() -> (u128, Vec<PhaseRow>) {
    let total_ns = T0
        .lock()
        .expect("profiler clock mutex poisoned")
        .map(|t0| t0.elapsed().as_nanos())
        .unwrap_or(0)
        .max(1);
    let phases = PHASES.lock().expect("profiler phase mutex poisoned");
    let rows = phases
        .iter()
        .map(|(name, stat)| PhaseRow {
            name: name.clone(),
            nanos: stat.nanos,
            spans: stat.spans,
            bytes: stat.bytes,
            items: stat.items,
        })
        .collect();
    (total_ns, rows)
}

fn add_counter(name: &'static str, bytes: u64, items: u64) {
    if !is_enabled() {
        return;
    }
    let mut phases = PHASES.lock().expect("profiler phase mutex poisoned");
    match phases.iter_mut().find(|(phase_name, _)| phase_name == name) {
        Some((_, stat)) => {
            stat.bytes += bytes;
            stat.items += items;
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

#[cfg(test)]
pub(crate) fn disable_for_tests() {
    ENABLED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_noop() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        ENABLED.store(false, Ordering::Relaxed);
        PHASES
            .lock()
            .expect("profiler phase mutex poisoned")
            .clear();
        {
            let _span = phase("noop_phase");
            record_bytes("noop_phase", 100);
        }
        assert!(
            !PHASES
                .lock()
                .expect("profiler phase mutex poisoned")
                .iter()
                .any(|(name, _)| name == "noop_phase")
        );
        crate::report();
    }

    #[test]
    fn enabled_accumulates_phase_time_and_counters() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        enable();
        {
            let _span = phase("parse");
            std::thread::sleep(std::time::Duration::from_millis(2));
            record_bytes("parse", 4096);
            record_items("parse", 3);
        }
        let phases = PHASES.lock().expect("profiler phase mutex poisoned");
        let (_, stat) = phases
            .iter()
            .find(|(name, _)| name == "parse")
            .expect("phase present");
        assert!(stat.nanos >= 1_000_000);
        assert_eq!(stat.spans, 1);
        assert_eq!(stat.bytes, 4096);
        assert_eq!(stat.items, 3);
        ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn reentering_a_phase_accumulates() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        enable();
        for _ in 0..3 {
            let _span = phase("loop");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let phases = PHASES.lock().expect("profiler phase mutex poisoned");
        let (_, stat) = phases.iter().find(|(name, _)| name == "loop").unwrap();
        assert_eq!(stat.spans, 3);
        ENABLED.store(false, Ordering::Relaxed);
    }
}
