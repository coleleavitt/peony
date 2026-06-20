use crate::TraceField;

fn reset_trace_test_state() {
    crate::trace::disable_for_tests();
    crate::trace::reset();
}

#[test]
fn trace_builds_nested_caller_callee_tree() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    crate::trace::trace_enable();
    {
        let _outer = crate::trace::trace("outer");
        {
            let _inner = crate::trace::trace("inner");
            let _leaf = crate::trace::trace("leaf");
        }
    }
    let nodes = crate::trace::snapshot_nodes();
    let by_label = |label: &str| nodes.iter().find(|node| node.label == label).unwrap();
    assert_eq!(by_label("outer").depth, 0);
    assert_eq!(by_label("inner").depth, 1);
    assert_eq!(by_label("leaf").depth, 2);
    assert!(by_label("outer").enter_seq < by_label("inner").enter_seq);
    assert!(by_label("inner").enter_seq < by_label("leaf").enter_seq);
    assert!(by_label("outer").loc.line() > 0);
    assert!(by_label("outer").stack.is_empty());
    reset_trace_test_state();
}

#[test]
fn trace_collects_worker_thread_events() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    crate::trace::trace_enable();
    let handle = std::thread::spawn(|| crate::trace::event("worker", "done"));
    handle.join().expect("worker finished");
    let nodes = crate::trace::snapshot_nodes();
    assert!(nodes.iter().any(|node| node.label == "worker"));
    reset_trace_test_state();
}

#[test]
fn trace_disabled_is_noop() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    reset_trace_test_state();
    {
        let _frame = crate::trace::trace("x");
        crate::trace::event("e", "d");
    }
    assert!(crate::trace::snapshot_nodes().is_empty());
}

#[test]
fn events_logged_at_frame_depth_with_detail() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    crate::trace::trace_enable();
    {
        let _frame = crate::trace::trace("phase");
        crate::trace::event("conflict", "sym_foo");
        crate::trace::event("excluded", String::from("comdat_bar"));
    }
    let nodes = crate::trace::snapshot_nodes();
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
    reset_trace_test_state();
}

#[test]
fn detail_events_are_capped() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    crate::trace::trace_detail_enable();
    crate::trace::set_detail_limit_for_tests(1);
    crate::trace::detail_event_fields("detail", [TraceField::count("i", 0)]);
    crate::trace::detail_event_fields("detail", [TraceField::count("i", 1)]);
    let nodes = crate::trace::snapshot_nodes();
    assert_eq!(
        nodes.iter().filter(|node| node.label == "detail").count(),
        1
    );
    assert!(nodes.iter().any(|node| node.label == "trace-detail-limit"));
    reset_trace_test_state();
}

#[test]
fn trace_stack_mode_captures_backtraces() {
    let _guard = crate::TEST_LOCK.lock().expect("test mutex poisoned");
    crate::trace::trace_stack_enable();
    {
        let _frame = crate::trace::trace("with-stack");
        crate::trace::event("stack-event", "detail");
    }
    let nodes = crate::trace::snapshot_nodes();
    assert!(nodes.iter().any(|node| !node.stack.is_empty()));
    reset_trace_test_state();
}
