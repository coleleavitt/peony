use std::path::Path;

use object::read::archive::{ArchiveFile, ArchiveOffset};

use crate::{MappedInput, ObjectError, Result};

/// A single member extracted from an `.rlib` / `.a` archive.
pub struct ArchiveMember {
    pub offset: Option<u64>,
    pub name: String,
    pub data: Vec<u8>,
}

/// Iterate over all members of an `ar`-format archive, yielding raw bytes.
///
/// Used for `.rlib` files: rustc does **not** set modification timestamps on
/// archive members, so byte-level comparison is required for incremental
/// diffing.
pub fn iter_archive_members(path: &Path) -> Result<Vec<ArchiveMember>> {
    let mapped = read_archive(path)?;
    let data = mapped.bytes();
    let Some(file) = parse_archive(path, data)? else {
        return Ok(Vec::new());
    };
    archive_members(path, data, &file)
}

pub fn archive_defines_needed(
    path: &Path,
    mut is_needed: impl FnMut(&[u8]) -> bool,
) -> Result<Option<bool>> {
    let mapped = read_archive(path)?;
    let data = mapped.bytes();
    let Some(file) = parse_archive(path, data)? else {
        return Ok(None);
    };
    Ok(archive_matching_offsets(&file, &mut is_needed).map(|offsets| !offsets.is_empty()))
}

pub fn iter_archive_members_matching(
    path: &Path,
    mut is_needed: impl FnMut(&[u8]) -> bool,
    mut is_new_offset: impl FnMut(u64) -> bool,
) -> Result<Option<Vec<ArchiveMember>>> {
    let mapped = read_archive(path)?;
    let data = mapped.bytes();
    let Some(file) = parse_archive(path, data)? else {
        return Ok(Some(Vec::new()));
    };
    let Some(mut offsets) = archive_matching_offsets(&file, &mut is_needed) else {
        return Ok(None);
    };
    offsets.sort_unstable();
    offsets.dedup();

    let mut members = Vec::with_capacity(offsets.len());
    for offset in offsets {
        if !is_new_offset(offset) {
            continue;
        }
        let member = file
            .member(ArchiveOffset(offset))
            .map_err(|source| archive_parse_error(path, source))?;
        members.push(archive_member(path, data, member, Some(offset))?);
    }
    Ok(Some(members))
}

/// Memory-map an archive rather than copying it onto the heap. A no-pull link
/// only reads the symbol index (a few KB at the front); pulled-member data faults
/// in lazily. Avoids `std::fs::read` copying every multi-MB `.a` (libc.a, libm.a,
/// …) in full just to scan its index. The map outlives every member-byte copy.
fn read_archive(path: &Path) -> Result<MappedInput> {
    MappedInput::open(path).ok_or_else(|| ObjectError::Io {
        path: path.display().to_string(),
        source: std::io::Error::other("could not memory-map archive"),
    })
}

fn parse_archive<'data>(path: &Path, data: &'data [u8]) -> Result<Option<ArchiveFile<'data>>> {
    if data.starts_with(&object::archive::THIN_MAGIC) {
        return Err(ObjectError::UnsupportedArchive {
            path: path.display().to_string(),
            reason: "thin archives are not supported",
        });
    }
    if !data.starts_with(&object::archive::MAGIC)
        && !data.starts_with(&object::archive::AIX_BIG_MAGIC)
    {
        return Ok(None);
    }
    ArchiveFile::parse(data)
        .map(Some)
        .map_err(|source| archive_parse_error(path, source))
}

fn archive_matching_offsets(
    file: &ArchiveFile<'_>,
    is_needed: &mut impl FnMut(&[u8]) -> bool,
) -> Option<Vec<u64>> {
    let Some(mut symbols) = file.symbols().ok()? else {
        return match file.members().next() {
            None => Some(Vec::new()),
            Some(Ok(_)) | Some(Err(_)) => None,
        };
    };
    let mut offsets = Vec::new();
    for symbol in &mut symbols {
        let symbol = symbol.ok()?;
        if is_needed(symbol.name()) {
            offsets.push(symbol.offset().0);
        }
    }
    Some(offsets)
}

fn archive_members<'data>(
    path: &Path,
    data: &'data [u8],
    file: &ArchiveFile<'data>,
) -> Result<Vec<ArchiveMember>> {
    let mut members = Vec::new();
    for member in file.members() {
        let member = member.map_err(|source| archive_parse_error(path, source))?;
        members.push(archive_member(path, data, member, None)?);
    }
    Ok(members)
}

fn archive_member(
    path: &Path,
    data: &[u8],
    member: object::read::archive::ArchiveMember<'_>,
    offset: Option<u64>,
) -> Result<ArchiveMember> {
    let data = member
        .data(data)
        .map_err(|source| archive_parse_error(path, source))?
        .to_vec();
    Ok(ArchiveMember {
        offset,
        name: String::from_utf8_lossy(member.name()).into_owned(),
        data,
    })
}

fn archive_parse_error(path: &Path, source: object::Error) -> ObjectError {
    ObjectError::Parse {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn thin_archive_reports_unsupported() {
        let path =
            std::env::temp_dir().join(format!("peony-thin-{}-{}.a", std::process::id(), line!()));
        std::fs::write(&path, b"!<thin>\n").unwrap();
        let err = super::archive_defines_needed(&path, |_| true).unwrap_err();
        assert!(err.to_string().contains("thin archives are not supported"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn indexed_lookup_returns_only_matching_members() {
        let dir = std::env::temp_dir().join(format!("peony-archive-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let needed = assemble(&dir, "needed", ".text\n.globl needed\nneeded:\n ret\n");
        let unused = assemble(&dir, "unused", ".text\n.globl unused\nunused:\n ret\n");
        let archive = dir.join("libindexed.a");
        let status = Command::new("ar")
            .arg("rcs")
            .arg(&archive)
            .arg(&needed)
            .arg(&unused)
            .status()
            .expect("run ar");
        assert!(status.success());

        let selected =
            super::iter_archive_members_matching(&archive, |name| name == b"needed", |_| true)
                .unwrap()
                .unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected[0].offset.is_some());
        assert!(selected[0].name.contains("needed"));
        assert_eq!(super::iter_archive_members(&archive).unwrap().len(), 2);

        std::fs::remove_dir_all(dir).ok();
    }

    fn assemble(dir: &std::path::Path, name: &str, src: &str) -> std::path::PathBuf {
        let source = dir.join(format!("{name}.s"));
        let object = dir.join(format!("{name}.o"));
        std::fs::write(&source, src).unwrap();
        let status = Command::new("as")
            .args(["--64", "-o"])
            .arg(&object)
            .arg(&source)
            .status()
            .expect("run as");
        assert!(status.success());
        object
    }
}
