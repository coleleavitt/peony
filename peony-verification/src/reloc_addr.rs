mod types;

pub use types::{
    ApplyRelocAddressError,
    ApplyRelocAddressWitness,
    ApplyRelocPlaceWitness,
    ApplyRelocTargetWitness,
    ApplyRelocTlsWitness,
};

use crate::RelocationByteInputs;

pub fn check_apply_reloc_address_witness(
    witness: &ApplyRelocAddressWitness,
) -> Result<RelocationByteInputs, ApplyRelocAddressError> {
    let expected = expected_inputs(witness)?;
    require_inputs(expected, witness.inputs)?;
    Ok(witness.inputs)
}

fn expected_inputs(
    witness: &ApplyRelocAddressWitness,
) -> Result<RelocationByteInputs, ApplyRelocAddressError> {
    let (s, g, l, z) = resolved_target(witness.target)?;
    let (tls, tls_size, tls_gd, tls_ie, tls_desc, tls_ldm, tls_imported) =
        resolved_tls(witness.tls)?;
    let offset = usize::try_from(witness.place.reloc_offset).map_err(|_| {
        ApplyRelocAddressError::OffsetOutOfRange {
            offset: witness.place.reloc_offset,
        }
    })?;
    let p = witness
        .place
        .section_va
        .checked_add(witness.place.reloc_offset)
        .ok_or(ApplyRelocAddressError::AddressOverflow {
            field: "p",
            base: witness.place.section_va,
            addend: witness.place.reloc_offset,
        })?;

    Ok(RelocationByteInputs {
        s,
        a: witness.addend,
        p,
        g,
        l,
        z,
        got_base: witness.got_base,
        tls,
        tls_size,
        offset,
        shared: witness.shared,
        tls_gd,
        tls_ie,
        tls_desc,
        tls_ldm,
        tls_imported,
    })
}

fn resolved_target(
    target: ApplyRelocTargetWitness,
) -> Result<(u64, u64, u64, u64), ApplyRelocAddressError> {
    match target {
        ApplyRelocTargetWitness::LocalSection {
            section_address,
            symbol_value,
            size,
        } => {
            let section_address =
                section_address.ok_or(ApplyRelocAddressError::MissingAddress {
                    field: "target.section_address",
                })?;
            let s = section_address.checked_add(symbol_value).ok_or(
                ApplyRelocAddressError::AddressOverflow {
                    field: "s",
                    base: section_address,
                    addend: symbol_value,
                },
            )?;
            Ok((s, 0, 0, size))
        }
        ApplyRelocTargetWitness::LocalAbsolute { symbol_value, size } => {
            Ok((symbol_value, 0, 0, size))
        }
        ApplyRelocTargetWitness::GlobalDefined {
            virtual_address,
            got_address,
            plt_address,
            size,
        } => Ok((virtual_address, got_address, plt_address, size)),
        ApplyRelocTargetWitness::WeakUndefined { got_address } => Ok((0, got_address, 0, 0)),
    }
}

fn resolved_tls(
    tls: ApplyRelocTlsWitness,
) -> Result<(u64, u64, u64, u64, u64, u64, bool), ApplyRelocAddressError> {
    match tls {
        ApplyRelocTlsWitness::Absent { tls_size } => Ok((0, tls_size, 0, 0, 0, 0, false)),
        ApplyRelocTlsWitness::SectionRelative {
            section_tls_offset,
            symbol_value,
            tls_size,
            tls_gd,
            tls_ie,
            tls_desc,
            tls_ldm,
        } => {
            let section_tls_offset =
                section_tls_offset.ok_or(ApplyRelocAddressError::MissingAddress {
                    field: "tls.section_tls_offset",
                })?;
            let tls = section_tls_offset.checked_add(symbol_value).ok_or(
                ApplyRelocAddressError::AddressOverflow {
                    field: "tls",
                    base: section_tls_offset,
                    addend: symbol_value,
                },
            )?;
            Ok((tls, tls_size, tls_gd, tls_ie, tls_desc, tls_ldm, false))
        }
        ApplyRelocTlsWitness::Imported {
            tls_size,
            tls_gd,
            tls_ie,
            tls_desc,
            tls_ldm,
        } => Ok((0, tls_size, tls_gd, tls_ie, tls_desc, tls_ldm, true)),
    }
}

fn require_inputs(
    expected: RelocationByteInputs,
    actual: RelocationByteInputs,
) -> Result<(), ApplyRelocAddressError> {
    require_u64("s", expected.s, actual.s)?;
    require_i64("a", expected.a, actual.a)?;
    require_u64("p", expected.p, actual.p)?;
    require_u64("g", expected.g, actual.g)?;
    require_u64("l", expected.l, actual.l)?;
    require_u64("z", expected.z, actual.z)?;
    require_u64("got_base", expected.got_base, actual.got_base)?;
    require_u64("tls", expected.tls, actual.tls)?;
    require_u64("tls_size", expected.tls_size, actual.tls_size)?;
    require_usize("offset", expected.offset, actual.offset)?;
    require_bool("shared", expected.shared, actual.shared)?;
    require_u64("tls_gd", expected.tls_gd, actual.tls_gd)?;
    require_u64("tls_ie", expected.tls_ie, actual.tls_ie)?;
    require_u64("tls_desc", expected.tls_desc, actual.tls_desc)?;
    require_u64("tls_ldm", expected.tls_ldm, actual.tls_ldm)?;
    require_bool("tls_imported", expected.tls_imported, actual.tls_imported)?;
    Ok(())
}

fn require_u64(
    field: &'static str,
    expected: u64,
    actual: u64,
) -> Result<(), ApplyRelocAddressError> {
    require_i128(field, i128::from(expected), i128::from(actual))
}

fn require_i64(
    field: &'static str,
    expected: i64,
    actual: i64,
) -> Result<(), ApplyRelocAddressError> {
    require_i128(field, i128::from(expected), i128::from(actual))
}

fn require_usize(
    field: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), ApplyRelocAddressError> {
    let expected = i128::try_from(expected)
        .map_err(|_| ApplyRelocAddressError::OffsetOutOfRange { offset: u64::MAX })?;
    let actual = i128::try_from(actual)
        .map_err(|_| ApplyRelocAddressError::OffsetOutOfRange { offset: u64::MAX })?;
    require_i128(field, expected, actual)
}

fn require_bool(
    field: &'static str,
    expected: bool,
    actual: bool,
) -> Result<(), ApplyRelocAddressError> {
    let expected = if expected { 1 } else { 0 };
    let actual = if actual { 1 } else { 0 };
    require_i128(field, expected, actual)
}

fn require_i128(
    field: &'static str,
    expected: i128,
    actual: i128,
) -> Result<(), ApplyRelocAddressError> {
    if expected == actual {
        return Ok(());
    }
    Err(ApplyRelocAddressError::InputMismatch {
        field,
        expected,
        actual,
    })
}
