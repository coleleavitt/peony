use std::sync::atomic::{AtomicU64, Ordering};

static COUNTERS: [AtomicU64; 24] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

const COUNTER_NAMES: [&str; 24] = [
    "file_opens",
    "file_reads",
    "bytes_read",
    "objects_parsed",
    "symbols_resolved",
    "relocs_scanned",
    "relocs_applied",
    "sections_emitted",
    "archive_index_checks",
    "archive_index_skips",
    "archive_members_parsed",
    "archive_members_pulled",
    "archive_members_seen",
    "archive_strong_undefs",
    "got_slots",
    "plt_slots",
    "tls_gd_refs",
    "tls_ie_refs",
    "tls_desc_refs",
    "copy_relocs",
    "dynamic_imports",
    "dynamic_exports",
    "layout_sections",
    "layout_segments",
];

#[inline]
pub fn count(name: &str, n: u64) {
    if !crate::is_enabled() {
        return;
    }
    if let Some(index) = COUNTER_NAMES.iter().position(|counter| *counter == name) {
        COUNTERS[index].fetch_add(n, Ordering::Relaxed);
    }
}

pub(crate) fn reset() {
    for counter in &COUNTERS {
        counter.store(0, Ordering::Relaxed);
    }
}

pub(crate) fn report() {
    let mut printed_header = false;
    for (index, name) in COUNTER_NAMES.iter().enumerate() {
        let value = COUNTERS[index].load(Ordering::Relaxed);
        if value == 0 {
            continue;
        }
        if !printed_header {
            eprintln!("── counters ──");
            printed_header = true;
        }
        eprintln!("  {name:<22} {value}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn count_is_disabled_when_profiler_is_off() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        crate::phase::disable_for_tests();
        super::reset();
        super::count("file_opens", 7);
        assert_eq!(
            super::COUNTERS[0].load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn count_accumulates_known_counter() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        crate::enable();
        super::count("file_opens", 7);
        super::count("file_opens", 5);
        assert_eq!(
            super::COUNTERS[0].load(std::sync::atomic::Ordering::Relaxed),
            12
        );
        crate::phase::disable_for_tests();
    }
}
