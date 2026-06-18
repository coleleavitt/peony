use std::sync::Mutex;

static SNAPSHOTS: Mutex<Vec<RssSnapshot>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct RssSnapshot {
    label: String,
    current_kb: u64,
    peak_kb: u64,
}

pub fn record_rss(label: impl Into<String>) {
    if !crate::is_enabled() {
        return;
    }
    let Some((current_kb, peak_kb)) = read_linux_rss_kb() else {
        return;
    };
    SNAPSHOTS
        .lock()
        .expect("profiler rss mutex poisoned")
        .push(RssSnapshot {
            label: label.into(),
            current_kb,
            peak_kb,
        });
}

pub(crate) fn reset() {
    SNAPSHOTS
        .lock()
        .expect("profiler rss mutex poisoned")
        .clear();
}

pub(crate) fn report() {
    let snapshots = SNAPSHOTS.lock().expect("profiler rss mutex poisoned");
    if snapshots.is_empty() {
        return;
    }
    eprintln!("── rss snapshots (KB) ──");
    eprintln!(
        "  {:<24} {:>10} {:>10} {:>10} {:>10}",
        "label", "current", "peak", "Δprev", "Δstart"
    );
    let mut previous = snapshots[0].current_kb;
    let start = previous;
    for row in snapshots.iter() {
        let delta_previous = row.current_kb as i64 - previous as i64;
        let delta_start = row.current_kb as i64 - start as i64;
        eprintln!(
            "  {:<24} {:>10} {:>10} {:>+10} {:>+10}",
            row.label, row.current_kb, row.peak_kb, delta_previous, delta_start
        );
        previous = row.current_kb;
    }
}

fn read_linux_rss_kb() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut current = None;
    let mut peak = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            current = parse_kb(value);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak = parse_kb(value);
        }
    }
    Some((current?, peak.or(current)?))
}

fn parse_kb(value: &str) -> Option<u64> {
    value.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_record_is_noop() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        crate::phase::disable_for_tests();
        super::reset();
        super::record_rss("disabled");
        assert!(
            super::SNAPSHOTS
                .lock()
                .expect("profiler rss mutex poisoned")
                .is_empty()
        );
    }

    #[test]
    fn parses_status_kb_value() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        assert_eq!(super::parse_kb("\t1234 kB"), Some(1234));
    }
}
