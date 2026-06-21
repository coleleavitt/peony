use peony_layout::{Layout, LayoutConfig, SectionFilter, TlsGotInfo, compute_layout};
use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};
use peony_symbols::{SymbolId, SymbolTable};
use peony_verification::{
    RelocationByteInputs,
    X86_64RelocationExpression,
    model_x86_64_relocation_bytes,
    x86_64_relocation_expression,
};

use crate::apply::{ApplyCtx, RelocAction, RelocAddrs, apply_reloc, resolve_reloc_action_for_test};

struct SectionSpec<'a> {
    index: usize,
    name: &'a [u8],
    kind: SectionKind,
    section_type: u32,
    flags: u64,
    align: u64,
    data: &'a [u8],
}

fn input_section(arena: &mut InputArena, spec: SectionSpec<'_>) -> InputSection {
    InputSection {
        index: SectionIndex(spec.index),
        name: Name::from_slice(spec.name),
        kind: spec.kind,
        sh_type: spec.section_type,
        data: if spec.data.is_empty() {
            SectionData::EMPTY
        } else {
            arena.intern_bytes(spec.data)
        },
        align: spec.align,
        size: u64::try_from(spec.data.len().max(1)).expect("section size fits in u64"),
        flags: spec.flags,
        relocs: Vec::new(),
    }
}

pub(super) fn text_section(arena: &mut InputArena) -> InputSection {
    input_section(
        arena,
        SectionSpec {
            index: 1,
            name: b".text",
            kind: SectionKind::Text,
            section_type: elf::SHT_PROGBITS,
            flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            align: 16,
            data: &[0x90; 32],
        },
    )
}

pub(super) fn data_section(arena: &mut InputArena) -> InputSection {
    input_section(
        arena,
        SectionSpec {
            index: 2,
            name: b".data",
            kind: SectionKind::Data,
            section_type: elf::SHT_PROGBITS,
            flags: elf::SHF_ALLOC | elf::SHF_WRITE,
            align: 8,
            data: &[1, 2, 3, 4],
        },
    )
}

pub(super) fn tdata_section(arena: &mut InputArena) -> InputSection {
    input_section(
        arena,
        SectionSpec {
            index: 2,
            name: b".tdata",
            kind: SectionKind::Tdata,
            section_type: elf::SHT_PROGBITS,
            flags: elf::SHF_ALLOC | elf::SHF_WRITE | elf::SHF_TLS,
            align: 8,
            data: &[9, 10, 11, 12],
        },
    )
}

pub(super) fn symbol(
    index: usize,
    name: &[u8],
    binding: Binding,
    section: Option<usize>,
    value: u64,
    size: u64,
) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(index),
        name: Name::from_slice(name),
        binding,
        is_undefined: section.is_none(),
        is_common: false,
        is_ifunc: false,
        st_type: elf::STT_FUNC,
        visibility: elf::STV_DEFAULT,
        section: section.map(SectionIndex),
        value,
        size,
    }
}

pub(super) fn object(
    path: &str,
    sections: Vec<InputSection>,
    symbols: Vec<InputSymbol>,
) -> InputObject {
    let mut section_map = IndexLookup::default();
    for (pos, section) in sections.iter().enumerate() {
        section_map.insert(section.index.0, pos);
    }
    let mut symbol_map = IndexLookup::default();
    for (pos, symbol) in symbols.iter().enumerate() {
        symbol_map.insert(symbol.index.0, pos);
    }
    InputObject {
        path: path.to_string(),
        sections,
        symbols,
        section_map,
        symbol_map,
        comdat_groups: Vec::new(),
    }
}

pub(super) fn symbol_table_for(objects: &[InputObject]) -> SymbolTable {
    let mut table = SymbolTable::new();
    for object in objects {
        let id = table.add_object(object.path.clone());
        table
            .process_object(id, object)
            .expect("test symbols resolve");
    }
    table
}

pub(super) fn layout_for(
    arena: &InputArena,
    objects: &[InputObject],
    table: &mut SymbolTable,
    got_syms: &[SymbolId],
    plt_syms: &[SymbolId],
    live: SectionFilter<'_>,
    tls: &TlsGotInfo,
) -> Layout {
    let layout = compute_layout(
        arena,
        objects,
        table,
        got_syms,
        plt_syms,
        live,
        None,
        &LayoutConfig::default(),
        tls,
    )
    .expect("layout computes");
    peony_layout::finalize_symbols(table, &layout);
    layout
}

pub(super) fn assert_apply_matches_witness_and_model(
    ctx: &ApplyCtx<'_>,
    obj: &InputObject,
    reloc: &InputReloc,
    section_va: u64,
    original: Vec<u8>,
    expression: X86_64RelocationExpression,
) -> RelocAddrs {
    let action = resolve_reloc_action_for_test(ctx, obj, 0, reloc, section_va)
        .expect("relocation address resolution succeeds");
    let RelocAction::Patch(addrs) = action else {
        panic!("relocation unexpectedly skipped: {action:?}");
    };
    let inputs = model_inputs(&addrs);
    assert_eq!(
        x86_64_relocation_expression(reloc.r_type, &inputs),
        expression
    );
    let expected = model_x86_64_relocation_bytes(reloc.r_type, &inputs, &original)
        .expect("R1a byte model accepts resolved inputs");
    let mut actual = original;

    apply_reloc(ctx, obj, 0, reloc, section_va, &mut actual)
        .expect("apply_reloc patches with the same resolved inputs");

    assert_eq!(actual, expected.produced_bytes);
    addrs
}

fn model_inputs(addrs: &RelocAddrs) -> RelocationByteInputs {
    RelocationByteInputs {
        s: addrs.s,
        a: addrs.a,
        p: addrs.p,
        g: addrs.g,
        l: addrs.l,
        z: addrs.z,
        got_base: addrs.got_base,
        tls: addrs.tls,
        tls_size: addrs.tls_size,
        offset: addrs.offset,
        shared: addrs.shared,
        tls_gd: addrs.tls_gd,
        tls_ie: addrs.tls_ie,
        tls_desc: addrs.tls_desc,
        tls_ldm: addrs.tls_ldm,
        tls_imported: addrs.tls_imported,
    }
}
