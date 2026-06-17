//! `peony-reloc` — Relocation scanning and application (x86-64 ELF).
//!
//! Implements MaskRay's pass 6 (scan → which GOT slots are needed) and the
//! relocation-application part of pass 9.
//!
//! ## Symbol address resolution
//!
//! * **Global / weak** symbols are resolved through the global [`SymbolTable`]
//!   (their final VA / GOT address were written back by
//!   `peony_layout::finalize_symbols`).
//! * **Local / section** symbols are not in the global table; their address is
//!   computed directly from the defining object's section placement via
//!   [`Layout::address_of`].
//!
//! ## Static linking note
//!
//! For a fully-resolved static executable a `PLT32` reference to a *defined*
//! symbol is resolved directly to the symbol (identical to `PC32`); no PLT stub
//! is synthesised. GOT-relative references still allocate a GOT slot holding the
//! symbol's absolute address.

use peony_layout::Layout;
use peony_object::{Binding, InputObject, InputReloc};
use peony_symbols::{SymbolId, SymbolTable};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use thiserror::Error;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RelocError {
    #[error("undefined symbol `{name}` referenced in `{object}`")]
    UndefinedSymbol { name: String, object: String },
    #[error(
        "relocation overflow in `{object}` at offset {offset:#x}: value {value} out of range for type {r_type}"
    )]
    Overflow {
        object: String,
        offset: u64,
        value: i64,
        r_type: u32,
    },
}

pub type Result<T> = std::result::Result<T, RelocError>;

// ── x86-64 relocation type constants ─────────────────────────────────────────

pub mod r_x86_64 {
    pub const NONE: u32 = 0;
    pub const R64: u32 = 1;
    pub const PC32: u32 = 2;
    pub const GOT32: u32 = 3;
    pub const PLT32: u32 = 4;
    pub const GOTPCREL: u32 = 9;
    pub const R32: u32 = 10;
    pub const R32S: u32 = 11;
    pub const R16: u32 = 12;
    pub const PC16: u32 = 13;
    pub const R8: u32 = 14;
    pub const PC8: u32 = 15;
    pub const DTPOFF64: u32 = 17;
    pub const TPOFF64: u32 = 18;
    pub const TLSGD: u32 = 19; // General-Dynamic: relaxed to Local-Exec in an exe
    pub const TLSLD: u32 = 20; // Local-Dynamic: relaxed to Local-Exec in an exe
    pub const DTPOFF32: u32 = 21;
    pub const GOTTPOFF: u32 = 22; // Initial-Exec GOT slot with the TP offset
    pub const TPOFF32: u32 = 23;
    pub const PC64: u32 = 24;
    pub const GOTOFF64: u32 = 25;
    pub const GOTPC32: u32 = 26;
    pub const SIZE32: u32 = 32;
    pub const SIZE64: u32 = 33;
    pub const GOTPCRELX: u32 = 41;
    pub const REX_GOTPCRELX: u32 = 42;
}

/// True for the Local-Exec / Local-Dynamic TLS relocations we resolve statically.
fn is_tls(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::TPOFF32
            | r_x86_64::TPOFF64
            | r_x86_64::DTPOFF32
            | r_x86_64::DTPOFF64
            | r_x86_64::TLSGD
            | r_x86_64::TLSLD
            | r_x86_64::GOTTPOFF
    )
}

/// True for relocation types that reference a symbol through the GOT.
fn needs_got(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::GOT32 | r_x86_64::GOTPCREL | r_x86_64::GOTPCRELX | r_x86_64::REX_GOTPCRELX
    )
}

// ── Synthetic slots ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntheticSlot {
    Got(SymbolId),
    Plt(SymbolId),
}

/// Result of the relocation scan: the GOT/PLT slots required, in stable order.
pub struct RelocScanResult {
    pub slots: Vec<SyntheticSlot>,
    pub slot_set: FxHashMap<SyntheticSlot, u64>,
}

impl RelocScanResult {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            slot_set: FxHashMap::default(),
        }
    }

    fn add(&mut self, slot: SyntheticSlot) {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(v) = self.slot_set.entry(slot) {
            v.insert(0);
            self.slots.push(slot);
        }
    }

    /// The symbols needing a GOT slot, in slot order.
    pub fn got_symbols(&self) -> Vec<SymbolId> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::Got(id) => Some(*id),
                SyntheticSlot::Plt(_) => None,
            })
            .collect()
    }

    /// The symbols needing a PLT entry (imported functions called via `@PLT`).
    pub fn plt_symbols(&self) -> Vec<SymbolId> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::Plt(id) => Some(*id),
                SyntheticSlot::Got(_) => None,
            })
            .collect()
    }
}

impl Default for RelocScanResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Assign real [`SymbolId`]s to weak-undefined symbols that are referenced via
/// the GOT, so their slots are tracked and get a recorded address (holding 0).
/// Must run on `&mut symbols` before [`scan_relocations`].
pub fn assign_weak_got_ids(objects: &[InputObject], symbols: &mut SymbolTable) {
    for obj in objects {
        for sec in &obj.sections {
            for reloc in &sec.relocs {
                if !needs_got(reloc.r_type) {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                // Only weak, currently-undefined, non-import symbols need this.
                let is_weak_undef = matches!(
                    symbols.lookup(&sym.name),
                    Some(r) if !r.is_defined() && !r.import && sym.binding == Binding::Weak
                );
                if is_weak_undef {
                    symbols.ensure_id(&sym.name);
                }
            }
        }
    }
}

// ── Scan phase (parallel) ────────────────────────────────────────────────────

/// Scan all relocations to determine the required GOT and PLT slots.
pub fn scan_relocations(objects: &[InputObject], symbols: &SymbolTable) -> RelocScanResult {
    let per_object: Vec<Vec<SyntheticSlot>> = objects
        .par_iter()
        .map(|obj| scan_object(obj, symbols))
        .collect();

    let mut result = RelocScanResult::new();
    for slots in per_object {
        for slot in slots {
            result.add(slot);
        }
    }
    result
}

fn scan_object(obj: &InputObject, symbols: &SymbolTable) -> Vec<SyntheticSlot> {
    let mut slots = Vec::new();
    for sec in &obj.sections {
        for reloc in &sec.relocs {
            let Some(sym) = obj
                .symbols
                .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
            else {
                continue;
            };
            // Slots are keyed by global SymbolId; local refs are handled directly.
            let Some(res) = symbols.lookup(&sym.name) else {
                continue;
            };
            // A GOT reference needs a slot whenever the symbol is defined, an
            // import, OR a weak-undefined reference (whose slot holds 0 so code
            // like `mov gmon@GOTPCREL,%rax; test %rax,%rax` correctly sees null).
            if needs_got(reloc.r_type)
                && (res.is_defined() || res.import || res.binding == Binding::Weak)
            {
                slots.push(SyntheticSlot::Got(res.id));
            } else if reloc.r_type == r_x86_64::PLT32 && res.import && sym.name != b"__tls_get_addr"
            {
                // A direct `call foo@PLT` to an imported function needs a PLT
                // stub — except `__tls_get_addr`, whose GD/LD call sites are
                // relaxed to Local-Exec (the import is dropped entirely).
                slots.push(SyntheticSlot::Plt(res.id));
            }
        }
    }
    slots
}

/// Count how many `R_X86_64_RELATIVE` dynamic relocations a PIE will need,
/// without requiring final addresses. Used to size `.rela.dyn` before layout.
/// Must match the set produced by [`collect_relative`] exactly.
pub fn count_relative(objects: &[InputObject], symbols: &SymbolTable) -> usize {
    let mut n = 0;
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                let counts = if sym.binding == Binding::Local {
                    // Local defined in a kept, allocated section.
                    sym.section.is_some()
                } else {
                    symbols
                        .lookup(&sym.name)
                        .is_some_and(|r| r.is_defined() && !r.import)
                };
                if counts {
                    n += 1;
                }
            }
        }
    }
    n
}

// ── Dynamic-base (RELATIVE) relocation collection ────────────────────────────

/// For a position-independent executable, collect the `R_X86_64_RELATIVE`
/// dynamic relocations: every `R_X86_64_64` site that targets a symbol defined
/// *in this image* (not a dynamic import) holds an absolute address the loader
/// must bias by the load base. We return `(site_vaddr, target_vaddr)` where
/// `target_vaddr = S + A` is the link-time value (which becomes the `r_addend`,
/// since the loader computes `*site = base + addend`).
///
/// Must run after [`peony_layout::finalize_symbols`] so symbol VAs are final.
pub fn collect_relative(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            // Only allocated sections land in the loaded image.
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                // Resolve the target VA exactly as the apply phase will.
                let target = if sym.binding == Binding::Local {
                    match sym.section.and_then(|si| layout.address_of(obj_id, si.0)) {
                        Some(va) => va + sym.value,
                        None => continue, // absolute or dropped — no base bias needed
                    }
                } else {
                    match symbols.lookup(&sym.name) {
                        // Imports are handled by GLOB_DAT, not RELATIVE.
                        Some(r) if r.is_defined() && !r.import => r.virtual_address,
                        _ => continue,
                    }
                };
                let Some(site_va) = layout.address_of(obj_id, sec.index.0) else {
                    continue;
                };
                let site = site_va + reloc.offset;
                let value = (target as i64).wrapping_add(reloc.addend) as u64;
                out.push((site, value));
            }
        }
    }

    // GOT slots holding the address of a locally-defined (non-import) symbol also
    // need an R_X86_64_RELATIVE in a PIE: the slot contains an absolute VA that
    // the loader must bias. (Import GOT slots use GLOB_DAT instead.)
    for (i, id) in layout.got_slots.iter().enumerate() {
        let Some(name) = symbols.name_by_id(*id) else {
            continue;
        };
        let Some(res) = symbols.lookup(name) else {
            continue;
        };
        if res.import || !res.is_defined() {
            continue;
        }
        let site = layout.got_base + (i as u64) * 8;
        out.push((site, res.virtual_address));
    }

    // Deterministic order (by site) for reproducible output + DT_RELACOUNT runs.
    out.sort_unstable();
    out
}

/// Count GOT slots that will need an `R_X86_64_RELATIVE` (defined non-import
/// symbols referenced through the GOT in a PIE). Added to [`count_relative`] by
/// the driver since GOT slots are known before layout.
pub fn count_got_relative(got_syms: &[SymbolId], symbols: &SymbolTable) -> usize {
    got_syms
        .iter()
        .filter(|id| {
            symbols
                .name_by_id(**id)
                .and_then(|n| symbols.lookup(n))
                .is_some_and(|r| !r.import && r.is_defined())
        })
        .count()
}

// ── Apply phase ──────────────────────────────────────────────────────────────

/// Context for applying relocations.
#[derive(Copy, Clone)]
pub struct ApplyCtx<'a> {
    pub symbols: &'a SymbolTable,
    pub layout: &'a Layout,
}

/// Addresses used when computing a relocation value.
struct RelocAddrs {
    s: u64, // symbol VA
    a: i64, // addend
    p: u64, // place (relocation site) VA
    g: u64, // GOT entry VA for the symbol (0 if none)
    l: u64, // PLT stub VA (0 = resolve directly)
    z: u64, // symbol size
    got_base: u64,
    tls: u64,      // symbol's offset within the static TLS block
    tls_size: u64, // total static TLS block size
    offset: usize,
}

/// Apply a single relocation, patching `buf` (the relocated section's bytes).
///
/// `obj_id` is the object's index (used to resolve local-symbol addresses).
/// `section_va` is the virtual address of the section start.
pub fn apply_reloc(
    ctx: &ApplyCtx<'_>,
    obj: &InputObject,
    obj_id: usize,
    reloc: &InputReloc,
    section_va: u64,
    buf: &mut [u8],
) -> Result<()> {
    if reloc.r_type == r_x86_64::NONE {
        return Ok(());
    }
    let Some(sym) = obj
        .symbols
        .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
    else {
        return Ok(());
    };

    // The `call __tls_get_addr@PLT` that follows a GD/LD `lea` is rewritten by
    // the TLSGD/TLSLD relaxation; its PLT32/PC32 relocation must not run, or it
    // would corrupt the relaxed Local-Exec instruction bytes.
    if matches!(reloc.r_type, r_x86_64::PLT32 | r_x86_64::PC32) && sym.name == b"__tls_get_addr" {
        return Ok(());
    }

    // Resolve the symbol's address (S), GOT slot (G), PLT (L), size (Z).
    let (s, g, l, z) = if sym.binding == Binding::Local {
        let s = match sym.section {
            Some(si) => match ctx.layout.address_of(obj_id, si.0) {
                Some(va) => va + sym.value,
                None => return Ok(()), // defined in a dropped section
            },
            None => sym.value, // absolute local
        };
        (s, 0, 0, sym.size)
    } else {
        match ctx.symbols.lookup(&sym.name) {
            Some(r) if r.is_defined() => (r.virtual_address, r.got_address, r.plt_address, r.size),
            // Weak-undefined: address resolves to 0, but a GOT reference still
            // uses the symbol's allocated GOT slot (which holds 0). Passing the
            // real `got_address` lets `mov sym@GOTPCREL,%rax; test %rax,%rax`
            // correctly observe null instead of dereferencing a bogus slot.
            Some(r) if sym.binding == Binding::Weak => (0, r.got_address, 0, 0),
            Some(_) if sym.binding == Binding::Weak => (0, 0, 0, 0),
            _ => {
                return Err(RelocError::UndefinedSymbol {
                    name: String::from_utf8_lossy(&sym.name).into_owned(),
                    object: obj.path.clone(),
                });
            }
        }
    };

    // For TLS relocations, compute the symbol's offset within the TLS block.
    let tls = if is_tls(reloc.r_type) {
        if sym.binding == Binding::Local {
            sym.section
                .and_then(|si| ctx.layout.tls_offset(obj_id, si.0))
                .map(|b| b + sym.value)
        } else {
            ctx.symbols.lookup(&sym.name).and_then(|r| {
                let def = r.defined_in?;
                r.section_index
                    .and_then(|si| ctx.layout.tls_offset(def.0 as usize, si))
                    .map(|b| b + r.value)
            })
        }
        .unwrap_or(0)
    } else {
        0
    };

    let addrs = RelocAddrs {
        s,
        a: reloc.addend,
        p: section_va + reloc.offset,
        g,
        l,
        z,
        got_base: ctx.layout.got_base,
        tls,
        tls_size: ctx.layout.tls_size,
        offset: reloc.offset as usize,
    };

    patch_buf(buf, reloc.r_type, &addrs, &obj.path)
}

fn patch_buf(buf: &mut [u8], r_type: u32, a: &RelocAddrs, object: &str) -> Result<()> {
    use r_x86_64::*;
    let off = a.offset;
    let s = a.s as i64;
    let p = a.p as i64;
    match r_type {
        R64 => write_u64(buf, off, s.wrapping_add(a.a) as u64),
        PC64 => write_u64(buf, off, s.wrapping_add(a.a).wrapping_sub(p) as u64),
        GOTOFF64 => write_u64(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(a.got_base as i64) as u64,
        ),
        SIZE64 => write_u64(buf, off, (a.z as i64).wrapping_add(a.a) as u64),

        R32 => write_u32(buf, off, s.wrapping_add(a.a), r_type, object, off as u64)?,
        R32S => write_i32(buf, off, s.wrapping_add(a.a), r_type, object, off as u64)?,
        PC32 => write_i32(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,
        SIZE32 => write_u32(
            buf,
            off,
            (a.z as i64).wrapping_add(a.a),
            r_type,
            object,
            off as u64,
        )?,
        // PLT32: static link with a defined target resolves directly (== PC32).
        PLT32 => {
            let target = if a.l != 0 { a.l as i64 } else { s };
            write_i32(
                buf,
                off,
                target.wrapping_add(a.a).wrapping_sub(p),
                r_type,
                object,
                off as u64,
            )?
        }
        // GOT-relative: slot VA + A - P.
        GOTPCREL | GOTPCRELX | REX_GOTPCRELX => write_i32(
            buf,
            off,
            (a.g as i64).wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,
        // Offset of the symbol's slot within the GOT, + A.
        GOT32 => write_u32(
            buf,
            off,
            (a.g as i64)
                .wrapping_sub(a.got_base as i64)
                .wrapping_add(a.a),
            r_type,
            object,
            off as u64,
        )?,
        // GOT base relative to the place.
        GOTPC32 => write_i32(
            buf,
            off,
            (a.got_base as i64).wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,

        // Local-Exec TLS: offset from the thread pointer (TP at end of the block).
        TPOFF32 => {
            let v = (a.tls as i64)
                .wrapping_add(a.a)
                .wrapping_sub(a.tls_size as i64);
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TPOFF64 => write_u64(
            buf,
            off,
            (a.tls as i64)
                .wrapping_add(a.a)
                .wrapping_sub(a.tls_size as i64) as u64,
        ),
        // Local-Dynamic TLS: offset from the start of the TLS block.
        DTPOFF32 => write_i32(
            buf,
            off,
            (a.tls as i64).wrapping_add(a.a),
            r_type,
            object,
            off as u64,
        )?,
        DTPOFF64 => write_u64(buf, off, (a.tls as i64).wrapping_add(a.a) as u64),

        // ── TLS access-model relaxation (executable ⇒ Local-Exec) ────────────
        //
        // In an executable the static TLS block is fixed, so General-Dynamic and
        // Local-Dynamic accesses are relaxed to Local-Exec, eliminating the
        // runtime `__tls_get_addr` call. We rewrite the fixed instruction
        // sequence the compiler emits and patch the TP-relative offset.
        TLSGD => {
            // GD sequence (16 bytes), reloc at the lea displacement (off):
            //   66 48 8d 3d <disp32>      lea x@tlsgd(%rip),%rdi   [off-4 .. off]
            //   66 66 48 e8 <pc32>        call __tls_get_addr@plt
            // → Local-Exec (16 bytes):
            //   64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax
            //   48 8d 80 <tpoff32>           lea x@tpoff(%rax),%rax
            let start = off.wrapping_sub(4);
            if start + 16 <= buf.len() {
                let le_off = (a.tls as i64)
                    .wrapping_add(a.a)
                    .wrapping_sub(a.tls_size as i64) as i32;
                buf[start..start + 16].copy_from_slice(&[
                    0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov %fs:0,%rax
                    0x48, 0x8d, 0x80, 0, 0, 0, 0, // lea off(%rax),%rax
                ]);
                buf[start + 12..start + 16].copy_from_slice(&le_off.to_le_bytes());
            }
        }
        TLSLD => {
            // LD sequence (12 bytes), reloc at the lea displacement (off):
            //   48 8d 3d <disp32>     lea x@tlsld(%rip),%rdi   [off-4 .. off]
            //   e8 <pc32>             call __tls_get_addr@plt
            // → Local-Exec base load (no symbol offset; DTPOFF adds the rest):
            //   66 66 66 64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax (padded)
            let start = off.wrapping_sub(4);
            if start + 12 <= buf.len() {
                buf[start..start + 12].copy_from_slice(&[
                    0x66, 0x66, 0x66, // nop padding
                    0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov %fs:0,%rax
                ]);
            }
        }
        // Initial-Exec: GOT slot holds the TP offset; here we keep the GOT
        // mechanism and write the slot value as a Local-Exec offset.
        GOTTPOFF => {
            let v = (a.g as i64).wrapping_add(a.a).wrapping_sub(p);
            write_i32(buf, off, v, r_type, object, off as u64)?
        }

        R16 => write_u16(buf, off, s.wrapping_add(a.a), object, off as u64)?,
        PC16 => write_u16(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            object,
            off as u64,
        )?,
        R8 => write_u8(buf, off, s.wrapping_add(a.a), object, off as u64)?,
        PC8 => write_u8(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            object,
            off as u64,
        )?,

        other => {
            tracing::warn!(r_type = other, %object, "unhandled relocation type — skipping");
        }
    }
    Ok(())
}

// ── Width-checked writers ────────────────────────────────────────────────────

fn write_u64(buf: &mut [u8], off: usize, val: u64) {
    buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i32(
    buf: &mut [u8],
    off: usize,
    val: i64,
    r_type: u32,
    object: &str,
    reloc_off: u64,
) -> Result<()> {
    let v = val as i32;
    if v as i64 != val {
        return Err(overflow(object, reloc_off, val, r_type));
    }
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn write_u32(
    buf: &mut [u8],
    off: usize,
    val: i64,
    r_type: u32,
    object: &str,
    reloc_off: u64,
) -> Result<()> {
    // Accept either zero-extended or sign-extended 32-bit values.
    let u = val as u64;
    if (u as u32) as u64 != u && (val as i32) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_type));
    }
    buf[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
    Ok(())
}

fn write_u16(buf: &mut [u8], off: usize, val: i64, object: &str, reloc_off: u64) -> Result<()> {
    if (val as i16) as i64 != val && (val as u16) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_x86_64::R16));
    }
    buf[off..off + 2].copy_from_slice(&(val as u16).to_le_bytes());
    Ok(())
}

fn write_u8(buf: &mut [u8], off: usize, val: i64, object: &str, reloc_off: u64) -> Result<()> {
    if (val as i8) as i64 != val && (val as u8) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_x86_64::R8));
    }
    buf[off] = val as u8;
    Ok(())
}

fn overflow(object: &str, offset: u64, value: i64, r_type: u32) -> RelocError {
    RelocError::Overflow {
        object: object.to_owned(),
        offset,
        value,
        r_type,
    }
}
