mod common;
mod sections;
mod segments;

use crate::{LayoutWitness, WitnessError};

pub fn check_layout_witness(witness: &LayoutWitness) -> Result<(), WitnessError> {
    let load_segments = segments::check_segments(witness)?;
    sections::check_output_sections(witness, &load_segments)
}
