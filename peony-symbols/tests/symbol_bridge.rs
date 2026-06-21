use peony_object::{
    Binding,
    IndexLookup,
    InputObject,
    InputSection,
    InputSymbol,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};
use peony_symbols::{SymbolError, SymbolTable};

fn text_section(index: usize) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".text"),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: SectionData::EMPTY,
        align: 1,
        size: 1,
        flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        relocs: Vec::new(),
    }
}

fn symbol(index: usize, name: &[u8], binding: Binding, section: Option<usize>) -> InputSymbol {
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
        value: 0,
        size: 1,
    }
}

#[derive(Clone, Copy)]
struct CommonSymbol {
    size: u64,
    align: u64,
}

fn common_symbol(index: usize, name: &[u8], common: CommonSymbol) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(index),
        name: Name::from_slice(name),
        binding: Binding::Global,
        is_undefined: false,
        is_common: true,
        is_ifunc: false,
        st_type: elf::STT_OBJECT,
        visibility: elf::STV_DEFAULT,
        section: None,
        value: common.align,
        size: common.size,
    }
}

fn object(path: &str, symbols: Vec<InputSymbol>) -> InputObject {
    let sections = vec![text_section(1)];
    let mut section_map = IndexLookup::default();
    section_map.insert(1, 0);
    let mut symbol_map = IndexLookup::default();
    for (pos, sym) in symbols.iter().enumerate() {
        symbol_map.insert(sym.index.0, pos);
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

#[test]
fn strong_definition_satisfies_prior_undefined_reference() {
    let undef = object("undef.o", vec![symbol(0, b"foo", Binding::Global, None)]);
    let strong = object(
        "strong.o",
        vec![symbol(0, b"foo", Binding::Global, Some(1))],
    );
    let mut table = SymbolTable::new();

    let undef_id = table.add_object(undef.path.clone());
    table
        .process_object(undef_id, &undef)
        .expect("undefined reference is recorded");
    let strong_id = table.add_object(strong.path.clone());
    table
        .process_object(strong_id, &strong)
        .expect("strong definition satisfies undefined reference");

    let resolved = table.lookup(b"foo").expect("foo resolved");
    assert!(resolved.is_defined());
    assert_eq!(resolved.binding, Binding::Global);
    assert_eq!(resolved.defined_in, Some(strong_id));
}

#[test]
fn undefined_reference_remains_unresolved_until_definition_or_import_arrives() {
    let undef = object(
        "undef.o",
        vec![symbol(0, b"missing", Binding::Global, None)],
    );
    let mut table = SymbolTable::new();

    let undef_id = table.add_object(undef.path.clone());
    table
        .process_object(undef_id, &undef)
        .expect("undefined reference is recorded");

    let resolved = table.lookup(b"missing").expect("missing recorded");
    assert!(!resolved.is_defined());
    assert_eq!(resolved.binding, Binding::Global);
    assert_eq!(resolved.defined_in, None);
    assert_eq!(resolved.section_index, None);
}

#[test]
fn strong_definition_replaces_weak_definition() {
    let weak = object("weak.o", vec![symbol(0, b"foo", Binding::Weak, Some(1))]);
    let strong = object(
        "strong.o",
        vec![symbol(0, b"foo", Binding::Global, Some(1))],
    );
    let mut table = SymbolTable::new();

    let weak_id = table.add_object(weak.path.clone());
    table
        .process_object(weak_id, &weak)
        .expect("weak definition is accepted");
    let strong_id = table.add_object(strong.path.clone());
    table
        .process_object(strong_id, &strong)
        .expect("strong definition upgrades weak definition");

    let resolved = table.lookup(b"foo").expect("foo resolved");
    assert_eq!(resolved.binding, Binding::Global);
    assert_eq!(resolved.defined_in, Some(strong_id));
}

#[test]
fn first_weak_definition_wins_when_later_weak_definition_uses_same_name() {
    let first = object("first.o", vec![symbol(0, b"foo", Binding::Weak, Some(1))]);
    let second = object("second.o", vec![symbol(0, b"foo", Binding::Weak, Some(1))]);
    let mut table = SymbolTable::new();

    let first_id = table.add_object(first.path.clone());
    table
        .process_object(first_id, &first)
        .expect("first weak definition is accepted");
    let second_id = table.add_object(second.path.clone());
    table
        .process_object(second_id, &second)
        .expect("second weak definition keeps existing winner");

    let resolved = table.lookup(b"foo").expect("foo resolved");
    assert_eq!(resolved.binding, Binding::Weak);
    assert_eq!(resolved.defined_in, Some(first_id));
}

#[test]
fn common_definitions_merge_size_and_alignment_without_changing_provenance() {
    let small = object(
        "small.o",
        vec![common_symbol(0, b"buf", CommonSymbol { size: 8, align: 4 })],
    );
    let large = object(
        "large.o",
        vec![common_symbol(
            0,
            b"buf",
            CommonSymbol {
                size: 32,
                align: 16,
            },
        )],
    );
    let mut table = SymbolTable::new();

    let small_id = table.add_object(small.path.clone());
    table
        .process_object(small_id, &small)
        .expect("first common definition is accepted");
    let large_id = table.add_object(large.path.clone());
    table
        .process_object(large_id, &large)
        .expect("second common definition merges");

    let resolved = table.lookup(b"buf").expect("buf resolved");
    assert_eq!(resolved.common, Some((32, 16)));
    assert_eq!(resolved.size, 32);
    assert_eq!(resolved.defined_in, Some(small_id));
}

#[test]
fn absolute_definition_records_absolute_value_without_section_provenance() {
    let mut table = SymbolTable::new();

    table.define_absolute(b"_end", 0x402000);

    let resolved = table.lookup(b"_end").expect("_end resolved");
    assert!(resolved.is_defined());
    assert_eq!(resolved.defined_in, Some(peony_symbols::ObjectId(0)));
    assert_eq!(resolved.section_index, None);
    assert_eq!(resolved.value, 0x402000);
    assert_eq!(resolved.virtual_address, 0x402000);
}

#[test]
fn shared_export_satisfies_undefined_reference_as_import_with_metadata() {
    let undef = object("undef.o", vec![symbol(0, b"puts", Binding::Global, None)]);
    let mut table = SymbolTable::new();

    let undef_id = table.add_object(undef.path.clone());
    table
        .process_object(undef_id, &undef)
        .expect("undefined reference is recorded");
    assert_eq!(
        table.register_shared_exports_versioned(
            &[b"puts".to_vec()],
            &[Some(b"GLIBC_2.2.5".to_vec())],
            "libc.so.6",
        ),
        1,
    );
    table.mark_copy_reloc(b"puts");

    let resolved = table.lookup(b"puts").expect("puts resolved");
    assert!(resolved.is_defined());
    assert_eq!(resolved.defined_in, None);
    assert!(resolved.import);
    assert!(resolved.copy_reloc);
    assert_eq!(resolved.version.as_deref(), Some(b"GLIBC_2.2.5".as_slice()));
    assert_eq!(resolved.soname.as_deref(), Some("libc.so.6"));
}

#[test]
fn duplicate_strong_definition_is_rejected() {
    let first = object("first.o", vec![symbol(0, b"foo", Binding::Global, Some(1))]);
    let second = object(
        "second.o",
        vec![symbol(0, b"foo", Binding::Global, Some(1))],
    );
    let mut table = SymbolTable::new();

    let first_id = table.add_object(first.path.clone());
    table
        .process_object(first_id, &first)
        .expect("first strong definition is accepted");
    let second_id = table.add_object(second.path.clone());
    let err = table
        .process_object(second_id, &second)
        .expect_err("second strong definition must fail");

    assert!(matches!(
        err,
        SymbolError::DuplicateSymbol { name, first, second }
            if name == "foo" && first == "first.o" && second == "second.o"
    ));
}
