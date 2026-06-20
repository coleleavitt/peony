//! `peony-emit` — Output ELF binary emission (MaskRay's pass 9).
//!
//! Writes a complete, loadable static ELF-64 executable:
//!
//! 1. truncate the output file to [`Layout::file_size`] and `mmap` it RW;
//! 2. write the ELF header and program headers;
//! 3. copy input section bytes and materialise the synthetic sections
//!    (`.got`, `.symtab`, `.strtab`, `.shstrtab`);
//! 4. apply all relocations against the final virtual addresses;
//! 5. write the section-header table.
//!
//! The layout maintains `file_offset == vaddr - base`, so writing each section
//! at its `sh_offset` simultaneously places it at the correct in-memory address.

use std::collections::HashSet;
use std::path::Path;

use peony_layout::Layout;
use peony_object::{InputArena, InputObject};
use peony_symbols::SymbolTable;
use thiserror::Error;

mod build_id;
mod eh_frame;
mod file;
mod headers;
mod input_sections;
mod input_work;
mod sections;

use build_id::finalize_build_id;
use eh_frame::write_eh_frame_hdr;
use file::{chmod_executable, open_output_map};
use headers::{write_headers, write_section_headers};
use sections::{write_section_data_parallel, write_tls_got};

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EmitError {
    #[error("I/O error writing output `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("relocation error: {0}")]
    Reloc(#[from] peony_reloc::RelocError),
    #[error("output file size {size} exceeds platform limits")]
    TooLarge { size: u64 },
}

pub type Result<T> = std::result::Result<T, EmitError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionWriteFilter<'a> {
    All,
    RedOnly(&'a HashSet<String>),
}

impl SectionWriteFilter<'_> {
    pub(crate) fn writes_input_section(self, name: &str) -> bool {
        match self {
            SectionWriteFilter::All => true,
            SectionWriteFilter::RedOnly(red) => red.contains(name),
        }
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct EmitConfig {
    /// Reserved for a future build-id; currently ignored.
    pub build_id: bool,
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Write the linked ELF executable to `output_path`.
///
/// ## TLB-aware overwrite-in-place (QUAD §7, Schimmelpfennig et al. 2024)
///
/// If the output file already exists and its size matches `layout.file_size`,
/// we open it with `O_RDWR` (no truncate) and overwrite in-place. This avoids
/// TLB shootdowns for pages that remain allocated to the same process — the OS
/// can recycle physical pages without invalidating all TLB entries, reducing
/// per-link wall-clock time by up to 28% on I/O-bound workloads.
///
/// ## Parallel section copy + relocation apply (QUAD Theorem 5.1)
///
/// All input sections write to disjoint file ranges (by Theorem 4.1), so they
/// can be copied and relocated in parallel with zero synchronization.
pub fn emit_full(
    output_path: &Path,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    config: &EmitConfig,
) -> Result<()> {
    emit_with_filter(
        output_path,
        arena,
        objects,
        symbols,
        layout,
        config,
        SectionWriteFilter::All,
        false,
    )?;
    Ok(())
}

pub fn emit_partial(
    output_path: &Path,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    config: &EmitConfig,
    red_sections: &HashSet<String>,
) -> Result<bool> {
    emit_with_filter(
        output_path,
        arena,
        objects,
        symbols,
        layout,
        config,
        SectionWriteFilter::RedOnly(red_sections),
        true,
    )
}

fn emit_with_filter(
    output_path: &Path,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    _config: &EmitConfig,
    filter: SectionWriteFilter<'_>,
    require_existing_size: bool,
) -> Result<bool> {
    let io = |e: std::io::Error| EmitError::Io {
        path: output_path.display().to_string(),
        source: e,
    };

    if layout.file_size > usize::MAX as u64 {
        return Err(EmitError::TooLarge {
            size: layout.file_size,
        });
    }

    let file_size = layout.file_size;

    // TLB-aware: check if we can overwrite in-place (avoids truncate+remap shootdowns).
    let existing_size = std::fs::metadata(output_path).ok().map(|m| m.len());
    let can_overwrite = existing_size == Some(file_size);
    if require_existing_size && !can_overwrite {
        return Ok(false);
    }

    // SAFETY: we hold the file open exclusively for the duration of the map.
    let mut mmap = {
        let _t = peony_prof::trace("emit:mmap-open");
        open_output_map(output_path, file_size, can_overwrite).map_err(io)?
    };

    // Write ELF header + program headers (serial, small, must be first).
    {
        let _t = peony_prof::trace("emit:headers");
        write_headers(&mut mmap, layout);
    }

    // Parallel section data copy + relocation apply (QUAD Theorem 5.1).
    // Each output section writes to a disjoint file range — zero-sync parallel.
    // Uses ws-deque's Chase-Lev work-stealing deque for load-balanced dispatch.
    {
        let _t = peony_prof::trace("emit:section-data");
        write_section_data_parallel(
            &mut mmap,
            arena,
            objects,
            symbols,
            layout,
            output_path,
            filter,
        )?;
    }

    // Shared-object TLS GOT static slots (DTPOFF in GD/LDM pair slot1) — written
    // directly by VA→file-offset mapping through the `.got` section.
    {
        let _t = peony_prof::trace("emit:tls-got");
        write_tls_got(&mut mmap, layout);
    }

    // `.eh_frame_hdr` must be built from the RELOCATED `.eh_frame` bytes (the FDE
    // `PC begin` fields are set by R_X86_64_PC32 relocations applied above), so
    // it runs after section data + relocations are written.
    {
        let _t = peony_prof::trace("emit:eh-frame-hdr");
        write_eh_frame_hdr(&mut mmap, layout);
    }

    // Section header table (serial, small, at end of file).
    {
        let _t = peony_prof::trace("emit:section-headers");
        write_section_headers(&mut mmap, layout);
    }

    // `.note.gnu.build-id` descriptor: hash the FINAL image now that every byte
    // (section data, relocations, headers) is written and the descriptor is
    // still zero. Parallel blocked hash over ~4MB of output, not ~18.5MB of
    // scattered input — the former 30%-of-link serial bottleneck.
    {
        let _t = peony_prof::trace("emit:build-id-hash");
        finalize_build_id(&mut mmap, layout);
    }

    drop(mmap);

    chmod_executable(output_path);

    tracing::info!(
        output = %output_path.display(),
        size = file_size,
        entry = format_args!("{:#x}", layout.entry),
        overwrite_in_place = can_overwrite,
        partial = require_existing_size,
        "emitted ELF executable"
    );
    Ok(true)
}

// ── .eh_frame_hdr (built from the relocated .eh_frame) ───────────────────────
