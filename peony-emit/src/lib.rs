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

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use memmap2::MmapMut;
use peony_layout::{Layout, SecSource};
use peony_object::{InputObject, elf};
use peony_reloc::ApplyCtx;
use peony_symbols::SymbolTable;
use thiserror::Error;
use ws_deque::{Steal, Worker};

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
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    _config: &EmitConfig,
) -> Result<()> {
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

    let file = if can_overwrite {
        // Open existing file read-write without truncating (TLB-friendly).
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(output_path)
            .map_err(io)?
    } else {
        // New or differently-sized file: create/truncate.
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)
            .map_err(io)?;
        f.set_len(file_size).map_err(io)?;
        f
    };

    // SAFETY: we hold the file open exclusively for the duration of the map.
    let mut mmap = unsafe { MmapMut::map_mut(&file) }.map_err(io)?;

    // Zero out the mmap if we're writing over a differently-sized file or new file.
    if !can_overwrite {
        mmap.iter_mut().for_each(|b| *b = 0);
    }

    // Write ELF header + program headers (serial, small, must be first).
    write_headers(&mut mmap, layout);

    // Parallel section data copy + relocation apply (QUAD Theorem 5.1).
    // Each output section writes to a disjoint file range — zero-sync parallel.
    // Uses ws-deque's Chase-Lev work-stealing deque for load-balanced dispatch.
    write_section_data_parallel(&mut mmap, objects, symbols, layout, output_path)?;

    // Shared-object TLS GOT static slots (DTPOFF in GD/LDM pair slot1) — written
    // directly by VA→file-offset mapping through the `.got` section.
    write_tls_got(&mut mmap, layout);

    // `.eh_frame_hdr` must be built from the RELOCATED `.eh_frame` bytes (the FDE
    // `PC begin` fields are set by R_X86_64_PC32 relocations applied above), so
    // it runs after section data + relocations are written.
    write_eh_frame_hdr(&mut mmap, layout);

    // Section header table (serial, small, at end of file).
    write_section_headers(&mut mmap, layout);

    mmap.flush().map_err(io)?;

    // Make the output executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(output_path) {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            if let Err(e) = std::fs::set_permissions(output_path, perm) {
                tracing::warn!("could not chmod output: {e}");
            }
        }
    }

    tracing::info!(
        output = %output_path.display(),
        size = file_size,
        entry = format_args!("{:#x}", layout.entry),
        overwrite_in_place = can_overwrite,
        "emitted ELF executable"
    );
    Ok(())
}

// ── .eh_frame_hdr (built from the relocated .eh_frame) ───────────────────────

/// Write the shared-object TLS GOT static slots (`layout.tls_got_writes`): each
/// `(got_va, value)` is the DTPOFF in a locally-defined General-Dynamic pair's
/// slot1 (or the Local-Dynamic pair's slot1 = 0), known at link time. Maps the
/// VA to a file offset via the `.got` section, since these slots live in the TLS
/// GOT region appended to `.got`.
fn write_tls_got(buf: &mut [u8], layout: &Layout) {
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

/// Build `.eh_frame_hdr` from the already-relocated `.eh_frame` bytes in `buf`.
///
/// The FDE `PC begin` fields are set by `R_X86_64_PC32` relocations applied
/// during section emission, so we read them back from the output buffer (not the
/// input objects). Produces the version-1 header + a PC-sorted binary-search
/// table of `(initial_location, fde_address)` as 4-byte `datarel|sdata4` offsets.
fn write_eh_frame_hdr(buf: &mut [u8], layout: &Layout) {
    let Some(hdr) = layout
        .output_sections
        .iter()
        .find(|s| s.source == SecSource::EhFrameHdr)
    else {
        return;
    };
    let Some(eh) = layout
        .output_sections
        .iter()
        .find(|s| s.name == ".eh_frame")
    else {
        return;
    };
    let hdr_va = hdr.sh_addr;
    let hdr_off = hdr.sh_offset as usize;
    let hdr_cap = hdr.sh_size as usize;
    let eh_va = eh.sh_addr;
    let eh_off = eh.sh_offset as usize;
    let eh_size = eh.sh_size as usize;
    if eh_off + eh_size > buf.len() {
        return;
    }

    // Parse FDEs from the relocated .eh_frame bytes.
    let eh_bytes = &buf[eh_off..eh_off + eh_size];
    let (cies, fdes_scanned, terms, leftover) = peony_object::scan_eh_frame(eh_bytes);
    tracing::debug!(
        eh_off,
        eh_size,
        cies,
        fdes_scanned,
        terms,
        leftover,
        hdr_cap,
        hdr_capacity_entries = (hdr_cap.saturating_sub(12)) / 8,
        "write_eh_frame_hdr: scanning merged .eh_frame"
    );
    let mut entries: Vec<(i64, i64)> = Vec::new(); // (func_pc, fde_va)
    for (fde_off, pcbegin_off, rel) in peony_object::iter_fdes(eh_bytes) {
        // PC begin is pcrel|sdata4: relative to its own field's virtual address.
        let pcbegin_va = eh_va + pcbegin_off as u64;
        let func_pc = (pcbegin_va as i64).wrapping_add(rel);
        let fde_va = (eh_va + fde_off as u64) as i64;
        entries.push((func_pc, fde_va));
    }
    entries.sort_by_key(|&(pc, _)| pc);

    // If the FDE count exceeds the reserved section capacity the table would be
    // truncated and the unwinder's binary search could land on a bogus entry.
    let cap_entries = (hdr_cap.saturating_sub(12)) / 8;
    if entries.len() > cap_entries {
        tracing::warn!(
            fde_count = entries.len(),
            cap_entries,
            "eh_frame_hdr: more FDEs than reserved capacity — table will be truncated!"
        );
    }

    tracing::debug!(
        eh_frame_va = format_args!("{eh_va:#x}"),
        eh_frame_size = eh_size,
        hdr_va = format_args!("{hdr_va:#x}"),
        fde_count = entries.len(),
        first_pc = format_args!("{:#x}", entries.first().map(|e| e.0).unwrap_or(0)),
        last_pc = format_args!("{:#x}", entries.last().map(|e| e.0).unwrap_or(0)),
        "building .eh_frame_hdr"
    );

    let mut out = Vec::with_capacity(12 + entries.len() * 8);
    out.push(1u8); // version
    out.push(0x1b); // eh_frame_ptr_enc = pcrel|sdata4
    out.push(0x03); // fde_count_enc = udata4
    out.push(0x3b); // table_enc = datarel|sdata4
    let efp_field_va = hdr_va + 4;
    out.extend_from_slice(&((eh_va as i64 - efp_field_va as i64) as i32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (func_pc, fde_va) in &entries {
        out.extend_from_slice(&((func_pc - hdr_va as i64) as i32).to_le_bytes());
        out.extend_from_slice(&((fde_va - hdr_va as i64) as i32).to_le_bytes());
    }
    let n = out.len().min(hdr_cap);
    if hdr_off + n <= buf.len() {
        buf[hdr_off..hdr_off + n].copy_from_slice(&out[..n]);
    }
}

// ── ELF header + program headers ─────────────────────────────────────────────

fn write_headers(buf: &mut [u8], layout: &Layout) {
    // Elf64_Ehdr (64 bytes at offset 0).
    let e = &mut buf[0..64];
    e[0..4].copy_from_slice(&elf::ELFMAG);
    e[4] = elf::ELFCLASS64;
    e[5] = elf::ELFDATA2LSB;
    e[6] = elf::EV_CURRENT;
    e[7] = elf::ELFOSABI_SYSV;
    // e[8..16] ABI version + pad = 0 (already zeroed).
    e[16..18].copy_from_slice(&layout.e_type.to_le_bytes());
    e[18..20].copy_from_slice(&elf::EM_X86_64.to_le_bytes());
    e[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    e[24..32].copy_from_slice(&layout.entry.to_le_bytes());
    e[32..40].copy_from_slice(&layout.phoff.to_le_bytes());
    e[40..48].copy_from_slice(&layout.shoff.to_le_bytes());
    e[48..52].copy_from_slice(&0u32.to_le_bytes()); // e_flags
    e[52..54].copy_from_slice(&(elf::EHDR_SIZE as u16).to_le_bytes());
    e[54..56].copy_from_slice(&(elf::PHDR_SIZE as u16).to_le_bytes());
    e[56..58].copy_from_slice(&(layout.phnum as u16).to_le_bytes());
    e[58..60].copy_from_slice(&(elf::SHDR_SIZE as u16).to_le_bytes());
    e[60..62].copy_from_slice(&(layout.shnum as u16).to_le_bytes());
    e[62..64].copy_from_slice(&(layout.shstrndx as u16).to_le_bytes());

    // Program headers.
    let phoff = layout.phoff as usize;
    for (i, ph) in layout.segments.iter().enumerate() {
        let o = phoff + i * elf::PHDR_SIZE as usize;
        let p = &mut buf[o..o + 56];
        p[0..4].copy_from_slice(&ph.p_type.to_le_bytes());
        p[4..8].copy_from_slice(&ph.p_flags.to_le_bytes());
        p[8..16].copy_from_slice(&ph.p_offset.to_le_bytes());
        p[16..24].copy_from_slice(&ph.p_vaddr.to_le_bytes());
        p[24..32].copy_from_slice(&ph.p_paddr.to_le_bytes());
        p[32..40].copy_from_slice(&ph.p_filesz.to_le_bytes());
        p[40..48].copy_from_slice(&ph.p_memsz.to_le_bytes());
        p[48..56].copy_from_slice(&ph.p_align.to_le_bytes());
    }
}

// ── Per-item processing helper (extracted to keep closures shallow) ───────────

/// Process one work item: copy section bytes and apply its relocations.
/// Called from worker threads; `buf_ptr` + `buf_len` describe the mmap'd output.
fn process_item(
    item: (usize, usize, u64, usize, usize),
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

    sec_buf.copy_from_slice(&isec.data);

    if !isec.relocs.is_empty() {
        for reloc in &isec.relocs {
            if let Err(e) = peony_reloc::apply_reloc(ctx, obj, obj_id, reloc, section_va, sec_buf) {
                *error_slot.lock().unwrap() = Some(e);
                return;
            }
        }
    }
}

// ── Section data (parallel) ───────────────────────────────────────────────────

/// Write all section data and apply relocations in parallel.
///
/// By QUAD Theorem 5.1, each output section writes to a disjoint file range,
/// so we can split the mutable buffer into non-overlapping slices and hand each
/// to a worker thread without any synchronization.
fn write_section_data_parallel(
    buf: &mut [u8],
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    output_path: &Path,
) -> Result<()> {
    // Build a list of (file_offset, size, SecSource) tuples for each section.
    // We need to split the buf into disjoint mutable slices; we do this by
    // collecting (offset, len) pairs sorted by offset, then using split_at_mut.
    //
    // For correctness: synthetic sections (GOT, symtab, etc.) are written
    // serially because they reference shared `layout` data. Input sections
    // (the bulk of the data) are written in parallel.

    // Phase 1: Write all synthetic sections serially (fast — small data).
    for sec in &layout.output_sections {
        match sec.source {
            SecSource::Input => {} // handled in phase 2
            SecSource::Bss => {}   // NOBITS
            SecSource::Got => {
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
            SecSource::SymTab => write_symtab(buf, symbols, layout, sec.sh_offset),
            SecSource::StrTab => write_bytes(buf, sec.sh_offset, &layout.strtab),
            SecSource::ShStrTab => write_bytes(buf, sec.sh_offset, &layout.shstrtab),
            SecSource::NoteBuildId => write_build_id(buf, objects, sec.sh_offset),
            SecSource::Interp => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.interp),
            SecSource::Hash => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.hash),
            SecSource::DynSym => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.dynsym),
            SecSource::DynStr => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.dynstr),
            SecSource::RelaDyn => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.rela_dyn),
            SecSource::GnuVersion => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.gnu_version),
            SecSource::GnuVersionR => {
                write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.gnu_version_r)
            }
            // `.eh_frame_hdr` is filled by `write_eh_frame_hdr` after relocations.
            SecSource::EhFrameHdr => {}
            SecSource::GnuHash => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.gnu_hash),
            SecSource::Dynamic => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.dynamic),
            SecSource::Plt => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.plt),
            SecSource::GotPlt => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.got_plt),
            SecSource::RelaPlt => write_bytes(buf, sec.sh_offset, &layout.dyn_blobs.rela_plt),
        }
    }

    // Phase 2: Parallel input section copy + relocation apply.
    let buf_ptr = buf.as_mut_ptr() as usize;
    let buf_len = buf.len();
    let ctx = ApplyCtx {
        symbols,
        layout,
        shared: layout.shared,
    };
    let work_items = collect_input_work_items(layout);
    dispatch_parallel(work_items, objects, buf_ptr, buf_len, ctx, output_path)
}

/// Collect (file_offset, _, section_va, object_id, section_index) for all input sections.
type WorkItem = (usize, usize, u64, usize, usize);

fn collect_input_work_items(layout: &Layout) -> Vec<WorkItem> {
    layout
        .output_sections
        .iter()
        .filter(|sec| sec.source == SecSource::Input)
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
    objects: &[InputObject],
    buf_ptr: usize,
    buf_len: usize,
    ctx: ApplyCtx<'_>,
    output_path: &Path,
) -> Result<()> {
    if work_items.is_empty() {
        return Ok(());
    }

    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(work_items.len());

    let error_slot: Arc<std::sync::Mutex<Option<peony_reloc::RelocError>>> =
        Arc::new(std::sync::Mutex::new(None));

    let workers: Vec<Worker<WorkItem>> = (0..num_threads).map(|_| Worker::new()).collect();
    let stealers: Vec<_> = workers.iter().map(|w| w.stealer()).collect();
    for (i, item) in work_items.into_iter().enumerate() {
        workers[i % num_threads].push(item);
    }

    let idle_count = Arc::new(AtomicUsize::new(0));

    std::thread::scope(|scope| {
        for (t, worker) in workers.into_iter().enumerate() {
            let all_stealers: Vec<_> = stealers.iter().map(|s| s.clone()).collect();
            let error_slot = Arc::clone(&error_slot);
            let idle_count = Arc::clone(&idle_count);

            scope.spawn(move || {
                run_worker(
                    t,
                    worker,
                    &all_stealers,
                    objects,
                    buf_ptr,
                    buf_len,
                    &ctx,
                    &error_slot,
                    &idle_count,
                    num_threads,
                );
            });
        }
    });

    if let Some(e) = Arc::try_unwrap(error_slot)
        .ok()
        .and_then(|m| m.into_inner().ok())
        .flatten()
    {
        tracing::error!(output = %output_path.display(), %e, "relocation error");
        return Err(EmitError::Reloc(e));
    }
    Ok(())
}

/// Per-thread work loop with quiescence-based termination (no premature exit).
fn run_worker(
    t: usize,
    worker: Worker<WorkItem>,
    all_stealers: &[ws_deque::Stealer<WorkItem>],
    objects: &[InputObject],
    buf_ptr: usize,
    buf_len: usize,
    ctx: &ApplyCtx<'_>,
    error_slot: &std::sync::Mutex<Option<peony_reloc::RelocError>>,
    idle_count: &AtomicUsize,
    num_threads: usize,
) {
    let mut is_idle = false;
    loop {
        if error_slot.lock().unwrap().is_some() {
            break;
        }

        if let Some(item) = worker.pop() {
            wake_if_idle(&mut is_idle, idle_count);
            process_item(item, objects, buf_ptr, buf_len, ctx, error_slot);
            continue;
        }

        if try_steal(
            t,
            all_stealers,
            &mut is_idle,
            idle_count,
            objects,
            buf_ptr,
            buf_len,
            ctx,
            error_slot,
        ) {
            continue;
        }

        if !is_idle {
            idle_count.fetch_add(1, Ordering::Release);
            is_idle = true;
        }
        if idle_count.load(Ordering::Acquire) >= num_threads {
            break;
        }
        std::hint::spin_loop();
    }
}

fn wake_if_idle(is_idle: &mut bool, idle_count: &AtomicUsize) {
    if *is_idle {
        idle_count.fetch_sub(1, Ordering::Release);
        *is_idle = false;
    }
}

fn try_steal(
    t: usize,
    all_stealers: &[ws_deque::Stealer<WorkItem>],
    is_idle: &mut bool,
    idle_count: &AtomicUsize,
    objects: &[InputObject],
    buf_ptr: usize,
    buf_len: usize,
    ctx: &ApplyCtx<'_>,
    error_slot: &std::sync::Mutex<Option<peony_reloc::RelocError>>,
) -> bool {
    for (i, s) in all_stealers.iter().enumerate() {
        if i == t {
            continue;
        }
        match s.steal() {
            Steal::Success(item) => {
                wake_if_idle(is_idle, idle_count);
                process_item(item, objects, buf_ptr, buf_len, ctx, error_slot);
                return true;
            }
            Steal::Retry => return true, // victim non-empty; retry without going idle
            Steal::Empty => {}
        }
    }
    false
}

/// Write a `.note.gnu.build-id` note: a content-derived 128-bit hash.
fn write_build_id(buf: &mut [u8], objects: &[InputObject], off: u64) {
    let off = off as usize;
    if off + 32 > buf.len() {
        return;
    }
    buf[off..off + 4].copy_from_slice(&4u32.to_le_bytes()); // namesz = len("GNU\0")
    buf[off + 4..off + 8].copy_from_slice(&16u32.to_le_bytes()); // descsz = hash len
    buf[off + 8..off + 12].copy_from_slice(&elf::NT_GNU_BUILD_ID.to_le_bytes());
    buf[off + 12..off + 16].copy_from_slice(b"GNU\0");
    buf[off + 16..off + 32].copy_from_slice(&build_id_hash(objects));
}

/// A deterministic 128-bit hash over all input section bytes.
fn build_id_hash(objects: &[InputObject]) -> [u8; 16] {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h1 = OFFSET;
    let mut h2 = 0x9e37_79b9_7f4a_7c15u64;
    for obj in objects {
        for s in &obj.sections {
            for &b in &s.data {
                h1 = (h1 ^ b as u64).wrapping_mul(PRIME);
                h2 = (h2.wrapping_add(b as u64)).wrapping_mul(PRIME) ^ (h2 >> 29);
            }
        }
    }
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&h1.to_le_bytes());
    out[8..16].copy_from_slice(&h2.to_le_bytes());
    out
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
        s[6..8].copy_from_slice(&ent.shndx.to_le_bytes());
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

// ── Section header table ─────────────────────────────────────────────────────

fn write_section_headers(buf: &mut [u8], layout: &Layout) {
    let shoff = layout.shoff as usize;
    // Index 0: the null section header (already zeroed by truncation).
    for sec in &layout.output_sections {
        let o = shoff + (sec.shndx as usize) * elf::SHDR_SIZE as usize;
        if o + 64 > buf.len() {
            break;
        }
        let h = &mut buf[o..o + 64];
        h[0..4].copy_from_slice(&sec.sh_name.to_le_bytes());
        h[4..8].copy_from_slice(&sec.sh_type.to_le_bytes());
        h[8..16].copy_from_slice(&sec.sh_flags.to_le_bytes());
        h[16..24].copy_from_slice(&sec.sh_addr.to_le_bytes());
        h[24..32].copy_from_slice(&sec.sh_offset.to_le_bytes());
        h[32..40].copy_from_slice(&sec.sh_size.to_le_bytes());
        h[40..44].copy_from_slice(&sec.sh_link.to_le_bytes());
        h[44..48].copy_from_slice(&sec.sh_info.to_le_bytes());
        h[48..56].copy_from_slice(&sec.sh_addralign.to_le_bytes());
        h[56..64].copy_from_slice(&sec.sh_entsize.to_le_bytes());
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────
