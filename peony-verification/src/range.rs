use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::SectionRefWitness;

pub type HalfOpenRangeWitness = RangeBounds;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeOwnerWitness {
    OutputSection { name: String },
    ProgramHeader { index: usize, segment_type: u32 },
    InputSection(SectionRefWitness),
    RelocationWrite { relocation_type: u32 },
    IncrementalSection { name: String },
    Synthetic { label: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RangeBounds {
    pub start: u64,
    pub end: u64,
}

impl RangeBounds {
    pub const fn new(start: u64, end: u64) -> Result<Self, WitnessError> {
        if start > end {
            return Err(WitnessError::RangeStartAfterEnd { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn from_start_len(start: u64, len: u64) -> Result<Self, WitnessError> {
        let Some(end) = start.checked_add(len) else {
            return Err(WitnessError::RangeEndOverflow { start, len });
        };
        Self::new(start, end)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeWitness {
    pub owner: RangeOwnerWitness,
    pub file: RangeBounds,
    pub va: RangeBounds,
}

impl RangeWitness {
    pub const fn new(owner: RangeOwnerWitness, file: RangeBounds, va: RangeBounds) -> Self {
        Self { owner, file, va }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmitWorkRangeWitness {
    pub accepted: RangeWitness,
    pub section_copy: RangeBounds,
    pub relocation_footprints: Vec<RangeBounds>,
}

impl EmitWorkRangeWitness {
    pub fn new(
        accepted: RangeWitness,
        section_copy: RangeBounds,
        relocation_footprints: Vec<RangeBounds>,
    ) -> Result<Self, WitnessError> {
        require_contained("section copy", accepted.file, section_copy)?;
        for footprint in &relocation_footprints {
            require_contained("relocation footprint", accepted.file, *footprint)?;
        }
        Ok(Self {
            accepted,
            section_copy,
            relocation_footprints,
        })
    }
}

fn require_contained(
    kind: &'static str,
    accepted: RangeBounds,
    range: RangeBounds,
) -> Result<(), WitnessError> {
    if accepted.contains(range) {
        return Ok(());
    }
    Err(WitnessError::RangeNotContained {
        kind,
        range_start: range.start,
        range_end: range.end,
        accepted_start: accepted.start,
        accepted_end: accepted.end,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WitnessError {
    #[error("range start {start:#x} exceeds end {end:#x}")]
    RangeStartAfterEnd { start: u64, end: u64 },
    #[error("range start {start:#x} plus length {len:#x} overflows")]
    RangeEndOverflow { start: u64, len: u64 },
    #[error(
        "{kind} range [{range_start:#x}, {range_end:#x}) escapes accepted range [{accepted_start:#x}, {accepted_end:#x})"
    )]
    RangeNotContained {
        kind: &'static str,
        range_start: u64,
        range_end: u64,
        accepted_start: u64,
        accepted_end: u64,
    },
    #[error("missing incremental color for section `{section}`")]
    MissingIncrementalColor { section: String },
    #[error("missing previous section record for `{section}`")]
    MissingPreviousSectionRecord { section: String },
    #[error("layout {owner} has invalid alignment {alignment:#x}")]
    LayoutInvalidAlignment { owner: String, alignment: u64 },
    #[error("layout {owner} is not aligned to {alignment:#x}: start {start:#x}")]
    LayoutMisaligned {
        owner: String,
        start: u64,
        alignment: u64,
    },
    #[error("layout {owner} file end {range_end:#x} exceeds file size {file_size:#x}")]
    LayoutFileOutOfBounds {
        owner: String,
        range_end: u64,
        file_size: u64,
    },
    #[error(
        "layout {owner} file offset {file_start:#x} is not congruent with virtual address {virtual_start:#x} at alignment {alignment:#x}"
    )]
    LayoutPageCongruence {
        owner: String,
        file_start: u64,
        virtual_start: u64,
        alignment: u64,
    },
    #[error(
        "non-alloc output section `{owner}` has non-empty virtual range ending at {virtual_end:#x}"
    )]
    LayoutNonAllocVirtualAddress { owner: String, virtual_end: u64 },
    #[error(
        "{kind} ranges overlap: `{first}` ends at {first_end:#x}, `{second}` starts at {second_start:#x}"
    )]
    LayoutRangeOverlap {
        kind: &'static str,
        first: String,
        second: String,
        first_end: u64,
        second_start: u64,
    },
    #[error(
        "layout contribution {section:?} in `{output_section}` declares [{declared_start:#x}, {declared_end:#x}) but offset/size imply [{computed_start:#x}, {computed_end:#x})"
    )]
    LayoutContributionRangeMismatch {
        output_section: String,
        section: SectionRefWitness,
        declared_start: u64,
        declared_end: u64,
        computed_start: u64,
        computed_end: u64,
    },
    #[error(
        "layout contribution {section:?} in `{output_section}` ends at {contribution_end:#x}, beyond section size {section_size:#x}"
    )]
    LayoutContributionOutOfBounds {
        output_section: String,
        section: SectionRefWitness,
        contribution_end: u64,
        section_size: u64,
    },
    #[error("layout contribution {section:?} appears in multiple output sections")]
    LayoutDuplicateContributionOwner { section: SectionRefWitness },
    #[error(
        "layout segment {segment} has file size {file_size:#x} greater than memory size {memory_size:#x}"
    )]
    LayoutSegmentFileLargerThanMemory {
        segment: String,
        file_size: u64,
        memory_size: u64,
    },
    #[error("alloc output section `{section}` is not contained in any PT_LOAD segment")]
    LayoutSectionOutsideLoadSegment { section: String },
    #[error(
        "GC witness model reachability disagrees with Rust live set: model_only={model_only:?}, rust_only={rust_only:?}"
    )]
    GcReachabilityMismatch {
        model_only: Vec<SectionRefWitness>,
        rust_only: Vec<SectionRefWitness>,
    },
}
