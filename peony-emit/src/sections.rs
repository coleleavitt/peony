use std::path::Path;

use peony_layout::{Layout, SecSource};
use peony_object::{InputArena, InputObject, elf};
use peony_reloc::ApplyCtx;
use peony_symbols::SymbolTable;

use crate::build_id::write_build_id;
use crate::{EmitError, Result, SectionWriteFilter};

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

// ── Per-item processing helper (extracted to keep closures shallow) ───────────

/// Process one work item: copy section bytes and apply its relocations.
/// Called from worker threads; `buf_ptr` + `buf_len` describe the mmap'd output.
fn process_item(
    item: (usize, usize, u64, usize, usize),
    arena: &InputArena,
    objects: &[peony_object::InputObject],
    buf_ptr: usize,
    buf_len: usize,
    ctx: &peony_reloc::ApplyCtx<'_>,
    error_slot: &std::sync::Mutex<Option<peony_reloc::RelocError>>,
) {
    let (file_off, _, section_va, obj_id, sec_idx) = item;
    let Some(obj) = objects.get(obj_id) else {
        return;
    };
    let Some(&pos) = obj.section_map.get(&sec_idx) else {
        return;
    };
    let Some(isec) = obj.sections.get(pos) else {
        return;
    };

    let data_len = isec.data.len();
    if data_len == 0 || file_off + data_len > buf_len {
        return;
    }

    // SAFETY: each section has a unique, non-overlapping file range (QUAD Theorem 4.1).
    let sec_buf: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut((buf_ptr + file_off) as *mut u8, data_len) };

    // Zero-copy source: blit straight from the input mmap (via the arena) into
    // the output buffer — no intermediate per-section Vec.
    sec_buf.copy_from_slice(arena.bytes(isec.data));
    peony_prof::count("sections_emitted", 1);
    peony_prof::record_bytes("emit", data_len as u64);

    if !isec.relocs.is_empty() {
        for reloc in &isec.relocs {
            if let Err(e) = peony_reloc::apply_reloc(ctx, obj, obj_id, reloc, section_va, sec_buf) {
                *error_slot.lock().unwrap() = Some(e);
                return;
            }
            peony_prof::count("relocs_applied", 1);
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
                    zero_section(buf, sec.sh_offset, sec.sh_size);
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

    // Phase 2: Parallel input section copy + relocation apply.
    let buf_ptr = buf.as_mut_ptr() as usize;
    let buf_len = buf.len();
    // Integer-indexed symbol resolution for the per-relocation hot path: built
    // once here (≈49k name hashes) to replace ≈216k per-reloc name hashes with
    // array indexing. Borrows objects + symbols, which outlive this scope.
    let sym_index = {
        let _t = peony_prof::trace("emit:sym-index-build");
        peony_reloc::SymIndex::build(objects, symbols)
    };
    let ctx = ApplyCtx {
        symbols,
        layout,
        shared: layout.shared,
        sym_index: Some(&sym_index),
    };
    let work_items = collect_input_work_items(layout, filter);
    peony_prof::record_items("emit", work_items.len() as u64);
    let _t = peony_prof::trace("emit:section-copy-dispatch");
    dispatch_parallel(
        work_items,
        arena,
        objects,
        buf_ptr,
        buf_len,
        ctx,
        output_path,
    )
}

/// Collect (file_offset, _, section_va, object_id, section_index) for all input sections.
type WorkItem = (usize, usize, u64, usize, usize);

fn collect_input_work_items(layout: &Layout, filter: SectionWriteFilter<'_>) -> Vec<WorkItem> {
    layout
        .output_sections
        .iter()
        .filter(|sec| sec.source == SecSource::Input && filter.writes_input_section(&sec.name))
        .flat_map(|sec| {
            sec.contributions.iter().map(move |c| {
                (
                    (sec.sh_offset + c.offset) as usize,
                    0usize,
                    sec.sh_addr + c.offset,
                    c.object_id,
                    c.section_index,
                )
            })
        })
        .collect()
}

/// Dispatch work items across Chase-Lev workers with quiescence-based termination.
///
/// Uses the proper Chase-Lev drain protocol: an atomic idle counter prevents any
/// thread from exiting while a sibling still has stealable work. A thread only
/// exits when `idle_count == num_threads` (all threads simultaneously idle).
fn dispatch_parallel(
    work_items: Vec<WorkItem>,
    arena: &InputArena,
    objects: &[InputObject],
    buf_ptr: usize,
    buf_len: usize,
    ctx: ApplyCtx<'_>,
    output_path: &Path,
) -> Result<()> {
    if work_items.is_empty() {
        return Ok(());
    }

    // Small links (the common `cc`/incremental case) finish faster serially:
    // the work is a handful of section copies, and spinning up a worker scope
    // costs far more in `futex`/`sched_yield`/mutex churn than the copies save.
    // Profiling a hello-world link showed ~74% of syscall time in futex+yield
    // from the parallel scaffolding alone. Stay serial below this threshold.
    // Section-copy work items: only fan out across ws-deque workers when there
    // are enough that the concurrent copy outweighs spawning the worker scope
    // (which otherwise idle-spins on futex/sched_yield — 85% of a small link's
    // syscall time). Tiny links copy a handful of sections faster serially.
    const PARALLEL_THRESHOLD: usize = 2048;
    if work_items.len() < PARALLEL_THRESHOLD {
        peony_prof::event(
            "emit-dispatch",
            format!("serial: {} work items", work_items.len()),
        );
        let error_slot = std::sync::Mutex::new(None);
        let ctx_ref = &ctx;
        for item in work_items {
            process_item(item, arena, objects, buf_ptr, buf_len, ctx_ref, &error_slot);
            if let Some(e) = error_slot.lock().unwrap().take() {
                tracing::error!(output = %output_path.display(), %e, "relocation error");
                return Err(EmitError::Reloc(e));
            }
        }
        return Ok(());
    }

    // Use the link's CONFIGURED worker count (rayon's global pool, which honours
    // `--threads` and the small-link cap), not `available_parallelism()`. emit
    // formerly always spawned up to all cores, so a link configured for fewer
    // workers still spun 24 ws-deque threads here — measured as the bulk of the
    // futex/sched_yield churn on ripgrep (1040 sched_yield at the 8-thread cap).
    // Capping to the pool size keeps emit consistent with the rest of the link.
    let num_threads = rayon::current_num_threads().max(1).min(work_items.len());
    peony_prof::event(
        "emit-dispatch",
        format!(
            "parallel: {} work items, {num_threads} workers",
            work_items.len()
        ),
    );

    let error_slot: std::sync::Mutex<Option<peony_reloc::RelocError>> = std::sync::Mutex::new(None);

    const BATCH_SIZE: usize = 256;
    let batch_count = work_items.len().div_ceil(BATCH_SIZE);
    peony_prof::event(
        "emit-dispatch",
        format!(
            "batched: {} work items, {batch_count} batches, {BATCH_SIZE}/batch",
            work_items.len()
        ),
    );
    let batches: Vec<Vec<WorkItem>> = work_items
        .chunks(BATCH_SIZE)
        .map(<[WorkItem]>::to_vec)
        .collect();

    ws_deque::scheduler::run(num_threads, batches, |batch, _spawner| {
        // Errors are rare (a malformed reloc); record the first and let the rest
        // drain. We do not early-abort the schedule — the surviving sections are
        // still written to disjoint regions, and the error is returned below.
        for item in batch {
            process_item(item, arena, objects, buf_ptr, buf_len, &ctx, &error_slot);
        }
    });

    if let Some(e) = error_slot.into_inner().ok().flatten() {
        tracing::error!(output = %output_path.display(), %e, "relocation error");
        return Err(EmitError::Reloc(e));
    }
    Ok(())
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
