use std::os::raw::{c_char, c_int, c_ulonglong, c_void};

pub(super) const LDPS_OK: c_int = 0;
pub(super) const LDPS_ERR: c_int = 3;

pub(super) const LDPT_NULL: c_int = 0;
pub(super) const LDPT_LINKER_OUTPUT: c_int = 3;
pub(super) const LDPT_REGISTER_CLAIM_FILE_HOOK: c_int = 5;
pub(super) const LDPT_ADD_SYMBOLS: c_int = 8;
pub(super) const LDPT_GET_INPUT_FILE: c_int = 12;
pub(super) const LDPT_RELEASE_INPUT_FILE: c_int = 13;
pub(super) const LDPT_OUTPUT_NAME: c_int = 15;
pub(super) const LDPT_GET_VIEW: c_int = 18;
pub(super) const LDPT_ADD_SYMBOLS_V2: c_int = 33;
pub(super) const LDPT_GET_API_VERSION: c_int = 34;

pub(super) const LDPO_EXEC: c_int = 1;
pub(super) const LAPI_V1: c_int = 1;
pub(super) const RTLD_NOW: c_int = 2;
pub(super) const RTLD_LOCAL: c_int = 0;

pub(super) type PluginStatus = c_int;
pub(super) type ClaimFileHook =
    unsafe extern "C" fn(file: *const PluginInputFile, claimed: *mut c_int) -> PluginStatus;
pub(super) type OnloadFn = unsafe extern "C" fn(tv: *mut PluginTransfer) -> PluginStatus;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct PluginInputFile {
    pub(super) name: *const c_char,
    pub(super) fd: c_int,
    pub(super) offset: c_ulonglong,
    pub(super) filesize: c_ulonglong,
    pub(super) handle: *mut c_void,
}

#[repr(C)]
pub(super) struct PluginSymbol {
    pub(super) name: *const c_char,
    pub(super) version: *const c_char,
    pub(super) def: u8,
    pub(super) symbol_type: u8,
    pub(super) section_kind: u8,
    pub(super) padding: u8,
    pub(super) visibility: c_int,
    pub(super) size: c_ulonglong,
    pub(super) comdat_key: *const c_char,
    pub(super) resolution: c_int,
}

const _: () = {
    assert!(std::mem::size_of::<PluginSymbol>() == 48);
};

#[repr(C)]
union PluginTransferValue {
    val: c_int,
    ptr: *mut c_void,
}

#[repr(C)]
pub(super) struct PluginTransfer {
    tag: c_int,
    value: PluginTransferValue,
}

impl PluginTransfer {
    pub(super) fn int(tag: c_int, val: c_int) -> Self {
        Self {
            tag,
            value: PluginTransferValue { val },
        }
    }

    pub(super) fn ptr<T>(tag: c_int, ptr: *const T) -> Self {
        Self {
            tag,
            value: PluginTransferValue {
                ptr: ptr.cast::<c_void>().cast_mut(),
            },
        }
    }
}

#[link(name = "dl")]
unsafe extern "C" {
    pub(super) fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    pub(super) fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    pub(super) fn dlclose(handle: *mut c_void) -> c_int;
    pub(super) fn dlerror() -> *const c_char;
}
