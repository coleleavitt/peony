#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EmitWriteReport {
    ranges: Vec<EmitWriteRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitWriteRange {
    label: String,
    file_offset: u64,
    len: u64,
}

impl EmitWriteReport {
    pub fn ranges(&self) -> &[EmitWriteRange] {
        &self.ranges
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub(crate) fn push(&mut self, range: EmitWriteRange) {
        if range.len != 0 {
            self.ranges.push(range);
        }
    }
}

impl EmitWriteRange {
    pub(crate) fn new(label: String, file_offset: u64, len: u64) -> Self {
        Self {
            label,
            file_offset,
            len,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn file_offset(&self) -> u64 {
        self.file_offset
    }

    pub const fn len(&self) -> u64 {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}
