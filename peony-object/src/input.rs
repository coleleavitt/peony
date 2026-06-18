use std::path::Path;

use memmap2::Mmap;
use object::read::elf::ElfFile64;
use object::{Endianness, Object, ObjectSymbol};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Bare,
    Archive,
    Shared,
}

pub fn is_shared_object(path: &Path) -> bool {
    matches!(classify_file(path), FileKind::Shared)
}

pub fn classify_file(path: &Path) -> FileKind {
    use std::io::Read;
    let mut hdr = [0u8; 20];
    let Ok(mut f) = std::fs::File::open(path) else {
        return FileKind::Bare;
    };
    let n = f.read(&mut hdr).unwrap_or(0);
    classify_bytes(&hdr[..n])
}

pub fn classify_bytes(h: &[u8]) -> FileKind {
    if h.len() >= 8 && (&h[0..8] == b"!<arch>\n" || &h[0..8] == b"!<thin>\n") {
        return FileKind::Archive;
    }
    if h.len() >= 18 && &h[0..4] == b"\x7fELF" && u16::from_le_bytes([h[16], h[17]]) == 3 {
        return FileKind::Shared;
    }
    FileKind::Bare
}

pub struct MappedInput {
    _mmap: Mmap,
}

impl MappedInput {
    pub fn open(path: &Path) -> Option<MappedInput> {
        let file = std::fs::File::open(path).ok()?;
        let mmap = unsafe { Mmap::map(&file) }.ok()?;
        Some(MappedInput { _mmap: mmap })
    }

    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self._mmap
    }

    #[inline]
    pub fn into_mmap(self) -> Mmap {
        self._mmap
    }
}

pub fn object_defines_global_start(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
        return false;
    };
    let Ok(elf) = ElfFile64::<Endianness>::parse(&*mmap) else {
        return false;
    };
    elf.symbols()
        .any(|s| s.name_bytes() == Ok(b"_start") && !s.is_undefined() && s.is_global())
}

#[cfg(test)]
mod tests {
    #[test]
    fn is_shared_object_edge_cases() {
        let dir = std::env::temp_dir().join(format!("peony-isso-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let short = dir.join("short");
        std::fs::write(&short, b"\x7fELF").unwrap();
        assert!(!super::is_shared_object(&short));

        let nonelf = dir.join("nonelf");
        std::fs::write(&nonelf, vec![0u8; 64]).unwrap();
        assert!(!super::is_shared_object(&nonelf));

        let mut rel = vec![0u8; 20];
        rel[0..4].copy_from_slice(b"\x7fELF");
        rel[16] = 1;
        let relp = dir.join("rel");
        std::fs::write(&relp, &rel).unwrap();
        assert!(!super::is_shared_object(&relp), "ET_REL is not shared");

        rel[16] = 3;
        let dynp = dir.join("dyn");
        std::fs::write(&dynp, &rel).unwrap();
        assert!(super::is_shared_object(&dynp), "ET_DYN is shared");

        let missing = dir.join("nope");
        assert!(!super::is_shared_object(&missing), "missing file → false");

        std::fs::remove_dir_all(&dir).ok();
    }
}
