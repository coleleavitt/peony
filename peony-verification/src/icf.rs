mod check;
mod extract;
mod model;
mod taint;

pub use check::check_icf_fold_witnesses;
pub use extract::extract_icf_fold_witnesses;
pub use model::{
    IcfFoldKeyWitness,
    IcfFoldPairWitness,
    IcfFoldWitness,
    IcfRelocationSummaryWitness,
    IcfSectionWitness,
    IcfWitnessError,
};
