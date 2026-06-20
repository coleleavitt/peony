use crate::trace::snapshot_nodes;
use crate::trace_fields::format_fields;

pub fn trace_tree() {
    let mut nodes = snapshot_nodes();
    if nodes.is_empty() {
        return;
    }
    nodes.sort_by_key(|node| node.enter_seq);
    let show_threads = nodes
        .first()
        .is_some_and(|first| nodes.iter().any(|node| node.thread != first.thread));
    eprintln!("\n── peony --trace (call flow) ──────────────────────────────");
    for node in &nodes {
        let indent = "  ".repeat(node.depth);
        let file = node
            .loc
            .file()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| node.loc.file());
        let thread = if show_threads {
            format!(" [{:?}]", node.thread)
        } else {
            String::new()
        };
        let fields = format_fields(&node.fields);
        let fields = if fields.is_empty() {
            String::new()
        } else {
            format!(" [{fields}]")
        };
        if node.is_event {
            let detail = if node.detail.is_empty() {
                String::new()
            } else {
                format!(": {}", node.detail)
            };
            eprintln!(
                "{indent}• {}{}{}{}  ({}:{})",
                node.label,
                detail,
                fields,
                thread,
                file,
                node.loc.line()
            );
        } else {
            eprintln!(
                "{indent}{} {:>9}{}{}  ({}:{})",
                node.label,
                crate::fmt::fmt_ns(node.nanos),
                fields,
                thread,
                file,
                node.loc.line()
            );
        }
        print_stack(&indent, &node.stack);
    }
    eprintln!("───────────────────────────────────────────────────────────");
}

fn print_stack(indent: &str, stack: &str) {
    if stack.is_empty() {
        return;
    }
    for line in stack.lines().take(32) {
        eprintln!("{indent}    ↳ {line}");
    }
}
