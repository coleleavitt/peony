//! `peony-object` — ELF object file parsing and section/symbol extraction.
//!
//! This crate wraps the [`object`] crate and exposes the subset of ELF that
//! the linker cares about:
//!
//! * Parsed [`InputObject`]s loaded from disk via `mmap`.
//! * Per-object [`InputSection`] and [`InputSymbol`] tables.
//! * Archive (`.rlib` / `.a`) member iteration with byte-level comparison
//!   for incremental diffing (rustc does not set timestamps on archive members).
//!
//! ## Design notes
//!
//! Objects are memory-mapped and parsed in parallel by the caller (via
//! `rayon`).  This crate is intentionally stateless — all parsing state lives
//! in the returned structs; the caller is responsible for lifetime management.
//!
//! Section names that follow the Rust mangled-name convention
//! (`.text._ZN…`, `.rodata._ZN…`) are *stable* across incremental rebuilds
//! and can be used directly as diff keys.  Anonymous sections
//! (`.rodata..L__unnamed_N`) must be matched by their referrer set — that
//! logic lives in `peony-cache`.

// Re-export the index newtypes so downstream crates (and their tests) can name
// the types of `InputSection::index` / `InputSymbol::section` / `InputReloc::symbol`.
pub use object::{SectionIndex, SymbolIndex};

mod archive;
mod arena;
mod eh_frame;
pub mod elf;
mod error;
mod input;
mod model;
mod name;
mod parse;
mod section_parse;
mod shared;
pub use archive::{
    ArchiveMember,
    archive_defines_needed,
    iter_archive_members,
    iter_archive_members_matching,
};
pub use arena::{DataSrc, InputArena, SectionData};
pub use eh_frame::{count_fdes, ends_with_eh_terminator, iter_fdes, scan_eh_frame};
pub use error::{ObjectError, Result};
pub use input::{
    FileKind,
    MappedInput,
    classify_bytes,
    classify_file,
    is_shared_object,
    object_defines_global_start,
};
pub use model::{
    Binding,
    ComdatGroup,
    IndexLookup,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    SectionKind,
};
pub use name::Name;
pub use parse::{parse_bare_parallel, parse_bytes_into, parse_object, parse_owned_member};
pub use shared::{SharedExport, SharedObject, parse_shared_object};
