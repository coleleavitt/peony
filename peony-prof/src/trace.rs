use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::panic::Location;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::ThreadId;
use std::time::Instant;

use crate::trace_detail::DetailDecision;
use crate::trace_fields::TraceField;

static TRACE_ENABLED: AtomicBool = AtomicBool::new(false);
static STACK_ENABLED: AtomicBool = AtomicBool::new(false);
static DETAIL_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);
static TRACE_NODES: Mutex<Vec<TraceNode>> = Mutex::new(Vec::new());

#[derive(Clone)]
pub(crate) struct TraceNode {
    pub(crate) label: &'static str,
    pub(crate) loc: &'static Location<'static>,
    pub(crate) thread: ThreadId,
    pub(crate) depth: usize,
    pub(crate) nanos: u128,
    pub(crate) enter_seq: u64,
    pub(crate) is_event: bool,
    pub(crate) detail: String,
    pub(crate) fields: Vec<TraceField>,
    pub(crate) stack: String,
}

thread_local! {
    static TRACE_DEPTH: RefCell<usize> = const { RefCell::new(0) };
}

pub fn trace_enable() {
    enable_trace(false, false);
}

pub fn trace_stack_enable() {
    enable_trace(true, false);
}

pub fn trace_detail_enable() {
    enable_trace(false, true);
}

pub fn trace_stack_detail_enable() {
    enable_trace(true, true);
}

fn enable_trace(stack: bool, detail: bool) {
    crate::enable();
    TRACE_ENABLED.store(true, Ordering::Relaxed);
    STACK_ENABLED.store(stack, Ordering::Relaxed);
    DETAIL_ENABLED.store(detail, Ordering::Relaxed);
    crate::trace_detail::set_limit_from_env();
    reset();
}

pub(crate) fn reset() {
    TRACE_SEQ.store(0, Ordering::Relaxed);
    with_nodes_mut(Vec::clear);
    crate::trace_detail::reset_counts();
    TRACE_DEPTH.with(|depth| *depth.borrow_mut() = 0);
}

pub struct TraceFrame {
    label: &'static str,
    loc: &'static Location<'static>,
    start: Option<Instant>,
    depth: usize,
    enter_seq: u64,
    fields: Vec<TraceField>,
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
            fields: Vec::new(),
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
        fields: Vec::new(),
        stack: capture_stack_if_enabled(),
        active: true,
    }
}

#[inline]
#[track_caller]
pub fn trace_fields(
    label: &'static str,
    fields: impl IntoIterator<Item = TraceField>,
) -> TraceFrame {
    let mut frame = trace(label);
    if frame.active {
        frame.fields.extend(fields);
    }
    frame
}

impl Drop for TraceFrame {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(start) = self.start else { return };
        let nanos = start.elapsed().as_nanos();
        TRACE_DEPTH.with(|depth| *depth.borrow_mut() = self.depth);
        let node = TraceNode {
            label: self.label,
            loc: self.loc,
            thread: std::thread::current().id(),
            depth: self.depth,
            nanos,
            enter_seq: self.enter_seq,
            is_event: false,
            detail: String::new(),
            fields: std::mem::take(&mut self.fields),
            stack: std::mem::take(&mut self.stack),
        };
        with_nodes_mut(|nodes| nodes.push(node));
    }
}

#[inline]
#[track_caller]
pub fn event(label: &'static str, detail: impl Into<String>) {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    push_event(label, detail.into(), Vec::new());
}

#[inline]
#[track_caller]
pub fn event_fields(label: &'static str, fields: impl IntoIterator<Item = TraceField>) {
    if !TRACE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    push_event(label, String::new(), fields.into_iter().collect());
}

#[inline]
#[track_caller]
pub fn detail_event_fields(label: &'static str, fields: impl IntoIterator<Item = TraceField>) {
    if !detail_event_allowed(label) {
        return;
    }
    push_event(label, String::new(), fields.into_iter().collect());
}

#[inline]
pub fn trace_detail_enabled() -> bool {
    TRACE_ENABLED.load(Ordering::Relaxed) && DETAIL_ENABLED.load(Ordering::Relaxed)
}

#[track_caller]
fn push_event(label: &'static str, detail: String, fields: Vec<TraceField>) {
    let depth = TRACE_DEPTH.with(|value| *value.borrow());
    let seq = TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let loc = Location::caller();
    let node = TraceNode {
        label,
        loc,
        thread: std::thread::current().id(),
        depth,
        nanos: 0,
        enter_seq: seq,
        is_event: true,
        detail,
        fields,
        stack: capture_stack_if_enabled(),
    };
    with_nodes_mut(|nodes| nodes.push(node));
}

fn capture_stack_if_enabled() -> String {
    if STACK_ENABLED.load(Ordering::Relaxed) {
        Backtrace::force_capture().to_string()
    } else {
        String::new()
    }
}

fn detail_event_allowed(label: &'static str) -> bool {
    match crate::trace_detail::decision(label, trace_detail_enabled()) {
        DetailDecision::Allow => true,
        DetailDecision::Deny => false,
        DetailDecision::LimitReached { limit, seen } => {
            event_fields(
                "trace-detail-limit",
                [
                    TraceField::text("label", label),
                    TraceField::count("limit", limit),
                    TraceField::count("seen", seen),
                ],
            );
            false
        }
    }
}

fn with_nodes_mut(f: impl FnOnce(&mut Vec<TraceNode>)) {
    match TRACE_NODES.lock() {
        Ok(mut nodes) => f(&mut nodes),
        Err(poisoned) => {
            let mut nodes = poisoned.into_inner();
            f(&mut nodes);
        }
    }
}

pub(crate) fn snapshot_nodes() -> Vec<TraceNode> {
    match TRACE_NODES.lock() {
        Ok(nodes) => nodes.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[cfg(test)]
pub(crate) fn disable_for_tests() {
    TRACE_ENABLED.store(false, Ordering::Relaxed);
    STACK_ENABLED.store(false, Ordering::Relaxed);
    DETAIL_ENABLED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn set_detail_limit_for_tests(limit: u64) {
    crate::trace_detail::set_limit_for_tests(limit);
}
