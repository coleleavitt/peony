#[derive(Clone)]
pub struct TraceField {
    name: &'static str,
    value: TraceValue,
}

#[derive(Clone)]
enum TraceValue {
    Text(String),
    Count(u64),
    Signed(i64),
    Bytes(u64),
    Hex(u64),
    ByteRange { start: u64, len: u64 },
    AddrRange { start: u64, len: u64 },
}

impl TraceField {
    pub fn text(name: &'static str, value: impl Into<String>) -> Self {
        Self {
            name,
            value: TraceValue::Text(value.into()),
        }
    }

    pub const fn count(name: &'static str, value: u64) -> Self {
        Self {
            name,
            value: TraceValue::Count(value),
        }
    }

    pub const fn signed(name: &'static str, value: i64) -> Self {
        Self {
            name,
            value: TraceValue::Signed(value),
        }
    }

    pub const fn bytes(name: &'static str, value: u64) -> Self {
        Self {
            name,
            value: TraceValue::Bytes(value),
        }
    }

    pub const fn hex(name: &'static str, value: u64) -> Self {
        Self {
            name,
            value: TraceValue::Hex(value),
        }
    }

    pub const fn addr(name: &'static str, value: u64) -> Self {
        Self {
            name,
            value: TraceValue::Hex(value),
        }
    }

    pub const fn byte_range(name: &'static str, start: u64, len: u64) -> Self {
        Self {
            name,
            value: TraceValue::ByteRange { start, len },
        }
    }

    pub const fn addr_range(name: &'static str, start: u64, len: u64) -> Self {
        Self {
            name,
            value: TraceValue::AddrRange { start, len },
        }
    }
}

pub(crate) fn format_fields(fields: &[TraceField]) -> String {
    fields
        .iter()
        .map(format_field)
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_field(field: &TraceField) -> String {
    format!("{}={}", field.name, format_value(&field.value))
}

fn format_value(value: &TraceValue) -> String {
    match value {
        TraceValue::Text(value) => value.clone(),
        TraceValue::Count(value) => value.to_string(),
        TraceValue::Signed(value) => value.to_string(),
        TraceValue::Bytes(value) => crate::fmt::human(*value),
        TraceValue::Hex(value) => format!("{value:#x}"),
        TraceValue::ByteRange { start, len } => format_range(*start, *len),
        TraceValue::AddrRange { start, len } => format_range(*start, *len),
    }
}

fn format_range(start: u64, len: u64) -> String {
    let end = start.saturating_add(len);
    format!("{start:#x}..{end:#x}/{}", crate::fmt::human(len))
}

#[cfg(test)]
mod tests {
    #[test]
    fn trace_fields_render_counts_bytes_and_ranges() {
        let fields = [
            super::TraceField::count("sections", 7),
            super::TraceField::bytes("size", 4096),
            super::TraceField::byte_range("file", 0x10, 0x20),
            super::TraceField::addr_range("va", 0x400000, 0x10),
        ];

        assert_eq!(
            super::format_fields(&fields),
            "sections=7 size=4.0KB file=0x10..0x30/32B va=0x400000..0x400010/16B"
        );
    }
}
