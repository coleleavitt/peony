use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    Name,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};

const SHT_LLVM_ADDRSIG: u32 = 0x6fff_4c03;

pub(super) fn safe_fold_objects(
    arena: &mut InputArena,
    include_addrsig: bool,
    address_relocs: Vec<InputReloc>,
) -> Vec<InputObject> {
    let mut left_sections = vec![text_section(arena, b".text.f", Vec::new())];
    if !address_relocs.is_empty() {
        left_sections.push(data_ptr_section(arena, address_relocs));
    }
    let right_sections = vec![text_section(arena, b".text.g", Vec::new())];
    vec![
        object(
            arena,
            ObjectInput {
                path: "left.o",
                sections: left_sections,
                symbols: vec![local_hidden_func(b"f")],
                addrsig: addrsig_from_bool(include_addrsig),
            },
        ),
        object(
            arena,
            ObjectInput {
                path: "right.o",
                sections: right_sections,
                symbols: vec![local_hidden_func(b"g")],
                addrsig: addrsig_from_bool(include_addrsig),
            },
        ),
    ]
}

pub(super) fn fold_objects_with_text_relocs(
    arena: &mut InputArena,
    reloc: InputReloc,
) -> Vec<InputObject> {
    let left_sections = vec![text_section(arena, b".text.f", vec![reloc.clone()])];
    let right_sections = vec![text_section(arena, b".text.g", vec![reloc])];
    vec![
        object(
            arena,
            ObjectInput {
                path: "left.o",
                sections: left_sections,
                symbols: vec![local_hidden_func(b"f")],
                addrsig: AddrsigFixture::Empty,
            },
        ),
        object(
            arena,
            ObjectInput {
                path: "right.o",
                sections: right_sections,
                symbols: vec![local_hidden_func(b"g")],
                addrsig: AddrsigFixture::Empty,
            },
        ),
    ]
}

pub(super) fn fold_objects_with_raw_addrsig(
    arena: &mut InputArena,
    data: &[u8],
) -> Vec<InputObject> {
    let left_sections = vec![text_section(arena, b".text.f", Vec::new())];
    let right_sections = vec![text_section(arena, b".text.g", Vec::new())];
    vec![
        object(
            arena,
            ObjectInput {
                path: "left.o",
                sections: left_sections,
                symbols: vec![local_hidden_func(b"f")],
                addrsig: AddrsigFixture::Raw(data),
            },
        ),
        object(
            arena,
            ObjectInput {
                path: "right.o",
                sections: right_sections,
                symbols: vec![local_hidden_func(b"g")],
                addrsig: AddrsigFixture::Raw(data),
            },
        ),
    ]
}

struct ObjectInput<'a> {
    path: &'static str,
    sections: Vec<InputSection>,
    symbols: Vec<InputSymbol>,
    addrsig: AddrsigFixture<'a>,
}

fn object(arena: &mut InputArena, mut input: ObjectInput<'_>) -> InputObject {
    if let Some(data) = input.addrsig.data() {
        let next_index = next_section_index(&input.sections);
        input
            .sections
            .push(addrsig_section(arena, next_index, data));
    }
    let mut section_map = IndexLookup::default();
    for (pos, section) in input.sections.iter().enumerate() {
        section_map.insert(section.index.0, pos);
    }
    let mut symbol_map = IndexLookup::default();
    for (pos, symbol) in input.symbols.iter().enumerate() {
        symbol_map.insert(symbol.index.0, pos);
    }
    InputObject {
        path: input.path.to_string(),
        sections: input.sections,
        symbols: input.symbols,
        section_map,
        symbol_map,
        comdat_groups: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum AddrsigFixture<'a> {
    Missing,
    Empty,
    Raw(&'a [u8]),
}

impl<'a> AddrsigFixture<'a> {
    const fn data(self) -> Option<&'a [u8]> {
        match self {
            Self::Missing => None,
            Self::Empty => Some(&[]),
            Self::Raw(data) => Some(data),
        }
    }
}

const fn addrsig_from_bool(include_addrsig: bool) -> AddrsigFixture<'static> {
    if include_addrsig {
        AddrsigFixture::Empty
    } else {
        AddrsigFixture::Missing
    }
}

fn next_section_index(sections: &[InputSection]) -> usize {
    sections
        .iter()
        .map(|section| section.index.0)
        .max()
        .unwrap_or(0)
        + 1
}

fn text_section(arena: &mut InputArena, name: &[u8], relocs: Vec<InputReloc>) -> InputSection {
    InputSection {
        index: SectionIndex(1),
        name: Name::from_slice(name),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: arena.intern_bytes(&[0xc3, 0x90]),
        align: 1,
        size: 2,
        flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        relocs,
    }
}

fn data_ptr_section(arena: &mut InputArena, relocs: Vec<InputReloc>) -> InputSection {
    InputSection {
        index: SectionIndex(2),
        name: Name::from_slice(b".data.ptr"),
        kind: SectionKind::Data,
        sh_type: elf::SHT_PROGBITS,
        data: arena.intern_bytes(&[0; 8]),
        align: 1,
        size: 8,
        flags: elf::SHF_ALLOC | elf::SHF_WRITE,
        relocs,
    }
}

fn addrsig_section(arena: &mut InputArena, index: usize, data: &[u8]) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".llvm_addrsig"),
        kind: SectionKind::Other,
        sh_type: SHT_LLVM_ADDRSIG,
        data: arena.intern_bytes(data),
        align: 1,
        size: u64::try_from(data.len()).expect("test addrsig section size fits u64"),
        flags: 0,
        relocs: Vec::new(),
    }
}

fn local_hidden_func(name: &[u8]) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(0),
        name: Name::from_slice(name),
        binding: Binding::Local,
        is_undefined: false,
        is_common: false,
        is_ifunc: false,
        st_type: elf::STT_FUNC,
        visibility: elf::STV_HIDDEN,
        section: Some(SectionIndex(1)),
        value: 0,
        size: 1,
    }
}
