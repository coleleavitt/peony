use peony_object::{Binding, InputArena, InputObject, InputSymbol, Name, SymbolIndex, elf};
use peony_symbols::SymbolError;

use crate::{
    SymbolErrorWitness,
    SymbolStateWitness,
    extract_symbol_error_witness,
    extract_symbol_witnesses,
};

fn symbol_object(path: &str, symbols: Vec<InputSymbol>) -> InputObject {
    let mut arena = InputArena::new();
    super::object(path, vec![super::text_section(&mut arena, 1, &[])], symbols)
}

fn undefined_symbol(name: &[u8], binding: Binding) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(0),
        name: Name::from_slice(name),
        binding,
        is_undefined: true,
        is_common: false,
        is_ifunc: false,
        st_type: elf::STT_NOTYPE,
        visibility: elf::STV_DEFAULT,
        section: None,
        value: 0,
        size: 0,
    }
}

fn common_symbol(name: &[u8], size: u64, align: u64) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(0),
        name: Name::from_slice(name),
        binding: Binding::Global,
        is_undefined: false,
        is_common: true,
        is_ifunc: false,
        st_type: elf::STT_OBJECT,
        visibility: elf::STV_DEFAULT,
        section: None,
        value: align,
        size,
    }
}

#[test]
fn extracts_s1_symbol_witness_variants_when_table_contains_concrete_results() {
    let objects = vec![
        symbol_object(
            "defined.o",
            vec![super::symbol(b"defined", Binding::Global, 1)],
        ),
        symbol_object(
            "undefined.o",
            vec![undefined_symbol(b"undefined", Binding::Global)],
        ),
        symbol_object(
            "weak.o",
            vec![super::symbol(b"weak_defined", Binding::Weak, 1)],
        ),
        symbol_object("common.o", vec![common_symbol(b"common", 32, 16)]),
    ];
    let mut table = super::symbol_table_for(&objects);
    table.define_absolute(b"absolute", 0x401234);
    table.force_undefined(b"imported");
    assert_eq!(
        table.register_shared_exports_versioned(
            &[b"imported".to_vec()],
            &[Some(b"LIB_1.0".to_vec())],
            "libfixture.so",
        ),
        1,
    );
    table.mark_copy_reloc(b"imported");
    table
        .lookup_mut(b"imported")
        .expect("imported symbol is present")
        .dynsym_index = 7;

    let witnesses = extract_symbol_witnesses(&table);

    let defined = witness(&witnesses, b"defined");
    assert!(matches!(
        defined.state,
        SymbolStateWitness::Defined {
            object_id: 0,
            section_index: 1,
        }
    ));
    let undefined = witness(&witnesses, b"undefined");
    assert_eq!(undefined.state, SymbolStateWitness::Undefined);
    let weak_defined = witness(&witnesses, b"weak_defined");
    assert!(matches!(
        weak_defined.state,
        SymbolStateWitness::Defined {
            object_id: 2,
            section_index: 1,
        }
    ));
    let common = witness(&witnesses, b"common");
    assert_eq!(
        common.state,
        SymbolStateWitness::Common {
            size: 32,
            align: 16,
        }
    );
    let absolute = witness(&witnesses, b"absolute");
    assert_eq!(
        absolute.state,
        SymbolStateWitness::Absolute { object_id: 0 }
    );
    let imported = witness(&witnesses, b"imported");
    assert_eq!(
        imported.state,
        SymbolStateWitness::Import {
            copy_reloc: true,
            dynsym_index: 7,
            version: Some(b"LIB_1.0".to_vec()),
            soname: Some("libfixture.so".to_string()),
        }
    );
}

#[test]
fn extracts_duplicate_strong_error_witness_when_symbol_resolution_rejects_input() {
    let error = SymbolError::DuplicateSymbol {
        name: "dup".to_string(),
        first: "first.o".to_string(),
        second: "second.o".to_string(),
    };

    let witness = extract_symbol_error_witness(&error);

    assert_eq!(
        witness,
        SymbolErrorWitness::DuplicateStrong {
            name: b"dup".to_vec(),
            first: "first.o".to_string(),
            second: "second.o".to_string(),
        }
    );
}

fn witness<'a>(witnesses: &'a [crate::SymbolWitness], name: &[u8]) -> &'a crate::SymbolWitness {
    witnesses
        .iter()
        .find(|witness| witness.name == name)
        .expect("symbol witness exists")
}
