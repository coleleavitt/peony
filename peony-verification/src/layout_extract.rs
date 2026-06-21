use peony_layout::{Layout, SecSource};
use peony_object::elf;

use crate::{
    ContributionOwnerWitness,
    LayoutSegmentWitness,
    LayoutWindowWitness,
    LayoutWitness,
    RangeBounds,
    RangeOwnerWitness,
    RangeWitness,
    SectionRefWitness,
    WitnessError,
};

pub fn extract_layout_witness(layout: &Layout) -> Result<LayoutWitness, WitnessError> {
    Ok(LayoutWitness {
        image_base: layout.image_base,
        file_size: layout.file_size,
        output_sections: extract_layout_window_witnesses(layout)?,
        segments: extract_layout_segment_witnesses(layout)?,
    })
}

pub fn extract_layout_window_witnesses(
    layout: &Layout,
) -> Result<Vec<LayoutWindowWitness>, WitnessError> {
    let mut witnesses = Vec::with_capacity(layout.output_sections.len());
    for section in &layout.output_sections {
        let file = RangeBounds::from_start_len(section.sh_offset, file_len(section))?;
        let va = RangeBounds::from_start_len(section.sh_addr, virtual_len(section))?;
        let mut contributions = section_contributions(section)?;
        contributions.sort_by_key(|owner| {
            (
                owner.range.start,
                owner.section.object_id,
                owner.section.section_index,
            )
        });
        witnesses.push(LayoutWindowWitness {
            output_section_name: section.name.clone(),
            section_type: section.sh_type,
            flags: section.sh_flags,
            range: RangeWitness::new(
                RangeOwnerWitness::OutputSection {
                    name: section.name.clone(),
                },
                file,
                va,
            ),
            alignment: section.sh_addralign,
            contributions,
        });
    }
    witnesses.sort_by(|left, right| {
        left.range
            .file
            .cmp(&right.range.file)
            .then_with(|| left.output_section_name.cmp(&right.output_section_name))
    });
    Ok(witnesses)
}

pub fn extract_layout_segment_witnesses(
    layout: &Layout,
) -> Result<Vec<LayoutSegmentWitness>, WitnessError> {
    let mut witnesses = Vec::with_capacity(layout.segments.len());
    for (index, segment) in layout.segments.iter().enumerate() {
        witnesses.push(LayoutSegmentWitness {
            index,
            segment_type: segment.p_type,
            flags: segment.p_flags,
            range: RangeWitness::new(
                RangeOwnerWitness::ProgramHeader {
                    index,
                    segment_type: segment.p_type,
                },
                RangeBounds::from_start_len(segment.p_offset, segment.p_filesz)?,
                RangeBounds::from_start_len(segment.p_vaddr, segment.p_memsz)?,
            ),
            alignment: segment.p_align,
        });
    }
    Ok(witnesses)
}

fn file_len(section: &peony_layout::OutputSection) -> u64 {
    match section.source {
        SecSource::Bss => 0,
        SecSource::Input
        | SecSource::Got
        | SecSource::SymTab
        | SecSource::SymTabShndx
        | SecSource::StrTab
        | SecSource::ShStrTab
        | SecSource::NoteBuildId
        | SecSource::NoteGnuProperty
        | SecSource::Interp
        | SecSource::Hash
        | SecSource::DynSym
        | SecSource::DynStr
        | SecSource::RelaDyn
        | SecSource::Dynamic
        | SecSource::Plt
        | SecSource::GotPlt
        | SecSource::RelaPlt
        | SecSource::GnuVersion
        | SecSource::GnuVersionR
        | SecSource::EhFrameHdr
        | SecSource::GnuHash
        | SecSource::RelaEmit(_) => section.sh_size,
    }
}

fn virtual_len(section: &peony_layout::OutputSection) -> u64 {
    if section.sh_flags & elf::SHF_ALLOC != 0 {
        section.sh_size
    } else {
        0
    }
}

fn section_contributions(
    section: &peony_layout::OutputSection,
) -> Result<Vec<ContributionOwnerWitness>, WitnessError> {
    let mut contributions = Vec::with_capacity(section.contributions.len());
    for contribution in &section.contributions {
        contributions.push(ContributionOwnerWitness {
            section: SectionRefWitness::new(contribution.object_id, contribution.section_index),
            output_offset: contribution.offset,
            size: contribution.size,
            range: RangeBounds::from_start_len(contribution.offset, contribution.size)?,
        });
    }
    Ok(contributions)
}
