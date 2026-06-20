use peony_object::{Binding, InputObject};
use peony_symbols::{SymbolId, SymbolTable};
use rustc_hash::FxHashMap;

use crate::may_need_copy_reloc;

pub fn copy_reloc_symbols(objects: &[InputObject], symbols: &SymbolTable) -> Vec<SymbolId> {
    let mut out = FxHashMap::<SymbolId, Vec<u8>>::default();
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            // A COPY reloc is only needed when the reference lives in a section
            // the loader CANNOT relocate at runtime — i.e. a read-only allocated
            // section (`.text`, `.rodata`). A reference from a WRITABLE section
            // (`.data`, `.data.rel.*`) is resolved by a symbolic dynamic
            // relocation instead (see `collect_symbolic_data_relocs`); making it
            // a COPY would wrongly duplicate the DSO object (e.g. a C++ typeinfo),
            // breaking address-identity comparisons during exception unwinding.
            if sec.flags & peony_object::elf::SHF_WRITE != 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if !may_need_copy_reloc(reloc.r_type) {
                    continue;
                }
                let Some(sym) = obj.symbol_by_index(reloc.symbol.0) else {
                    continue;
                };
                if sym.binding == Binding::Local {
                    continue;
                }
                // Never emit COPY relocs for C++ vtables (`_ZTV*`), VTT
                // (`_ZTT*`), or typeinfo-name (`_ZTS*`) symbols. Vtables are
                // referenced indirectly (GOT/relative) — copying them into the
                // executable would create a duplicate that breaks the program-
                // wide identity those objects rely on.
                //
                // Note: typeinfo objects (`_ZTI*`) are deliberately NOT skipped.
                // When `.text` takes a PC-relative reference to a DSO typeinfo
                // object (`R_X86_64_PC32`), a COPY reloc is the only ABI-correct
                // resolution (it binds the executable and the DSO to one copy),
                // matching what mold/bfd/gold emit. Skipping it would leave the
                // reference resolving to address 0.
                if sym.name.starts_with(b"_ZTV")
                    || sym.name.starts_with(b"_ZTT")
                    || sym.name.starts_with(b"_ZTS")
                {
                    continue;
                }
                let Some(res) = symbols.lookup(&sym.name) else {
                    continue;
                };
                if !res.import {
                    continue;
                }
                // Protected/hidden definitions cannot be preempted by a COPY
                // reloc in the executable, so they are never copy-reloc eligible.
                if res.visibility == peony_object::elf::STV_HIDDEN
                    || res.visibility == peony_object::elf::STV_PROTECTED
                {
                    continue;
                }
                // COPY relocations are for DSO data objects. Avoid turning direct
                // function calls into copy relocations if an assembler emitted PC32.
                if res.st_type == peony_object::elf::STT_FUNC
                    || res.st_type == peony_object::elf::STT_GNU_IFUNC
                    || res.st_type == peony_object::elf::STT_TLS
                {
                    continue;
                }
                if res.size == 0 && res.st_type != peony_object::elf::STT_OBJECT {
                    continue;
                }
                out.entry(res.id).or_insert_with(|| sym.name.to_vec());
            }
        }
    }
    let mut out: Vec<(SymbolId, Vec<u8>)> = out.into_iter().collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.into_iter().map(|(id, _)| id).collect()
}
