use std::path::Path;

use object::read::elf::{Dyn, ElfFile64};
use object::{Endianness, Object, ObjectSymbol};

use crate::{ObjectError, Result, elf};

#[derive(Debug, Clone)]
pub struct SharedObject {
    pub soname: String,
    pub exports: Vec<Vec<u8>>,
    pub export_versions: Vec<Option<Vec<u8>>>,
    pub export_symbols: Vec<SharedExport>,
}

#[derive(Debug, Clone)]
pub struct SharedExport {
    pub name: Vec<u8>,
    pub version: Option<Vec<u8>>,
    pub size: u64,
    pub st_type: u8,
}

pub fn parse_shared_object(path: &Path) -> Result<SharedObject> {
    let data = std::fs::read(path).map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let elf: ElfFile64<Endianness> =
        ElfFile64::parse(data.as_slice()).map_err(|e| ObjectError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;

    let endian = elf.endian();
    let versions = elf
        .elf_section_table()
        .versions(endian, data.as_slice())
        .ok()
        .flatten();

    let mut exports = Vec::new();
    let mut export_versions = Vec::new();
    let mut export_symbols = Vec::new();
    for sym in elf.dynamic_symbols() {
        if sym.is_undefined() || sym.is_local() {
            continue;
        }
        let Ok(name) = sym.name_bytes() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let ver = versions.as_ref().and_then(|vt| {
            let vidx = vt.version_index(endian, sym.index());
            match vt.version(vidx) {
                Ok(Some(v)) => Some(v.name().to_vec()),
                _ => None,
            }
        });
        let name = name.to_vec();
        let st_type = match sym.kind() {
            object::SymbolKind::Text => elf::STT_FUNC,
            object::SymbolKind::Data => elf::STT_OBJECT,
            object::SymbolKind::Tls => elf::STT_TLS,
            object::SymbolKind::Section => elf::STT_SECTION,
            object::SymbolKind::File => elf::STT_FILE,
            _ => elf::STT_NOTYPE,
        };
        export_symbols.push(SharedExport {
            name: name.clone(),
            version: ver.clone(),
            size: sym.size(),
            st_type,
        });
        exports.push(name);
        export_versions.push(ver);
    }

    let soname = elf_soname(&elf, data.as_slice()).unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    });

    Ok(SharedObject {
        soname,
        exports,
        export_versions,
        export_symbols,
    })
}

fn elf_soname(elf: &ElfFile64<Endianness>, data: &[u8]) -> Option<String> {
    let endian = elf.endian();
    let (dynamic, dyn_index) = elf.elf_section_table().dynamic(endian, data).ok()??;
    let strings = elf
        .elf_section_table()
        .strings(endian, data, dyn_index)
        .ok()?;
    for d in dynamic {
        if d.tag32(endian) == Some(elf::DT_SONAME as u32) {
            if let Ok(name) = d.string(endian, strings) {
                return Some(String::from_utf8_lossy(name).into_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_shared() {
        for p in [
            "/lib/x86_64-linux-gnu/libc.so.6",
            "/usr/lib/x86_64-linux-gnu/libc.so.6",
            "/lib64/libc.so.6",
        ] {
            let path = std::path::Path::new(p);
            if path.exists() {
                let r = super::parse_shared_object(path).unwrap();
                assert!(!r.exports.is_empty(), "libc should export symbols");
                assert!(
                    r.exports.iter().any(|e| e == b"printf"),
                    "libc should export printf"
                );
                return;
            }
        }
    }
}
