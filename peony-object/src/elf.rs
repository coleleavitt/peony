pub const ELFMAG: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const EV_CURRENT: u8 = 1;
pub const ELFOSABI_SYSV: u8 = 0;

pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

pub const EM_X86_64: u16 = 62;

pub const EHDR_SIZE: u64 = 64;
pub const PHDR_SIZE: u64 = 56;
pub const SHDR_SIZE: u64 = 64;
pub const SYM_SIZE: u64 = 24;
pub const RELA_SIZE: u64 = 24;

pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
pub const PT_GNU_STACK: u32 = 0x6474_e551;
pub const PT_GNU_RELRO: u32 = 0x6474_e552;
pub const PT_GNU_PROPERTY: u32 = 0x6474_e553;

pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_HASH: u32 = 5;
pub const SHT_DYNAMIC: u32 = 6;
pub const SHT_NOTE: u32 = 7;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_DYNSYM: u32 = 11;
pub const SHT_INIT_ARRAY: u32 = 14;
pub const SHT_FINI_ARRAY: u32 = 15;
pub const SHT_PREINIT_ARRAY: u32 = 16;
pub const SHT_GNU_HASH: u32 = 0x6fff_fff6;
pub const SHT_GNU_VERDEF: u32 = 0x6fff_fffd;
pub const SHT_GNU_VERNEED: u32 = 0x6fff_fffe;
pub const SHT_GNU_VERSYM: u32 = 0x6fff_ffff;
pub const SHT_SYMTAB_SHNDX: u32 = 18;

pub const NT_GNU_BUILD_ID: u32 = 3;

pub const DT_NULL: i64 = 0;
pub const DT_NEEDED: i64 = 1;
pub const DT_PLTRELSZ: i64 = 2;
pub const DT_PLTGOT: i64 = 3;
pub const DT_HASH: i64 = 4;
pub const DT_PLTREL: i64 = 20;
pub const DT_JMPREL: i64 = 23;
pub const DT_STRTAB: i64 = 5;
pub const DT_SONAME: i64 = 14;
pub const DT_SYMTAB: i64 = 6;
pub const DT_RELA: i64 = 7;
pub const DT_RELASZ: i64 = 8;
pub const DT_RELAENT: i64 = 9;
pub const DT_STRSZ: i64 = 10;
pub const DT_SYMENT: i64 = 11;
pub const DT_INIT: i64 = 12;
pub const DT_FINI: i64 = 13;
pub const DT_RPATH: i64 = 15;
pub const DT_INIT_ARRAY: i64 = 25;
pub const DT_FINI_ARRAY: i64 = 26;
pub const DT_INIT_ARRAYSZ: i64 = 27;
pub const DT_FINI_ARRAYSZ: i64 = 28;
pub const DT_RUNPATH: i64 = 29;
pub const DT_FLAGS: i64 = 30;
pub const DT_RELACOUNT: i64 = 0x6fff_fff9;
pub const DT_GNU_HASH: i64 = 0x6fff_fef5;
pub const DT_FLAGS_1: i64 = 0x6fff_fffb;
pub const DT_VERSYM: i64 = 0x6fff_fff0;
pub const DT_VERNEED: i64 = 0x6fff_fffe;
pub const DT_VERNEEDNUM: i64 = 0x6fff_ffff;
pub const DF_BIND_NOW: u64 = 0x8;
pub const DF_1_NOW: u64 = 0x1;
pub const DF_1_PIE: u64 = 0x0800_0000;

pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_COPY: u32 = 5;
pub const R_X86_64_GLOB_DAT: u32 = 6;
pub const R_X86_64_JUMP_SLOT: u32 = 7;
pub const R_X86_64_RELATIVE: u32 = 8;
pub const R_X86_64_IRELATIVE: u32 = 37;

pub const DEFAULT_INTERP: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

pub const SHF_WRITE: u64 = 0x1;
pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;
pub const SHF_MERGE: u64 = 0x10;
pub const SHF_STRINGS: u64 = 0x20;
pub const SHF_TLS: u64 = 0x400;
pub const SHF_GNU_RETAIN: u64 = 0x0020_0000;

pub const SHN_UNDEF: u16 = 0;
pub const SHN_LORESERVE: u16 = 0xff00;
pub const SHN_ABS: u16 = 0xfff1;
pub const SHN_COMMON: u16 = 0xfff2;
pub const SHN_XINDEX: u16 = 0xffff;

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;
pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_GNU_IFUNC: u8 = 10;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;
pub const STT_TLS: u8 = 6;
pub const STV_DEFAULT: u8 = 0;
pub const STV_INTERNAL: u8 = 1;
pub const STV_HIDDEN: u8 = 2;
pub const STV_PROTECTED: u8 = 3;

#[inline]
pub const fn st_info(bind: u8, typ: u8) -> u8 {
    (bind << 4) | (typ & 0xf)
}
