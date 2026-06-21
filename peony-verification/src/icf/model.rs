use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::SectionRefWitness;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IcfFoldPairWitness {
    pub duplicate: SectionRefWitness,
    pub canonical: SectionRefWitness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcfRelocationSummaryWitness {
    pub offset: u64,
    pub relocation_type: u32,
    pub addend: i64,
    pub target_name: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcfFoldKeyWitness {
    pub flags: u64,
    pub len: u64,
    pub content_digest: u128,
    pub relocation_summaries: Vec<IcfRelocationSummaryWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcfSectionWitness {
    pub section: SectionRefWitness,
    pub is_text: bool,
    pub has_contents: bool,
    pub object_has_addrsig: bool,
    pub section_address_taken: bool,
    pub has_addrsig_symbol: bool,
    pub has_named_address_taken_symbol: bool,
    pub has_abi_unique_symbol: bool,
    pub has_weak_definition: bool,
    pub has_default_visible_non_local_definition: bool,
    pub reloc_targets_resolved: bool,
    pub address_safe: bool,
    pub fold_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcfFoldWitness {
    pub duplicate: SectionRefWitness,
    pub canonical: SectionRefWitness,
    pub duplicate_key: IcfFoldKeyWitness,
    pub canonical_key: IcfFoldKeyWitness,
    pub duplicate_section: IcfSectionWitness,
    pub canonical_section: IcfSectionWitness,
    pub flags_equal: bool,
    pub len_equal: bool,
    pub bytes_equal: bool,
    pub relocation_summaries_equal: bool,
    pub address_taint_known: bool,
    pub address_safe: bool,
}

impl IcfFoldWitness {
    pub(super) const fn pair(&self) -> IcfFoldPairWitness {
        IcfFoldPairWitness {
            duplicate: self.duplicate,
            canonical: self.canonical,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IcfWitnessError {
    #[error("ICF witness references missing section {section:?}")]
    MissingSection { section: SectionRefWitness },
    #[error(
        "ICF section witness {section:?} mismatches {field}: witness={witness}, computed={computed}"
    )]
    SectionPredicateMismatch {
        section: SectionRefWitness,
        field: &'static str,
        witness: bool,
        computed: bool,
    },
    #[error("ICF fold witness {duplicate:?}->{canonical:?} mismatches {field}")]
    FoldPredicateMismatch {
        duplicate: SectionRefWitness,
        canonical: SectionRefWitness,
        field: &'static str,
    },
    #[error(
        "ICF fold witnesses differ from Rust fold map: missing={missing:?}, unexpected={unexpected:?}"
    )]
    FoldSetMismatch {
        missing: Vec<IcfFoldPairWitness>,
        unexpected: Vec<IcfFoldPairWitness>,
    },
    #[error(
        "ICF fold {duplicate:?}->{canonical:?} does not satisfy address_safe: duplicate_safe={duplicate_safe}, canonical_safe={canonical_safe}"
    )]
    AddressUnsafe {
        duplicate: SectionRefWitness,
        canonical: SectionRefWitness,
        duplicate_safe: bool,
        canonical_safe: bool,
    },
}
