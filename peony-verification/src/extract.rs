use std::collections::HashMap;

use peony_cache::{PartialRelinkPlan, PatchSectionRecord, SectionRecord};
use peony_object::{Binding, InputObject, SectionKind};
use peony_symbols::{SymbolError, SymbolResolution, SymbolTable};

use crate::{
    IncrementalColorWitness,
    SectionKindWitness,
    SectionRefWitness,
    SectionWitness,
    SymbolBindingWitness,
    SymbolErrorWitness,
    SymbolStateWitness,
    SymbolWitness,
    WitnessError,
};

pub fn extract_section_witnesses(objects: &[InputObject]) -> Vec<SectionWitness> {
    let mut witnesses = Vec::new();
    for (object_id, object) in objects.iter().enumerate() {
        for section in &object.sections {
            witnesses.push(SectionWitness {
                owner: SectionRefWitness::new(object_id, section.index.0),
                name: section.name.as_bytes().to_vec(),
                kind: section.kind.into(),
                flags: section.flags,
                size: section.size,
            });
        }
    }
    witnesses.sort_by(|left, right| {
        left.owner
            .cmp(&right.owner)
            .then_with(|| left.name.cmp(&right.name))
    });
    witnesses
}

pub fn extract_symbol_witnesses(table: &SymbolTable) -> Vec<SymbolWitness> {
    let mut witnesses: Vec<SymbolWitness> = table
        .iter()
        .map(|(name, resolution)| SymbolWitness {
            name: name.to_vec(),
            symbol_id: symbol_id(resolution),
            binding: resolution.binding.into(),
            state: symbol_state(resolution),
            value: resolution.value,
            size: resolution.size,
            virtual_address: resolution.virtual_address,
        })
        .collect();
    witnesses.sort_by(|left, right| left.name.cmp(&right.name));
    witnesses
}

pub fn extract_symbol_error_witness(error: &SymbolError) -> SymbolErrorWitness {
    match error {
        SymbolError::DuplicateSymbol {
            name,
            first,
            second,
        } => SymbolErrorWitness::DuplicateStrong {
            name: name.as_bytes().to_vec(),
            first: first.clone(),
            second: second.clone(),
        },
        SymbolError::UndefinedSymbol { name } => SymbolErrorWitness::Undefined {
            name: name.as_bytes().to_vec(),
        },
    }
}

pub fn extract_incremental_color_witnesses(
    plan: &PartialRelinkPlan,
    previous_sections: &[SectionRecord],
    current_sections: &[PatchSectionRecord],
) -> Result<Vec<IncrementalColorWitness>, WitnessError> {
    let previous_by_name: HashMap<&str, &SectionRecord> = previous_sections
        .iter()
        .map(|section| (section.name.as_str(), section))
        .collect();
    let mut current_sorted: Vec<&PatchSectionRecord> = current_sections.iter().collect();
    current_sorted.sort_by(|left, right| left.name.cmp(&right.name));

    let mut witnesses = Vec::with_capacity(current_sorted.len());
    for current in current_sorted {
        let color =
            plan.color(&current.name)
                .ok_or_else(|| WitnessError::MissingIncrementalColor {
                    section: current.name.clone(),
                })?;
        let previous = previous_by_name.get(current.name.as_str()).ok_or_else(|| {
            WitnessError::MissingPreviousSectionRecord {
                section: current.name.clone(),
            }
        })?;
        witnesses.push(IncrementalColorWitness {
            section_name: current.name.clone(),
            file_offset: current.file_offset,
            virtual_address: current.virtual_address,
            size: current.size,
            capacity: previous.capacity,
            color: color.into(),
        });
    }
    Ok(witnesses)
}

impl From<SectionKind> for SectionKindWitness {
    fn from(value: SectionKind) -> Self {
        match value {
            SectionKind::Text => Self::Text,
            SectionKind::ReadOnly => Self::ReadOnly,
            SectionKind::Data => Self::Data,
            SectionKind::Bss => Self::Bss,
            SectionKind::Debug => Self::Debug,
            SectionKind::EhFrame => Self::EhFrame,
            SectionKind::MergeString => Self::MergeString,
            SectionKind::MergeConst => Self::MergeConst,
            SectionKind::InitArray => Self::InitArray,
            SectionKind::Tdata => Self::Tdata,
            SectionKind::Tbss => Self::Tbss,
            SectionKind::Other => Self::Other,
        }
    }
}

impl From<Binding> for SymbolBindingWitness {
    fn from(value: Binding) -> Self {
        match value {
            Binding::Local => Self::Local,
            Binding::Global => Self::Global,
            Binding::Weak => Self::Weak,
        }
    }
}

fn symbol_id(resolution: &SymbolResolution) -> Option<u32> {
    let id = resolution.id.0;
    if id == u32::MAX { None } else { Some(id) }
}

fn symbol_state(resolution: &SymbolResolution) -> SymbolStateWitness {
    if resolution.import {
        return SymbolStateWitness::Import {
            copy_reloc: resolution.copy_reloc,
            dynsym_index: resolution.dynsym_index,
            version: resolution.version.clone(),
            soname: resolution.soname.clone(),
        };
    }
    if let Some((size, align)) = resolution.common {
        return SymbolStateWitness::Common { size, align };
    }
    match (resolution.defined_in, resolution.section_index) {
        (Some(object_id), Some(section_index)) => SymbolStateWitness::Defined {
            object_id: object_id.0,
            section_index,
        },
        (Some(object_id), None) => SymbolStateWitness::Absolute {
            object_id: object_id.0,
        },
        (None, _) => SymbolStateWitness::Undefined,
    }
}
