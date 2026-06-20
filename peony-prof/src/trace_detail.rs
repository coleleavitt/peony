use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static DETAIL_LIMIT: AtomicU64 = AtomicU64::new(256);
static DETAIL_COUNTS: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

pub(crate) enum DetailDecision {
    Allow,
    Deny,
    LimitReached { limit: u64, seen: u64 },
}

pub(crate) fn set_limit_from_env() {
    DETAIL_LIMIT.store(limit_from_env(), Ordering::Relaxed);
}

pub(crate) fn reset_counts() {
    with_counts_mut(Vec::clear);
}

pub(crate) fn decision(label: &'static str, enabled: bool) -> DetailDecision {
    if !enabled {
        return DetailDecision::Deny;
    }
    let limit = DETAIL_LIMIT.load(Ordering::Relaxed);
    let seen = event_index(label);
    if limit == 0 || seen < limit {
        return DetailDecision::Allow;
    }
    if seen == limit {
        return DetailDecision::LimitReached { limit, seen };
    }
    DetailDecision::Deny
}

fn limit_from_env() -> u64 {
    std::env::var("PEONY_TRACE_DETAIL_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
}

fn with_counts_mut(f: impl FnOnce(&mut Vec<(&'static str, u64)>)) {
    match DETAIL_COUNTS.lock() {
        Ok(mut counts) => f(&mut counts),
        Err(poisoned) => {
            let mut counts = poisoned.into_inner();
            f(&mut counts);
        }
    }
}

fn event_index(label: &'static str) -> u64 {
    let mut out = 0;
    with_counts_mut(|counts| {
        if let Some((_, count)) = counts.iter_mut().find(|(name, _)| *name == label) {
            out = *count;
            *count = count.saturating_add(1);
        } else {
            counts.push((label, 1));
        }
    });
    out
}

#[cfg(test)]
pub(crate) fn set_limit_for_tests(limit: u64) {
    DETAIL_LIMIT.store(limit, Ordering::Relaxed);
}
