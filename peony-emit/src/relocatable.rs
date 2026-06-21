//! Native relocatable (`-r`) output — partial linking.
//!
//! Produces a single `ET_REL` object: input sections are concatenated by name,
//! the symbol tables are merged, and relocations are KEPT (not applied) so the
//! result can be linked again later. No address assignment, no PLT/GOT, no
//! dynamic sections, no program headers.
//!
//! The one subtlety is keeping relocations valid across the merge. Instead of
//! per-relocation addend fixups, every DEFINED symbol's `st_value` is shifted by
//! the offset of its input section within the merged output section. A
//! relocation then carries over verbatim (its addend is unchanged), because the
//! offset now lives in the symbol value — including for `STT_SECTION` symbols
//! (whose input value is 0, so their output value becomes exactly that offset).
//!
//! COMDAT groups are not regenerated here; a link whose inputs carry COMDAT
//! groups is handed to GNU `ld` by the driver instead (see `handoff`).

use std::path::Path;

use peony_object::{Binding, InputArena, InputObject, elf};
use peony_symbols::SymbolTable;
use rustc_hash::FxHashMap;

use crate::{EmitError, Result};

const EHDR_SIZE: u64 = 64;
const SHDR_SIZE: u64 = 64;
const SYM_SIZE: u64 = 24;
const RELA_SIZE: u64 = 24;

/// Section types peony regenerates (input copies are dropped and rebuilt).
fn is_regenerated(sh_type: u32) -> bool {
    matches!(
        sh_type,
        elf::SHT_NULL
            | elf::SHT_SYMTAB
            | elf::SHT_STRTAB
            | elf::SHT_RELA
            | 9 /* SHT_REL */
            | 17 /* SHT_GROUP */
            | elf::SHT_SYMTAB_SHNDX
    )
}

/// One merged output section.
struct OutSection {
    name: String,
    sh_type: u32,
    sh_flags: u64,
    sh_addralign: u64,
    size: u64,
    /// (object id, input section) placed at `offset` within this section.
    parts: Vec<Part>,
}

struct Part {
    obj_id: usize,
    sec_pos: usize, // index into obj.sections
    offset: u64,
}

/// A string table builder: deduplicates and returns byte offsets.
#[derive(Default)]
struct StrTab {
    bytes: Vec<u8>,
    seen: FxHashMap<Vec<u8>, u32>,
}

impl StrTab {
    fn new() -> Self {
        StrTab {
            bytes: vec![0],
            seen: FxHashMap::default(),
        }
    }
    fn intern(&mut self, s: &[u8]) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(&off) = self.seen.get(s) {
            return off;
        }
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s);
        self.bytes.push(0);
        self.seen.insert(s.to_vec(), off);
        off
    }
}

struct OutSym {
    st_name: u32,
    st_info: u8,
    st_other: u8,
    st_shndx: u16,
    st_value: u64,
    st_size: u64,
}

fn align_up(v: u64, a: u64) -> u64 {
    if a <= 1 { v } else { (v + a - 1) & !(a - 1) }
}

/// Emit a native `ET_REL` relocatable object. The driver guarantees the inputs
/// carry no COMDAT groups (those are handed to GNU `ld`).
pub fn emit_relocatable(
    output_path: &Path,
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
) -> Result<()> {
    let io = |e: std::io::Error| EmitError::Io {
        path: output_path.display().to_string(),
        source: e,
    };

    // ── Pass 1: group input sections into output sections by name. ───────────
    let mut out_sections: Vec<OutSection> = Vec::new();
    let mut out_index_by_name: FxHashMap<String, usize> = FxHashMap::default();
    // placement[(obj_id, sec_pos)] = (out_section_index, offset within it)
    let mut placement: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();

    for (obj_id, obj) in objects.iter().enumerate() {
        for (sec_pos, sec) in obj.sections.iter().enumerate() {
            if is_regenerated(sec.sh_type) || sec.name.is_empty() {
                continue;
            }
            let name = String::from_utf8_lossy(&sec.name).into_owned();
            let align = sec.align.max(1);
            let idx = *out_index_by_name.entry(name.clone()).or_insert_with(|| {
                out_sections.push(OutSection {
                    name: name.clone(),
                    sh_type: sec.sh_type,
                    sh_flags: sec.flags,
                    sh_addralign: align,
                    size: 0,
                    parts: Vec::new(),
                });
                out_sections.len() - 1
            });
            let out = &mut out_sections[idx];
            let offset = align_up(out.size, align);
            out.size = offset + sec.size;
            out.sh_addralign = out.sh_addralign.max(align);
            out.sh_flags |= sec.flags;
            out.parts.push(Part {
                obj_id,
                sec_pos,
                offset,
            });
            placement.insert((obj_id, sec_pos), (idx, offset));
        }
    }

    // Section-header indices: 0 = null, then the output sections (1..=N), then
    // the synthesized .rela.*, .symtab, .strtab, .shstrtab assigned below.
    let out_shndx = |out_idx: usize| -> u16 { (out_idx + 1) as u16 };

    // ── Pass 2: merge symbols (locals first, then globals). ──────────────────
    let mut syms: Vec<OutSym> = vec![OutSym {
        st_name: 0,
        st_info: 0,
        st_other: 0,
        st_shndx: 0,
        st_value: 0,
        st_size: 0,
    }];
    let mut strtab = StrTab::new();
    // local_map[obj_id] : raw input symbol index -> output symbol index
    let mut local_map: Vec<FxHashMap<u32, u32>> = vec![FxHashMap::default(); objects.len()];

    for (obj_id, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.binding != Binding::Local {
                continue;
            }
            let (shndx, value) = match sym.section {
                _ if sym.st_type == elf::STT_FILE => (elf::SHN_ABS, sym.value),
                Some(si) => {
                    match placement.get(&(obj_id, obj.section_pos(si.0).unwrap_or(usize::MAX))) {
                        Some(&(out_idx, off)) => (out_shndx(out_idx), sym.value + off),
                        None => continue, // section dropped (e.g. a stripped group)
                    }
                }
                None => (elf::SHN_ABS, sym.value),
            };
            let st_name = strtab.intern(&sym.name);
            let out_index = syms.len() as u32;
            syms.push(OutSym {
                st_name,
                st_info: elf::st_info(elf::STB_LOCAL, sym.st_type),
                st_other: sym.visibility,
                st_shndx: shndx,
                st_value: value,
                st_size: sym.size,
            });
            local_map[obj_id].insert(sym.index.0 as u32, out_index);
        }
    }

    let first_global = syms.len() as u32;

    // Globals: one merged entry per resolved name.
    let mut global_index: FxHashMap<Vec<u8>, u32> = FxHashMap::default();
    for (name, res) in symbols.iter() {
        let bind = match res.binding {
            Binding::Weak => elf::STB_WEAK,
            _ => elf::STB_GLOBAL,
        };
        let (shndx, value, size, st_type) = if let Some((sz, align)) = res.common {
            (elf::SHN_COMMON, align, sz, elf::STT_OBJECT)
        } else if let (Some(def), Some(si)) = (res.defined_in, res.section_index) {
            let sec_pos = objects
                .get(def.0 as usize)
                .and_then(|o| o.section_pos(si))
                .unwrap_or(usize::MAX);
            match placement.get(&(def.0 as usize, sec_pos)) {
                Some(&(out_idx, off)) => {
                    (out_shndx(out_idx), res.value + off, res.size, res.st_type)
                }
                None if res.defined_in.is_some() && res.section_index.is_none() => {
                    (elf::SHN_ABS, res.value, res.size, res.st_type)
                }
                None => (elf::SHN_UNDEF, 0, 0, elf::STT_NOTYPE),
            }
        } else if res.defined_in.is_some() {
            (elf::SHN_ABS, res.value, res.size, res.st_type) // absolute
        } else {
            (elf::SHN_UNDEF, 0, 0, elf::STT_NOTYPE) // undefined reference, kept
        };
        let st_name = strtab.intern(name);
        let out_index = syms.len() as u32;
        syms.push(OutSym {
            st_name,
            st_info: elf::st_info(bind, st_type),
            st_other: res.visibility,
            st_shndx: shndx,
            st_value: value,
            st_size: size,
        });
        global_index.insert(name.to_vec(), out_index);
    }

    // Resolve a relocation's input symbol to its merged-output symbol index.
    let out_sym_for = |obj_id: usize, raw: u32| -> u32 {
        if let Some(&i) = local_map[obj_id].get(&raw) {
            return i;
        }
        if let Some(s) = objects[obj_id].symbol_by_index(raw as usize)
            && let Some(&i) = global_index.get(s.name.as_bytes())
        {
            return i;
        }
        0
    };

    // ── Pass 3: build .rela.<name> for output sections that have relocations. ─
    // Each entry: (output-section index, Vec<Elf64_Rela bytes>).
    let mut rela: Vec<(usize, Vec<u8>)> = Vec::new();
    for (out_idx, out) in out_sections.iter().enumerate() {
        let mut bytes: Vec<u8> = Vec::new();
        for part in &out.parts {
            let sec = &objects[part.obj_id].sections[part.sec_pos];
            for r in &sec.relocs {
                let r_offset = part.offset + r.offset;
                let sym = out_sym_for(part.obj_id, r.symbol.0 as u32);
                let r_info = ((sym as u64) << 32) | (r.r_type as u64);
                bytes.extend_from_slice(&r_offset.to_le_bytes());
                bytes.extend_from_slice(&r_info.to_le_bytes());
                bytes.extend_from_slice(&r.addend.to_le_bytes());
            }
        }
        if !bytes.is_empty() {
            rela.push((out_idx, bytes));
        }
    }

    // ── Section header table layout. ─────────────────────────────────────────
    // Order: null, output sections, .rela.*, .symtab, .strtab, .shstrtab.
    let n_out = out_sections.len();
    let rela_base = 1 + n_out; // first .rela section index
    let symtab_shndx = rela_base + rela.len();
    let strtab_shndx = symtab_shndx + 1;
    let shstrtab_shndx = strtab_shndx + 1;
    let shnum = shstrtab_shndx + 1;

    // Section-name string table.
    let mut shstr = StrTab::new();
    let out_name_off: Vec<u32> = out_sections
        .iter()
        .map(|s| shstr.intern(s.name.as_bytes()))
        .collect();
    let rela_name_off: Vec<u32> = rela
        .iter()
        .map(|(out_idx, _)| {
            let n = format!(".rela{}", out_sections[*out_idx].name);
            shstr.intern(n.as_bytes())
        })
        .collect();
    let symtab_name_off = shstr.intern(b".symtab");
    let strtab_name_off = shstr.intern(b".strtab");
    let shstrtab_name_off = shstr.intern(b".shstrtab");

    // Serialize the merged symbol table.
    let mut symtab_bytes: Vec<u8> = Vec::with_capacity(syms.len() * SYM_SIZE as usize);
    for s in &syms {
        symtab_bytes.extend_from_slice(&s.st_name.to_le_bytes());
        symtab_bytes.push(s.st_info);
        symtab_bytes.push(s.st_other);
        symtab_bytes.extend_from_slice(&s.st_shndx.to_le_bytes());
        symtab_bytes.extend_from_slice(&s.st_value.to_le_bytes());
        symtab_bytes.extend_from_slice(&s.st_size.to_le_bytes());
    }

    // ── Assign file offsets (after the ELF header). ──────────────────────────
    let mut file: Vec<u8> = vec![0u8; EHDR_SIZE as usize];

    // Per-section file offset + size, indexed by section-header index.
    let mut sh_offset = vec![0u64; shnum];
    let mut sh_size = vec![0u64; shnum];

    // Output sections (data or NOBITS).
    for (out_idx, out) in out_sections.iter().enumerate() {
        let shndx = 1 + out_idx;
        sh_size[shndx] = out.size;
        if out.sh_type == elf::SHT_NOBITS {
            sh_offset[shndx] = file.len() as u64; // conventional, no bytes
            continue;
        }
        let off = align_up(file.len() as u64, out.sh_addralign.max(1));
        file.resize(off as usize, 0);
        // Materialize the merged bytes (gaps already zero from resize).
        let start = file.len();
        file.resize(start + out.size as usize, 0);
        for part in &out.parts {
            let sec = &objects[part.obj_id].sections[part.sec_pos];
            let data = arena.bytes(sec.data);
            let dst = start + part.offset as usize;
            file[dst..dst + data.len()].copy_from_slice(data);
        }
        sh_offset[shndx] = off;
    }

    // .rela.* sections.
    for (i, (_out_idx, bytes)) in rela.iter().enumerate() {
        let shndx = rela_base + i;
        let off = align_up(file.len() as u64, 8);
        file.resize(off as usize, 0);
        file.extend_from_slice(bytes);
        sh_offset[shndx] = off;
        sh_size[shndx] = bytes.len() as u64;
    }

    // .symtab
    {
        let off = align_up(file.len() as u64, 8);
        file.resize(off as usize, 0);
        file.extend_from_slice(&symtab_bytes);
        sh_offset[symtab_shndx] = off;
        sh_size[symtab_shndx] = symtab_bytes.len() as u64;
    }
    // .strtab
    {
        let off = file.len() as u64;
        file.extend_from_slice(&strtab.bytes);
        sh_offset[strtab_shndx] = off;
        sh_size[strtab_shndx] = strtab.bytes.len() as u64;
    }
    // .shstrtab
    {
        let off = file.len() as u64;
        file.extend_from_slice(&shstr.bytes);
        sh_offset[shstrtab_shndx] = off;
        sh_size[shstrtab_shndx] = shstr.bytes.len() as u64;
    }

    // ── Section header table at the end. ─────────────────────────────────────
    let shoff = align_up(file.len() as u64, 8);
    file.resize(shoff as usize, 0);

    let write_shdr = |file: &mut Vec<u8>,
                      name: u32,
                      sh_type: u32,
                      flags: u64,
                      off: u64,
                      size: u64,
                      link: u32,
                      info: u32,
                      align: u64,
                      entsize: u64| {
        file.extend_from_slice(&name.to_le_bytes());
        file.extend_from_slice(&sh_type.to_le_bytes());
        file.extend_from_slice(&flags.to_le_bytes());
        file.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        file.extend_from_slice(&off.to_le_bytes());
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&link.to_le_bytes());
        file.extend_from_slice(&info.to_le_bytes());
        file.extend_from_slice(&align.to_le_bytes());
        file.extend_from_slice(&entsize.to_le_bytes());
    };

    // 0: null
    write_shdr(&mut file, 0, elf::SHT_NULL, 0, 0, 0, 0, 0, 0, 0);
    // output sections
    for (out_idx, out) in out_sections.iter().enumerate() {
        let shndx = 1 + out_idx;
        write_shdr(
            &mut file,
            out_name_off[out_idx],
            out.sh_type,
            out.sh_flags,
            sh_offset[shndx],
            sh_size[shndx],
            0,
            0,
            out.sh_addralign,
            section_entsize(out.sh_type),
        );
    }
    // .rela.* sections
    for (i, (out_idx, _)) in rela.iter().enumerate() {
        let shndx = rela_base + i;
        write_shdr(
            &mut file,
            rela_name_off[i],
            elf::SHT_RELA,
            0,
            sh_offset[shndx],
            sh_size[shndx],
            symtab_shndx as u32,  // sh_link → .symtab
            (out_idx + 1) as u32, // sh_info → the section it relocates
            8,
            RELA_SIZE,
        );
    }
    // .symtab
    write_shdr(
        &mut file,
        symtab_name_off,
        elf::SHT_SYMTAB,
        0,
        sh_offset[symtab_shndx],
        sh_size[symtab_shndx],
        strtab_shndx as u32, // sh_link → .strtab
        first_global,        // sh_info → first global symbol index
        8,
        SYM_SIZE,
    );
    // .strtab
    write_shdr(
        &mut file,
        strtab_name_off,
        elf::SHT_STRTAB,
        0,
        sh_offset[strtab_shndx],
        sh_size[strtab_shndx],
        0,
        0,
        1,
        0,
    );
    // .shstrtab
    write_shdr(
        &mut file,
        shstrtab_name_off,
        elf::SHT_STRTAB,
        0,
        sh_offset[shstrtab_shndx],
        sh_size[shstrtab_shndx],
        0,
        0,
        1,
        0,
    );

    // ── ELF header. ──────────────────────────────────────────────────────────
    write_ehdr(&mut file, shoff, shnum as u16, shstrtab_shndx as u16);

    std::fs::write(output_path, &file).map_err(io)?;
    crate::file::chmod_executable(output_path);
    tracing::info!(
        output = %output_path.display(),
        sections = out_sections.len(),
        symbols = syms.len(),
        "emitted ET_REL relocatable object"
    );
    Ok(())
}

fn section_entsize(sh_type: u32) -> u64 {
    match sh_type {
        elf::SHT_SYMTAB | elf::SHT_DYNSYM => SYM_SIZE,
        elf::SHT_RELA => RELA_SIZE,
        9 => 16, // SHT_REL
        _ => 0,
    }
}

fn write_ehdr(file: &mut [u8], shoff: u64, shnum: u16, shstrndx: u16) {
    let e = &mut file[..EHDR_SIZE as usize];
    e[0..4].copy_from_slice(&elf::ELFMAG);
    e[4] = elf::ELFCLASS64;
    e[5] = elf::ELFDATA2LSB;
    e[6] = elf::EV_CURRENT;
    // e[7..16] = OS/ABI + padding, left zero.
    e[16..18].copy_from_slice(&elf::ET_REL.to_le_bytes());
    e[18..20].copy_from_slice(&elf::EM_X86_64.to_le_bytes());
    e[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    // e_entry, e_phoff = 0
    e[40..48].copy_from_slice(&shoff.to_le_bytes());
    // e_flags = 0
    e[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes()); // e_ehsize
    e[54..56].copy_from_slice(&0u16.to_le_bytes()); // e_phentsize
    e[56..58].copy_from_slice(&0u16.to_le_bytes()); // e_phnum
    e[58..60].copy_from_slice(&(SHDR_SIZE as u16).to_le_bytes()); // e_shentsize
    e[60..62].copy_from_slice(&shnum.to_le_bytes());
    e[62..64].copy_from_slice(&shstrndx.to_le_bytes());
}
