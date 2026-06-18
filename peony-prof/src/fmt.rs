pub(crate) fn fmt_ns(ns: u128) -> String {
    if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

pub(crate) fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

pub(crate) fn rate(bytes: u64, items: u64, nanos: u128) -> String {
    if nanos == 0 {
        return String::new();
    }
    let seconds = nanos as f64 / 1_000_000_000.0;
    if bytes > 0 {
        format!("{}/s", human((bytes as f64 / seconds) as u64))
    } else if items > 0 {
        format!("{:.1}/s", items as f64 / seconds)
    } else {
        String::new()
    }
}
