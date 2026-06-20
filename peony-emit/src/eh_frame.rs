use peony_layout::{Layout, SecSource};

/// Build `.eh_frame_hdr` from the already-relocated `.eh_frame` bytes in `buf`.
///
/// The FDE `PC begin` fields are set by `R_X86_64_PC32` relocations applied
/// during section emission, so we read them back from the output buffer (not the
/// input objects). Produces the version-1 header + a PC-sorted binary-search
/// table of `(initial_location, fde_address)` as 4-byte `datarel|sdata4` offsets.
pub(crate) fn write_eh_frame_hdr(buf: &mut [u8], layout: &Layout) {
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
