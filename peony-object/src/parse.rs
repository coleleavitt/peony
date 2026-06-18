use std::path::Path;

use memmap2::Mmap;
use object::read::elf::{ElfFile64, SectionHeader};
use object::{Endianness, Object, ObjectSection, ObjectSymbol};
use rustc_hash::FxHashMap;

use crate::section_parse::{
    classify_section,
    collect_relocs,
    is_debug_section_name,
    normalize_debug_section_name,
    parse_comdat_groups,
};
use crate::{
    Binding,
    DataSrc,
    InputArena,
    InputObject,
    InputSection,
    InputSymbol,
    Name,
    ObjectError,
    Result,
    SectionData,
    SectionKind,
    elf,
    ends_with_eh_terminator,
    scan_eh_frame,
};

pub fn parse_object(arena: &mut InputArena, path: &Path) -> Result<InputObject> {
    let file = std::fs::File::open(path).map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let file_id = arena.push_mmap(mmap);
    let bytes = arena.mmap_bytes(file_id);
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.len()) };
    parse_bytes_into(arena, file_id, path.display().to_string(), bytes)
}

pub fn parse_bytes_into(
    arena: &mut InputArena,
    file_id: u32,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    let base = arena.owned.len() as u32;
    let mut sink = Vec::new();
    let mut obj = parse_backed(DataSrc::Mmap(file_id), base, 0, &mut sink, path, data)?;
    arena.owned.extend(sink);
    arena.intern_object_names(&mut obj);
    Ok(obj)
}

pub fn parse_owned_member(
    arena: &mut InputArena,
    owned_id: u32,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    let base = arena.owned.len() as u32;
    let mut sink = Vec::new();
    let mut obj = parse_backed(DataSrc::Owned(owned_id), base, 0, &mut sink, path, data)?;
    arena.owned.extend(sink);
    arena.intern_object_names(&mut obj);
    Ok(obj)
}

pub fn parse_bare_parallel(
    file_id: u32,
    owned_base: u32,
    path: String,
    data: &[u8],
) -> Result<(InputObject, Vec<Vec<u8>>)> {
    let mut sink = Vec::new();
    let obj = parse_backed(DataSrc::Mmap(file_id), owned_base, 0, &mut sink, path, data)?;
    Ok((obj, sink))
}

fn parse_backed(
    backing: DataSrc,
    owned_base: u32,
    base_off: u64,
    owned_sink: &mut Vec<Vec<u8>>,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    let elf: ElfFile64<Endianness> = ElfFile64::parse(data).map_err(|e| ObjectError::Parse {
        path: path.clone(),
        source: e,
    })?;

    let section_count = elf.sections().count();
    let mut sections = Vec::with_capacity(section_count);
    let mut section_map = FxHashMap::default();
    section_map.reserve(section_count);
    let endian = elf.endian();

    for section in elf.sections() {
        let idx = section.index();
        let sh_type = section.elf_section_header().sh_type(endian);
        let input_name = section.name_bytes().unwrap_or(b"");
        let is_debug = is_debug_section_name(input_name);
        let name = normalize_debug_section_name(input_name);
        let sh_flags = match section.flags() {
            object::SectionFlags::Elf { sh_flags } => sh_flags,
            _ => 0,
        };
        let kind = classify_section(&name, sh_flags);

        let (file_off, file_len) = section.file_range().unwrap_or((0, 0));
        let is_compressed = matches!(
            section.compressed_file_range(),
            Ok(r) if r.format != object::CompressionFormat::None
        );

        let mut data_handle: SectionData;
        let mut owned_len: Option<u64> = None;
        if is_debug && is_compressed {
            let bytes = section
                .uncompressed_data()
                .map_err(|e| ObjectError::Parse {
                    path: path.clone(),
                    source: e,
                })?
                .into_owned();
            let len = bytes.len();
            assert!(len <= u32::MAX as usize, "section too large for u32 len");
            let oid = owned_base + owned_sink.len() as u32;
            owned_sink.push(bytes);
            owned_len = Some(len as u64);
            data_handle = SectionData {
                src: DataSrc::Owned(oid),
                off: 0,
                len: len as u32,
            };
        } else if sh_type == elf::SHT_NOBITS || file_len == 0 {
            data_handle = SectionData::EMPTY;
        } else {
            let abs_off = base_off + file_off;
            assert!(
                abs_off + file_len <= u32::MAX as u64,
                "section offset+len {} exceeds u32 (archive/object > 4GiB not supported)",
                abs_off + file_len
            );
            data_handle = SectionData {
                src: backing,
                off: abs_off as u32,
                len: file_len as u32,
            };
        }
        let relocs = collect_relocs(&section);

        let mut size = if is_debug {
            owned_len.unwrap_or(file_len)
        } else {
            section.size()
        };
        if kind == SectionKind::EhFrame {
            let raw_data = &data[file_off as usize..(file_off + file_len) as usize];
            let (cies, fdes, terms, leftover) = scan_eh_frame(raw_data);
            let ends_with_terminator = ends_with_eh_terminator(raw_data);
            tracing::trace!(
                obj = %path,
                input_size = raw_data.len(),
                sh_size = size,
                cies,
                fdes,
                terms,
                leftover,
                ends_with_terminator,
                "parse .eh_frame contribution"
            );
            if ends_with_terminator {
                data_handle.len = data_handle.len.saturating_sub(4);
                size = size.saturating_sub(4);
            }
        }

        let isec = InputSection {
            index: idx,
            name,
            kind,
            sh_type,
            data: data_handle,
            align: section.align(),
            size,
            flags: sh_flags,
            relocs,
        };
        section_map.insert(idx.0, sections.len());
        sections.push(isec);
    }

    let symbol_count = elf.symbols().count();
    let mut symbols = Vec::with_capacity(symbol_count);
    let mut symbol_map = FxHashMap::default();
    symbol_map.reserve(symbol_count);
    let elf_symtab = elf.elf_symbol_table();

    for sym in elf.symbols() {
        let idx = sym.index();
        let name = Name::from_slice(sym.name_bytes().unwrap_or(b""));
        let binding = if sym.scope() == object::SymbolScope::Compilation {
            Binding::Local
        } else if sym.is_weak() {
            Binding::Weak
        } else {
            Binding::Global
        };
        let raw = elf_symtab.symbol(idx).ok();
        let st_type = raw.map(|s| s.st_type()).unwrap_or(elf::STT_NOTYPE);
        let visibility = raw.map(|s| s.st_visibility()).unwrap_or(elf::STV_DEFAULT);
        let is_ifunc = st_type == elf::STT_GNU_IFUNC;
        let isym = InputSymbol {
            index: idx,
            name,
            binding,
            is_undefined: sym.is_undefined(),
            is_common: sym.is_common(),
            is_ifunc,
            st_type,
            visibility,
            section: sym.section_index(),
            value: sym.address(),
            size: sym.size(),
        };
        symbol_map.insert(idx.0, symbols.len());
        symbols.push(isym);
    }

    let comdat_groups = parse_comdat_groups(&elf);

    Ok(InputObject {
        path,
        sections,
        symbols,
        section_map,
        symbol_map,
        comdat_groups,
    })
}
