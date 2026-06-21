use thiserror::Error;

use crate::WitnessError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IncrementalPreservationError {
    #[error(transparent)]
    Range(#[from] WitnessError),
    #[error("image length {len} does not fit in u64")]
    ImageLengthTooLarge { len: usize },
    #[error(
        "image length mismatch: previous {previous_len:#x}, partial {partial_len:#x}, full {full_len:#x}"
    )]
    ImageLengthMismatch {
        previous_len: u64,
        partial_len: u64,
        full_len: u64,
    },
    #[error(
        "section `{section}` range [{section_start:#x}, {section_end:#x}) exceeds image length {image_len:#x}"
    )]
    SectionOutOfBounds {
        section: String,
        section_start: u64,
        section_end: u64,
        image_len: u64,
    },
    #[error(
        "write `{write_label}` range [{write_start:#x}, {write_end:#x}) exceeds image length {image_len:#x}"
    )]
    WriteOutOfBounds {
        write_label: String,
        write_start: u64,
        write_end: u64,
        image_len: u64,
    },
    #[error(
        "write `{write_label}` [{write_start:#x}, {write_end:#x}) touches green section `{green_section}` [{green_start:#x}, {green_end:#x})"
    )]
    WriteTouchesGreen {
        write_label: String,
        green_section: String,
        write_start: u64,
        write_end: u64,
        green_start: u64,
        green_end: u64,
    },
    #[error(
        "write `{write_label}` [{write_start:#x}, {write_end:#x}) is not covered by red ranges"
    )]
    WriteOutsideRed {
        write_label: String,
        write_start: u64,
        write_end: u64,
    },
    #[error(
        "green section `{section}` byte {offset:#x} changed: previous {previous:#x}, partial {partial:#x}"
    )]
    GreenByteChanged {
        section: String,
        offset: u64,
        previous: u8,
        partial: u8,
    },
    #[error(
        "red section `{section}` byte {offset:#x} differs from full link: partial {partial:#x}, full {full:#x}"
    )]
    RedByteMismatch {
        section: String,
        offset: u64,
        partial: u8,
        full: u8,
    },
    #[error("byte offset {offset:#x} does not fit in usize")]
    ByteOffsetTooLarge { offset: u64 },
}
