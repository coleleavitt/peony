use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

static TRACE_ENABLED: AtomicBool = AtomicBool::new(false);
static STACK_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct TraceNode {
    label: &'static str,
    loc: &'static Location<'static>,
    depth: usize,
    nanos: u128,
    enter_seq: u64,
    is_event: bool,
    detail: String,
    stack: String,
}

thread_local! {
    static TRACE_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    static TRACE_NODES: RefCell<Vec<TraceNode>> = const { RefCell::new(Vec::new()) };
}

pub fn trace_enable() {
    crate::enable();
    TRACE_ENABLED.store(true, Ordering::Relaxed);
    STACK_ENABLED.store(false, Ordering::Relaxed);
    reset();
}

pub fn trace_stack_enable() {
    crate::enable();
    TRACE_ENABLED.store(true, Ordering::Relaxed);
    STACK_ENABLED.store(true, Ordering::Relaxed);
    reset();
}

pub(crate) fn reset() {
    TRACE_SEQ.store(0, Ordering::Relaxed);
    TRACE_NODES.with(|nodes| nodes.borrow_mut().clear());
    TRACE_DEPTH.with(|depth| *depth.borrow_mut() = 0);
}

pub struct TraceFrame {
    label: &'static str,
    loc: &'static Location<'static>,
    start: Option<Instant>,
    depth: usize,
    enter_seq: u64,
    stack: String,
    active: bool,
}

#[inline]
#[track_caller]
pub fn trace(label: &'static str) -> TraceFrame {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return TraceFrame {
            label,
            loc: Location::caller(),
            start: None,
            depth: 0,
            enter_seq: 0,
            stack: String::new(),
            active: false,
        };
    }
    let depth = TRACE_DEPTH.with(|value| {
        let mut value = value.borrow_mut();
        let current = *value;
        *value += 1;
        current
    });
    TraceFrame {
        label,
        loc: Location::caller(),
        start: Some(Instant::now()),
        depth,
        enter_seq: TRACE_SEQ.fetch_add(1, Ordering::Relaxed),
        stack: capture_stack_if_enabled(),
        active: true,
    }
}

impl Drop for TraceFrame {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(start) = self.start else { return };
        let nanos = start.elapsed().as_nanos();
        TRACE_DEPTH.with(|depth| *depth.borrow_mut() = self.depth);
        TRACE_NODES.with(|nodes| {
            nodes.borrow_mut().push(TraceNode {
                label: self.label,
                loc: self.loc,
                depth: self.depth,
                nanos,
                enter_seq: self.enter_seq,
                is_event: false,
                detail: String::new(),
                stack: std::mem::take(&mut self.stack),
            })
        });
    }
}

#[inline]
#[track_caller]
pub fn event(label: &'static str, detail: impl Into<String>) {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let depth = TRACE_DEPTH.with(|value| *value.borrow());
    let seq = TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let loc = Location::caller();
    TRACE_NODES.with(|nodes| {
        nodes.borrow_mut().push(TraceNode {
            label,
            loc,
            depth,
            nanos: 0,
            enter_seq: seq,
            is_event: true,
            detail: detail.into(),
            stack: capture_stack_if_enabled(),
        })
    });
}

pub fn trace_tree() {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut nodes = TRACE_NODES.with(|nodes| nodes.borrow().clone());
    if nodes.is_empty() {
        return;
    }
    nodes.sort_by_key(|node| node.enter_seq);
    eprintln!("\n── peony --trace (call flow) ──────────────────────────────");
    for node in &nodes {
        let indent = "  ".repeat(node.depth);
        let file = node
            .loc
            .file()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| node.loc.file());
        if node.is_event {
            let detail = if node.detail.is_empty() {
                String::new()
            } else {
                format!(": {}", node.detail)
            };
            eprintln!(
                "{indent}• {}{}  ({}:{})",
                node.label,
                detail,
                file,
                node.loc.line()
            );
        } else {
            eprintln!(
                "{indent}{} {:>9}  ({}:{})",
                node.label,
                crate::fmt::fmt_ns(node.nanos),
                file,
                node.loc.line()
            );
        }
        print_stack(&indent, &node.stack);
    }
    eprintln!("───────────────────────────────────────────────────────────");
}

fn capture_stack_if_enabled() -> String {
    if STACK_ENABLED.load(Ordering::Relaxed) {
        Backtrace::force_capture().to_string()
    } else {
        String::new()
    }
}

fn print_stack(indent: &str, stack: &str) {
    if stack.is_empty() {
        return;
    }
    for line in stack.lines().take(32) {
        eprintln!("{indent}    ↳ {line}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    #[test]
    fn trace_builds_nested_caller_callee_tree() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        super::trace_enable();
        {
            let _outer = super::trace("outer");
            {
                let _inner = super::trace("inner");
                let _leaf = super::trace("leaf");
            }
        }
        let nodes = super::TRACE_NODES.with(|nodes| nodes.borrow().clone());
        let by_label = |label: &str| nodes.iter().find(|node| node.label == label).unwrap();
        assert_eq!(by_label("outer").depth, 0);
        assert_eq!(by_label("inner").depth, 1);
        assert_eq!(by_label("leaf").depth, 2);
        assert!(by_label("outer").enter_seq < by_label("inner").enter_seq);
        assert!(by_label("inner").enter_seq < by_label("leaf").enter_seq);
        assert!(by_label("outer").loc.line() > 0);
        assert!(by_label("outer").stack.is_empty());
        super::TRACE_ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_disabled_is_noop() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        super::TRACE_ENABLED.store(false, Ordering::Relaxed);
        super::reset();
        {
            let _frame = super::trace("x");
            super::event("e", "d");
        }
        assert!(super::TRACE_NODES.with(|nodes| nodes.borrow().is_empty()));
    }

    #[test]
    fn events_logged_at_frame_depth_with_detail() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        super::trace_enable();
        {
            let _frame = super::trace("phase");
            super::event("conflict", "sym_foo");
            super::event("excluded", String::from("comdat_bar"));
        }
        let nodes = super::TRACE_NODES.with(|nodes| nodes.borrow().clone());
        let events: Vec<_> = nodes.iter().filter(|node| node.is_event).collect();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|node| node.depth == 1));
        assert!(
            events
                .iter()
                .any(|node| node.label == "conflict" && node.detail == "sym_foo")
        );
        assert!(events.iter().any(|node| node.detail == "comdat_bar"));
        assert!(events.iter().all(|node| node.nanos == 0));
        super::TRACE_ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_stack_mode_captures_backtraces() {
        let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
        super::trace_stack_enable();
        {
            let _frame = super::trace("with-stack");
            super::event("stack-event", "detail");
        }
        let nodes = super::TRACE_NODES.with(|nodes| nodes.borrow().clone());
        assert!(nodes.iter().any(|node| !node.stack.is_empty()));
        super::TRACE_ENABLED.store(false, Ordering::Relaxed);
        super::STACK_ENABLED.store(false, Ordering::Relaxed);
    }
}
