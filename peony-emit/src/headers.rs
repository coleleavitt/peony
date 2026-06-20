use peony_layout::Layout;
use peony_object::elf;

// ── ELF header + program headers ─────────────────────────────────────────────

pub(crate) fn write_headers(buf: &mut [u8], layout: &Layout) {
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
    let e_shnum = if layout.shnum >= elf::SHN_LORESERVE as u64 {
        0
    } else {
        layout.shnum as u16
    };
    let e_shstrndx = if layout.shstrndx >= elf::SHN_LORESERVE as u64 {
        elf::SHN_XINDEX
    } else {
        layout.shstrndx as u16
    };
    e[60..62].copy_from_slice(&e_shnum.to_le_bytes());
    e[62..64].copy_from_slice(&e_shstrndx.to_le_bytes());

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

// ── Section header table ─────────────────────────────────────────────────────

pub(crate) fn write_section_headers(buf: &mut [u8], layout: &Layout) {
    let shoff = layout.shoff as usize;
    // Index 0: the null section header. For oversized section tables, ELF stores
    // the true section count in sh_size and the true shstrndx in sh_link.
    if shoff + 64 <= buf.len() {
        let null = &mut buf[shoff..shoff + 64];
        if layout.shnum >= elf::SHN_LORESERVE as u64 {
            null[32..40].copy_from_slice(&layout.shnum.to_le_bytes());
        }
        if layout.shstrndx >= elf::SHN_LORESERVE as u64 {
            null[40..44].copy_from_slice(&(layout.shstrndx as u32).to_le_bytes());
        }
    }
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
