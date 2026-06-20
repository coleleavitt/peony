use std::path::Path;

use peony_layout::{Layout, SecSource};
use peony_object::{InputArena, InputObject, elf};
use peony_symbols::SymbolTable;

use crate::build_id::write_build_id;
use crate::input_sections::copy_input_sections;
use crate::{Result, SectionWriteFilter};

/// Write the shared-object TLS GOT static slots (`layout.tls_got_writes`): each
/// `(got_va, value)` is the DTPOFF in a locally-defined General-Dynamic pair's
/// slot1 (or the Local-Dynamic pair's slot1 = 0), known at link time. Maps the
/// VA to a file offset via the `.got` section, since these slots live in the TLS
/// GOT region appended to `.got`.
pub(crate) fn write_tls_got(buf: &mut [u8], layout: &Layout) {
    if layout.tls_got_writes.is_empty() {
        return;
    }
    let Some(got) = layout
        .output_sections
        .iter()
        .find(|s| s.source == SecSource::Got)
    else {
        return;
    };
    let (lo, hi) = (got.sh_addr, got.sh_addr + got.sh_size);
    for &(va, value) in &layout.tls_got_writes {
        if va < lo || va + 8 > hi {
            tracing::warn!(
                va = format_args!("{va:#x}"),
                "TLS GOT static write outside .got — skipping"
            );
            continue;
        }
        let file_off = (got.sh_offset + (va - got.sh_addr)) as usize;
        if file_off + 8 <= buf.len() {
            buf[file_off..file_off + 8].copy_from_slice(&value.to_le_bytes());
            tracing::trace!(
                va = format_args!("{va:#x}"),
                value,
                "TLS GOT static write (DTPOFF)"
            );
        }
    }
}

// ── Section data (parallel) ───────────────────────────────────────────────────

/// Write all section data and apply relocations in parallel.
///
/// By QUAD Theorem 5.1, each output section writes to a disjoint file range,
/// so we can split the mutable buffer into non-overlapping slices and hand each
/// to a worker thread without any synchronization.
pub(crate) fn write_section_data_parallel(
    buf: &mut [u8],
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    output_path: &Path,
    filter: SectionWriteFilter<'_>,
) -> Result<()> {
    // Build a list of (file_offset, size, SecSource) tuples for each section.
    // We need to split the buf into disjoint mutable slices; we do this by
    // collecting (offset, len) pairs sorted by offset, then using split_at_mut.
    //
    // For correctness: synthetic sections (GOT, symtab, etc.) are written
    // serially because they reference shared `layout` data. Input sections
    // (the bulk of the data) are written in parallel.

    // Phase 1: Write all synthetic sections serially (fast — small data,
    // except build-id which hashes all input bytes; framed separately).
    let synth = peony_prof::trace("emit:synthetic-sections");
    for sec in &layout.output_sections {
        match sec.source {
            SecSource::Input => {
                if filter.writes_input_section(&sec.name) {
                    zero_input_gaps(buf, sec.sh_offset, sec.sh_size, &sec.contributions);
                }
            } // handled in phase 2
            SecSource::Bss => {} // NOBITS
            SecSource::Got => {
                zero_section(buf, sec.sh_offset, sec.sh_size);
                for (i, &sym_id) in layout.got_slots.iter().enumerate() {
                    let va = symbols
                        .name_by_id(sym_id)
                        .and_then(|n| symbols.lookup(n))
                        .map(|r| r.virtual_address)
                        .unwrap_or(0);
                    let off = (sec.sh_offset + (i as u64) * 8) as usize;
                    if off + 8 <= buf.len() {
                        buf[off..off + 8].copy_from_slice(&va.to_le_bytes());
                    }
                }
            }
            SecSource::SymTab => {
                zero_section(buf, sec.sh_offset, sec.sh_size);
                write_symtab(buf, symbols, layout, sec.sh_offset);
            }
            SecSource::SymTabShndx => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.symtab_shndx)
            }
            SecSource::StrTab => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.strtab)
            }
            SecSource::ShStrTab => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.shstrtab)
            }
            // Header only; the descriptor is hashed from the final image by
            // finalize_build_id after all bytes are written.
            SecSource::NoteBuildId => {
                zero_section(buf, sec.sh_offset, sec.sh_size);
                write_build_id(buf, sec.sh_offset);
            }
            SecSource::NoteGnuProperty => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.gnu_property_note)
            }
            SecSource::Interp => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.interp)
            }
            SecSource::Hash => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.hash)
            }
            SecSource::DynSym => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.dynsym)
            }
            SecSource::DynStr => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.dynstr)
            }
            SecSource::RelaDyn => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.rela_dyn)
            }
            SecSource::GnuVersion => write_section_bytes(
                buf,
                sec.sh_offset,
                sec.sh_size,
                &layout.dyn_blobs.gnu_version,
            ),
            SecSource::GnuVersionR => write_section_bytes(
                buf,
                sec.sh_offset,
                sec.sh_size,
                &layout.dyn_blobs.gnu_version_r,
            ),
            // `.eh_frame_hdr` is filled by `write_eh_frame_hdr` after relocations.
            SecSource::EhFrameHdr => zero_section(buf, sec.sh_offset, sec.sh_size),
            SecSource::GnuHash => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.gnu_hash)
            }
            SecSource::Dynamic => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.dynamic)
            }
            SecSource::Plt => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.plt)
            }
            SecSource::GotPlt => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.got_plt)
            }
            SecSource::RelaPlt => {
                write_section_bytes(buf, sec.sh_offset, sec.sh_size, &layout.dyn_blobs.rela_plt)
            }
            SecSource::RelaEmit(idx) => {
                if let Some(bytes) = layout.emit_relocs.get(idx) {
                    write_section_bytes(buf, sec.sh_offset, sec.sh_size, bytes);
                }
            }
        }
    }
    drop(synth);

    let buf_ptr = buf.as_mut_ptr() as usize;
    let buf_len = buf.len();
    copy_input_sections(
        arena,
        objects,
        symbols,
        layout,
        filter,
        output_path,
        buf_ptr,
        buf_len,
    )
}

fn write_symtab(buf: &mut [u8], symbols: &SymbolTable, layout: &Layout, base_off: u64) {
    // Index 0 is the null symbol (already zeroed). Entries start at index 1.
    for (i, ent) in layout.symtab.iter().enumerate() {
        let off = (base_off + ((i + 1) as u64) * elf::SYM_SIZE) as usize;
        if off + 24 > buf.len() {
            break;
        }
        let (value, size) = match ent.local {
            Some((v, sz)) => (v, sz), // local: precomputed at layout time
            None => {
                let res = symbols.lookup(&ent.name);
                (
                    res.map(|r| r.virtual_address).unwrap_or(0),
                    res.map(|r| r.size).unwrap_or(0),
                )
            }
        };
        let s = &mut buf[off..off + 24];
        s[0..4].copy_from_slice(&ent.name_off.to_le_bytes());
        s[4] = ent.info;
        s[5] = elf::STV_DEFAULT;
        // `st_shndx` is a 16-bit field. Any value that fits — including the
        // reserved pseudo-indices SHN_ABS (0xfff1), SHN_COMMON (0xfff2) — is
        // written verbatim. SHN_XINDEX (with the true index in `.symtab_shndx`)
        // is used ONLY for a genuine real-section index that does not fit in 16
        // bits (≥ 0x10000 sections). Rewriting an SHN_ABS symbol to SHN_XINDEX
        // forces a bogus extended-index table that BFD/gdb reject ("not in
        // executable format") — the cause of the C++ exe load failure.
        let shndx = if ent.shndx >= 0x1_0000 {
            elf::SHN_XINDEX
        } else {
            ent.shndx as u16
        };
        s[6..8].copy_from_slice(&shndx.to_le_bytes());
        s[8..16].copy_from_slice(&value.to_le_bytes());
        s[16..24].copy_from_slice(&size.to_le_bytes());
    }
}

fn write_bytes(buf: &mut [u8], off: u64, data: &[u8]) {
    let off = off as usize;
    let end = off + data.len();
    if end <= buf.len() {
        buf[off..end].copy_from_slice(data);
    }
}

fn write_section_bytes(buf: &mut [u8], off: u64, size: u64, data: &[u8]) {
    zero_section(buf, off, size);
    write_bytes(buf, off, data);
}

fn zero_section(buf: &mut [u8], off: u64, size: u64) {
    let off = off as usize;
    let size = size as usize;
    if off.checked_add(size).is_some_and(|end| end <= buf.len()) {
        buf[off..off + size].fill(0);
    }
}

fn zero_input_gaps(
    buf: &mut [u8],
    section_offset: u64,
    section_size: u64,
    contributions: &[peony_layout::SectionContribution],
) {
    let section_end = section_offset.saturating_add(section_size);
    let mut cursor = section_offset;
    for contribution in contributions {
        let start = section_offset.saturating_add(contribution.offset);
        if start > cursor {
            zero_section(buf, cursor, start - cursor);
        }
        cursor = cursor.max(start.saturating_add(contribution.size));
    }
    if section_end > cursor {
        zero_section(buf, cursor, section_end - cursor);
    }
}
