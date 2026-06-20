use peony_layout::Layout;
use peony_object::{Binding, InputObject, InputReloc};
use peony_symbols::{SymbolResolution, SymbolTable};

use crate::{RelocError, Result, TlsRef, is_tls, r_x86_64};

pub struct ApplyCtx<'a> {
    pub symbols: &'a SymbolTable,
    pub layout: &'a Layout,
    /// Producing a shared object: keep General-/Local-Dynamic TLS (GOT pairs +
    /// `__tls_get_addr`) instead of relaxing to Local-Exec.
    pub shared: bool,
    /// Optional integer-indexed symbol resolution, replacing the per-relocation
    /// mangled-name HashMap lookup (216k relocs vs 49k symbols ⇒ ~4.4× reuse).
    /// When present, `apply_reloc` resolves a global via
    /// `index.resolution(obj_id, sym_pos)` — an array index, no hashing. Built
    /// once after resolution; `None` falls back to name lookup. Correctness is
    /// shadow-checked against the name path in debug builds.
    pub sym_index: Option<&'a SymIndex<'a>>,
}

/// Integer-indexed symbol resolution: a per-object `sym_pos → &resolution` side
/// table. Built in the driver from immutable objects + the frozen symbol table
/// (Option B: no `InputSymbol` mutation, so no phase-dependent cached state). The
/// per-relocation hot path indexes this instead of hashing the mangled name. See
/// gpt-5.5 council note.
pub struct SymIndex<'a> {
    /// `by_object[obj_id][sym_pos]` = the borrowed resolution for that input
    /// symbol, or `None` if it has no global resolution (local/empty name). We
    /// store the reference DIRECTLY rather than a SymbolId, because undefined
    /// symbols carry the sentinel id `u32::MAX` which is not a valid view index.
    by_object: Vec<Vec<Option<&'a SymbolResolution>>>,
}

impl<'a> SymIndex<'a> {
    /// Build the index from the objects and the frozen symbol table. Hashes each
    /// object symbol's name once (≈49k total) — replacing the ≈216k per-reloc
    /// name hashes with array indexing.
    pub fn build(objects: &[InputObject], symbols: &'a SymbolTable) -> Self {
        // NOTE: a par_iter build was measured SLOWER here (+0.2ms on ripgrep) —
        // the rayon pool is parked by emit time, so wakeup overhead exceeds the
        // speedup on this ~0.8ms memory-bandwidth-bound pass. Kept serial.
        let by_object = objects
            .iter()
            .map(|obj| {
                obj.symbols
                    .iter()
                    .map(|sym| {
                        if sym.binding == Binding::Local || sym.name.is_empty() {
                            None
                        } else {
                            symbols.lookup(&sym.name)
                        }
                    })
                    .collect()
            })
            .collect();
        SymIndex { by_object }
    }

    /// Resolve a global input symbol (by object + its position in that object's
    /// symbol list) to its resolution, by integer indexing only.
    #[inline]
    pub fn resolution(&self, obj_id: usize, sym_pos: usize) -> Option<&'a SymbolResolution> {
        *self.by_object.get(obj_id)?.get(sym_pos)?
    }
}

/// Addresses used when computing a relocation value.
pub(crate) struct RelocAddrs {
    pub(crate) s: u64, // symbol VA
    pub(crate) a: i64, // addend
    pub(crate) p: u64, // place (relocation site) VA
    pub(crate) g: u64, // GOT entry VA for the symbol (0 if none)
    pub(crate) l: u64, // PLT stub VA (0 = resolve directly)
    pub(crate) z: u64, // symbol size
    pub(crate) got_base: u64,
    pub(crate) tls: u64,      // symbol's offset within the static TLS block
    pub(crate) tls_size: u64, // total static TLS block size
    pub(crate) offset: usize,
    pub(crate) shared: bool, // producing a shared object (GD/LD/IE TLS, no LE relax)
    pub(crate) tls_gd: u64,  // GD GOT pair base VA for this symbol (shared); 0 if none
    pub(crate) tls_ie: u64,  // IE GOT slot VA for this symbol (shared); 0 if none
    pub(crate) tls_desc: u64, // TLSDESC GOT pair base VA for this symbol (shared); 0 if none
    pub(crate) tls_ldm: u64, // module LDM GOT pair base VA (shared); 0 if none
    pub(crate) tls_imported: bool,
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
    let Some(sym_pos) = obj.symbol_pos(reloc.symbol.0) else {
        return Ok(());
    };
    let Some(sym) = obj.symbols.get(sym_pos) else {
        return Ok(());
    };

    // In an EXECUTABLE the `call __tls_get_addr@PLT` after a GD/LD `lea` is
    // rewritten by the TLSGD/TLSLD→LE relaxation, so its PLT32/PC32 relocation
    // must not run (it would corrupt the relaxed Local-Exec bytes). In a SHARED
    // object the call is KEPT, so the relocation must run normally (resolve the
    // real PLT stub) — do not skip it there.
    if !ctx.shared
        && matches!(reloc.r_type, r_x86_64::PLT32 | r_x86_64::PC32)
        && sym.name == b"__tls_get_addr"
    {
        return Ok(());
    }

    // Resolve the global symbol's record ONCE (a name-keyed hash lookup) and
    // reuse it for the address/GOT/PLT, the TLS offset, and the TLS-GOT keying
    // below. apply_reloc runs per relocation, and this `sym` is the same across
    // all three uses, so hashing the mangled name once instead of up to 3× is a
    // free, local win (no behaviour change — guarded by the byte-identical
    // determinism tests). Locals never enter the global table.
    let res = if sym.binding == Binding::Local {
        None
    } else if let Some(index) = ctx.sym_index {
        // Integer-indexed resolution (no name hash). Shadow-checked against the
        // name path in debug builds to prove the index is faithful.
        let by_id = index.resolution(obj_id, sym_pos);
        debug_assert!(
            by_id.map(|r| r.id) == ctx.symbols.lookup(&sym.name).map(|r| r.id),
            "sym_index disagrees with name lookup for `{}`",
            String::from_utf8_lossy(&sym.name)
        );
        by_id
    } else {
        ctx.symbols.lookup(&sym.name)
    };

    // Resolve the symbol's address (S), GOT slot (G), PLT (L), size (Z).
    let (s, g, l, z) = if sym.binding == Binding::Local {
        let s = match sym.section {
            Some(si) => match ctx.layout.address_of(obj_id, si.0) {
                Some(va) => va + sym.value,
                None => {
                    // No placement for the target section. Normally this means
                    // the section was GC'd or COMDAT-discarded, so dropping the
                    // reloc is correct. But a section that survives into the
                    // output WITHOUT an `addresses` entry (a past bug for
                    // non-alloc `.debug_*`) silently leaves this field stale and
                    // corrupts the consumer (DWARF). Trace it so the next such
                    // regression is visible instead of mysterious.
                    tracing::debug!(
                        obj_id,
                        section_index = si.0,
                        r_type = reloc.r_type,
                        reloc_offset = reloc.offset,
                        sym = %String::from_utf8_lossy(&sym.name),
                        "reloc skipped: target section has no address (GC'd, \
                         discarded, or unplaced) — field left unrelocated"
                    );
                    return Ok(());
                }
            },
            None => sym.value, // absolute local
        };
        (s, 0, 0, sym.size)
    } else {
        match res {
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
            res.and_then(|r| {
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
    let tls_imported = is_tls(reloc.r_type)
        && sym.binding != Binding::Local
        && res.is_some_and(|r| r.import && r.defined_in.is_none());

    // TLS GOT addresses for this reference, keyed by `TlsRef` exactly as the
    // scan allocated them. IE (GOTTPOFF) slots exist in BOTH exe and shared
    // outputs; GD/LDM pairs only in a shared object.
    // Only TLS relocations consume these GOT-pair addresses (verified: every
    // patch_buf branch that reads `tls_gd`/`tls_ie`/`tls_desc` is a TLS reloc
    // type, all of which `is_tls` matches). For the ~97% of ordinary relocs we
    // skip building the `TlsRef` and the three (mostly-missing) hashmap probes.
    let (tls_gd, tls_ie, tls_desc) = if is_tls(reloc.r_type) {
        let tref = if sym.binding == Binding::Local {
            TlsRef::Local(obj_id, sym_pos)
        } else {
            match res {
                Some(r) => TlsRef::Global(r.id),
                None => TlsRef::Local(obj_id, sym_pos),
            }
        };
        let gd = ctx.layout.tls_gd_addr.get(&tref).copied().unwrap_or(0);
        let ie = ctx.layout.tls_ie_addr.get(&tref).copied().unwrap_or(0);
        let desc = ctx.layout.tls_desc_addr.get(&tref).copied().unwrap_or(0);
        (gd, ie, desc)
    } else {
        (0, 0, 0)
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
        shared: ctx.shared,
        tls_gd,
        tls_ie,
        tls_desc,
        tls_ldm: ctx.layout.tls_ldm_addr.unwrap_or(0),
        tls_imported,
    };

    patch_buf(buf, reloc.r_type, &addrs, &obj.path)
}

pub(crate) fn patch_buf(buf: &mut [u8], r_type: u32, a: &RelocAddrs, object: &str) -> Result<()> {
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
            tracing::trace!(
                tls_block_off = a.tls,
                addend = a.a,
                tls_size = a.tls_size,
                tpoff = v,
                "TPOFF32 (Local-Exec)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TPOFF64 => write_u64(
            buf,
            off,
            (a.tls as i64)
                .wrapping_add(a.a)
                .wrapping_sub(a.tls_size as i64) as u64,
        ),
        // DTPOFF: offset of the symbol within its module's TLS block.
        //
        // In a SHARED object the matching `lea x@tlsld(%rip)` + `call
        // __tls_get_addr` is KEPT, and the helper returns the module's TLS block
        // base, so DTPOFF stays module-relative (`tls + addend`).
        //
        // In an EXECUTABLE peony relaxes the Local-Dynamic sequence to
        // Local-Exec (`mov %fs:0,%rax`), so the base register now holds the
        // thread pointer (the END of the static TLS block). The per-symbol
        // DTPOFF access must therefore become TP-relative (`tls + addend -
        // tls_size`, i.e. negative) to match — exactly like TPOFF. Leaving it
        // module-relative yields a positive `%fs:0 + off` that corrupts the TCB.
        DTPOFF32 => {
            let base = (a.tls as i64).wrapping_add(a.a);
            let v = if a.shared {
                base
            } else {
                base.wrapping_sub(a.tls_size as i64)
            };
            tracing::trace!(
                tls_block_off = a.tls,
                addend = a.a,
                tls_size = a.tls_size,
                shared = a.shared,
                dtpoff = v,
                "DTPOFF32"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        DTPOFF64 => {
            let base = (a.tls as i64).wrapping_add(a.a);
            let v = if a.shared {
                base
            } else {
                base.wrapping_sub(a.tls_size as i64)
            };
            write_u64(buf, off, v as u64)
        }

        // ── TLS access-model relaxation (executable ⇒ Local-Exec) ────────────
        //
        // In an executable the static TLS block is fixed, so General-Dynamic and
        // Local-Dynamic accesses are relaxed to Local-Exec, eliminating the
        // runtime `__tls_get_addr` call. We rewrite the fixed instruction
        // sequence the compiler emits and patch the TP-relative offset.
        // In a shared object the General-Dynamic access is KEPT (the static TLS
        // offset is unknown when dlopen'd): patch the `lea x@tlsgd(%rip),%rdi`
        // displacement to point at the symbol's GD GOT pair (GOTPCREL math) and
        // leave the `call __tls_get_addr@PLT` intact.
        TLSGD if a.shared => {
            let v = (a.tls_gd as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                gd_pair = format_args!("{:#x}", a.tls_gd),
                disp = v,
                "TLSGD (shared, General-Dynamic, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TLSGD if a.tls_imported => {
            // GD→IE relaxation for preemptible TLS in an executable. The target
            // DSO owns the TLS block, so the loader fills a TPOFF64 IE GOT slot.
            let start = off.wrapping_sub(4);
            if start + 16 <= buf.len() {
                let disp = (a.tls_ie as i64)
                    .wrapping_add(a.a)
                    .wrapping_sub(p)
                    .wrapping_sub(8);
                tracing::trace!(
                    off,
                    start,
                    ie_slot = format_args!("{:#x}", a.tls_ie),
                    disp,
                    "TLSGD→IE relaxation"
                );
                if buf[start] != 0x66 || buf[start + 1] != 0x48 || buf[start + 2] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 4]),
                        "TLSGD: unexpected prologue, skipping IE relaxation"
                    );
                } else {
                    buf[start..start + 16].copy_from_slice(&[
                        0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, 0x48, 0x03, 0x05, 0,
                        0, 0, 0,
                    ]);
                    buf[start + 12..start + 16].copy_from_slice(&(disp as i32).to_le_bytes());
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSGD IE relaxation skipped: out of bounds"
                );
            }
        }
        TLSGD => {
            // GD→LE relaxation (x86-64 psABI). The input 16-byte sequence is:
            //   66 48 8d 3d <disp32>      data16 lea x@tlsgd(%rip),%rdi
            //   66 66 48 e8 <pc32>        data16 data16 rex.W call __tls_get_addr
            // The TLSGD reloc points at <disp32> (4 bytes into the lea), so the
            // sequence starts at off-4. Output (16 bytes):
            //   64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax
            //   48 8d 80 <tpoff32>           lea x@tpoff(%rax),%rax
            let start = off.wrapping_sub(4);
            if start + 16 <= buf.len() {
                // R_X86_64_TLSGD is encoded as a PC-relative LEA displacement,
                // so assemblers commonly give it addend -4. The relaxed LE
                // immediate is no longer PC-relative; compensate for that
                // displacement addend just like lld's relaxTlsGdToLe.
                let le_off = (a.tls as i64)
                    .wrapping_add(a.a)
                    .wrapping_sub(a.tls_size as i64)
                    .wrapping_add(4) as i32;
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 16]),
                    tls_block_off = a.tls,
                    tls_size = a.tls_size,
                    le_off,
                    "TLSGD→LE relaxation"
                );
                // Verify the input matches the expected GD prologue before
                // rewriting (guards against a non-canonical sequence).
                if buf[start] != 0x66 || buf[start + 1] != 0x48 || buf[start + 2] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 4]),
                        "TLSGD: unexpected prologue, skipping relaxation"
                    );
                } else {
                    buf[start..start + 16].copy_from_slice(&[
                        0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00,
                        0x00, // mov %fs:0,%rax
                        0x48, 0x8d, 0x80, 0, 0, 0, 0, // lea off(%rax),%rax
                    ]);
                    buf[start + 12..start + 16].copy_from_slice(&le_off.to_le_bytes());
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSGD relaxation skipped: out of bounds"
                );
            }
        }
        // Shared object: Local-Dynamic kept — patch `lea x@tlsld(%rip),%rdi` to
        // the module LDM GOT pair; the per-symbol offset is added by the
        // (static) DTPOFF32 relocations. The `call __tls_get_addr` is kept.
        TLSLD if a.shared => {
            let v = (a.tls_ldm as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ldm_pair = format_args!("{:#x}", a.tls_ldm),
                disp = v,
                "TLSLD (shared, Local-Dynamic, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TLSLD => {
            // LD→LE relaxation. The input sequence is:
            //   48 8d 3d <disp32>     lea x@tlsld(%rip),%rdi   (reloc at off, lea at off-3)
            //   e8 <pc32>             call __tls_get_addr@plt  (at off+4)
            // GNU ld replaces the whole 12-byte span [off-3 .. off+9] with:
            //   66 66 66 64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax (3 data16 prefixes pad)
            let start = off.wrapping_sub(3);
            if start + 12 <= buf.len() {
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 12]),
                    "TLSLD→LE relaxation"
                );
                buf[start..start + 12].copy_from_slice(&[
                    0x66, 0x66, 0x66, // data16 padding
                    0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov %fs:0,%rax
                ]);
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSLD relaxation skipped: out of bounds"
                );
            }
        }
        // Initial-Exec, shared object: reference the dedicated IE GOT slot, which
        // the loader fills via an R_X86_64_TPOFF64 dynamic relocation (the TP
        // offset is unknown until load). `mov x@gottpoff(%rip),%reg` is kept.
        GOTTPOFF if a.shared => {
            let v = (a.tls_ie as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ie_slot = format_args!("{:#x}", a.tls_ie),
                disp = v,
                "GOTTPOFF (shared, Initial-Exec, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        // Initial-Exec, executable: reference the dedicated IE GOT slot (filled
        // statically by `collect_tls_got` with the fixed TP offset). The
        // `mov x@gottpoff(%rip),%reg` access is kept; this patches its
        // displacement to the slot (GOTPCREL math). Using a real slot (not the
        // scalar GOT) is required — a missing slot would resolve to address 0.
        GOTTPOFF => {
            let v = (a.tls_ie as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ie_slot = format_args!("{:#x}", a.tls_ie),
                disp = v,
                "GOTTPOFF (executable, Initial-Exec GOT slot)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        GOTPC32_TLSDESC if a.shared => {
            let v = (a.tls_desc as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                tlsdesc_pair = format_args!("{:#x}", a.tls_desc),
                disp = v,
                "GOTPC32_TLSDESC (shared, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        GOTPC32_TLSDESC if a.tls_imported => {
            // TLSDESC→IE for preemptible TLS in an executable: keep the access as
            // a GOT load, then TLSDESC_CALL turns into a two-byte nop.
            let start = off.wrapping_sub(3);
            if start + 7 <= buf.len() {
                let disp = (a.tls_ie as i64).wrapping_add(a.a).wrapping_sub(p);
                tracing::trace!(
                    off,
                    start,
                    ie_slot = format_args!("{:#x}", a.tls_ie),
                    disp,
                    "TLSDESC→IE relaxation"
                );
                if buf[start] != 0x48 || buf[start + 1] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 3]),
                        "TLSDESC: unexpected prologue, skipping IE relaxation"
                    );
                } else {
                    buf[start + 1] = 0x8b;
                    write_i32(buf, off, disp, r_type, object, off as u64)?;
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSDESC IE relaxation skipped: out of bounds"
                );
            }
        }
        GOTPC32_TLSDESC => {
            // TLSDESC→LE relaxation for executables. The canonical input is:
            //   48 8d 05 <disp32>    lea x@tlsdesc(%rip),%rax
            //   ff 10                call *x@tlscall(%rax)
            // GNU ld rewrites the 9-byte pair to:
            //   48 c7 c0 <tpoff32>   mov $x@tpoff,%rax
            //   66 90                xchg %ax,%ax
            // The reloc points at <disp32>, so the LEA starts at off-3.
            let start = off.wrapping_sub(3);
            if start + 7 <= buf.len() {
                let le_off = (a.tls as i64)
                    .wrapping_add(a.a)
                    .wrapping_add(4)
                    .wrapping_sub(a.tls_size as i64);
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 7]),
                    tls_block_off = a.tls,
                    tls_size = a.tls_size,
                    le_off,
                    "TLSDESC→LE relaxation"
                );
                if buf[start] != 0x48 || buf[start + 1] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 3]),
                        "TLSDESC: unexpected prologue, skipping relaxation"
                    );
                } else {
                    buf[start..start + 7].copy_from_slice(&[0x48, 0xc7, 0xc0, 0, 0, 0, 0]);
                    write_i32(buf, off, le_off, r_type, object, off as u64)?;
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSDESC relaxation skipped: out of bounds"
                );
            }
        }
        TLSDESC_CALL => {
            // Marker relocation for `call *x@TLSCALL(%reg)`. Shared objects keep
            // the call; executables pair this with GOTPC32_TLSDESC→LE and turn
            // the two call bytes into GNU ld's canonical 2-byte NOP.
            if !a.shared {
                if off + 2 <= buf.len() {
                    buf[off..off + 2].copy_from_slice(&[0x66, 0x90]);
                } else {
                    tracing::warn!(
                        off,
                        buf_len = buf.len(),
                        "TLSDESC_CALL relaxation skipped: out of bounds"
                    );
                }
            }
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
