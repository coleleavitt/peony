use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::Result;

use super::abi::{
    ClaimFileHook,
    LAPI_V1,
    LDPO_EXEC,
    LDPS_ERR,
    LDPS_OK,
    LDPT_ADD_SYMBOLS,
    LDPT_ADD_SYMBOLS_V2,
    LDPT_GET_API_VERSION,
    LDPT_GET_INPUT_FILE,
    LDPT_GET_VIEW,
    LDPT_LINKER_OUTPUT,
    LDPT_NULL,
    LDPT_OUTPUT_NAME,
    LDPT_REGISTER_CLAIM_FILE_HOOK,
    LDPT_RELEASE_INPUT_FILE,
    PluginInputFile,
    PluginStatus,
    PluginSymbol,
    PluginTransfer,
};
use super::session::{LtoInput, LtoSymbol};

#[derive(Default)]
struct PluginState {
    claim_file_hook: Option<ClaimFileHook>,
    symbols: Vec<LtoSymbol>,
}

static PLUGIN_STATE: OnceLock<Mutex<PluginState>> = OnceLock::new();

pub(super) fn transfer_vector(output_name: &CString) -> Vec<PluginTransfer> {
    vec![
        PluginTransfer::int(LDPT_LINKER_OUTPUT, LDPO_EXEC),
        PluginTransfer::ptr(
            LDPT_REGISTER_CLAIM_FILE_HOOK,
            register_claim_file_hook as *const (),
        ),
        PluginTransfer::ptr(LDPT_ADD_SYMBOLS, add_symbols as *const ()),
        PluginTransfer::ptr(LDPT_GET_INPUT_FILE, get_input_file as *const ()),
        PluginTransfer::ptr(LDPT_RELEASE_INPUT_FILE, release_input_file as *const ()),
        PluginTransfer::ptr(LDPT_OUTPUT_NAME, output_name.as_ptr()),
        PluginTransfer::ptr(LDPT_GET_VIEW, get_view as *const ()),
        PluginTransfer::ptr(LDPT_ADD_SYMBOLS_V2, add_symbols as *const ()),
        PluginTransfer::ptr(LDPT_GET_API_VERSION, get_api_version as *const ()),
        PluginTransfer::int(LDPT_NULL, 0),
    ]
}

pub(super) fn reset_state() -> Result<()> {
    let mut state = lock_state()?;
    *state = PluginState::default();
    Ok(())
}

pub(super) fn claim_file_hook() -> Result<ClaimFileHook> {
    lock_state()?
        .claim_file_hook
        .ok_or_else(|| anyhow::anyhow!("LTO plugin did not register claim_file hook"))
}

pub(super) fn take_symbols() -> Result<Vec<LtoSymbol>> {
    let mut state = lock_state()?;
    Ok(std::mem::take(&mut state.symbols))
}

pub(super) fn check_status(status: PluginStatus, operation: &str) -> Result<()> {
    if status == LDPS_OK {
        return Ok(());
    }
    anyhow::bail!("{operation} returned plugin status {status}")
}

fn lock_state() -> Result<MutexGuard<'static, PluginState>> {
    state()
        .lock()
        .map_err(|_| anyhow::anyhow!("LTO plugin state mutex is poisoned"))
}

fn state() -> &'static Mutex<PluginState> {
    PLUGIN_STATE.get_or_init(|| Mutex::new(PluginState::default()))
}

unsafe extern "C" fn register_claim_file_hook(hook: Option<ClaimFileHook>) -> PluginStatus {
    let Ok(mut state) = state().lock() else {
        return LDPS_ERR;
    };
    state.claim_file_hook = hook;
    LDPS_OK
}

unsafe extern "C" fn add_symbols(
    _handle: *mut c_void,
    nsyms: c_int,
    symbols: *const PluginSymbol,
) -> PluginStatus {
    let Some(count) = usize::try_from(nsyms).ok() else {
        return LDPS_ERR;
    };
    let plugin_symbols = if count == 0 {
        &[]
    } else if symbols.is_null() {
        return LDPS_ERR;
    } else {
        // SAFETY: Category 8/10 - FFI boundary and bounds. The plugin-provided
        // pointer covers `count` contiguous PluginSymbol entries for this
        // callback; every entry is copied before control returns to the plugin.
        unsafe { std::slice::from_raw_parts(symbols, count) }
    };
    let Ok(mut state) = state().lock() else {
        return LDPS_ERR;
    };
    state.symbols = plugin_symbols.iter().map(lto_symbol_from_plugin).collect();
    LDPS_OK
}

unsafe extern "C" fn get_view(handle: *const c_void, view: *mut *const c_void) -> PluginStatus {
    if handle.is_null() || view.is_null() {
        return LDPS_ERR;
    }
    // SAFETY: Category 8 - FFI boundary. `handle` is the LtoInput pointer that
    // this module stored in PluginInputFile for the active synchronous claim.
    let input = unsafe { &*handle.cast::<LtoInput>() };
    // SAFETY: Category 8 - FFI boundary. `view` is a plugin-provided writable
    // out pointer, and `input.bytes` outlives the claim_file call using it.
    unsafe {
        view.write(input.bytes.as_ptr().cast::<c_void>());
    }
    LDPS_OK
}

unsafe extern "C" fn get_input_file(
    handle: *const c_void,
    file: *mut PluginInputFile,
) -> PluginStatus {
    if handle.is_null() || file.is_null() {
        return LDPS_ERR;
    }
    // SAFETY: Category 8 - FFI boundary. `handle` is the LtoInput pointer that
    // this module stored in PluginInputFile for the active synchronous claim.
    let input = unsafe { &*handle.cast::<LtoInput>() };
    let Ok(plugin_file) = input.plugin_file() else {
        return LDPS_ERR;
    };
    // SAFETY: Category 8 - FFI boundary. `file` is plugin-provided writable
    // storage for the PluginInputFile value.
    unsafe {
        file.write(plugin_file);
    }
    LDPS_OK
}

unsafe extern "C" fn release_input_file(_handle: *const c_void) -> PluginStatus {
    LDPS_OK
}

unsafe extern "C" fn get_api_version(
    _plugin_identifier: *const c_char,
    _plugin_version: c_uint,
    _minimal_api_supported: c_int,
    maximal_api_supported: c_int,
    linker_identifier: *mut *const c_char,
    linker_version: *mut *const c_char,
) -> c_int {
    static LINKER_IDENTIFIER: &[u8] = b"peony\0";
    static LINKER_VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

    if !linker_identifier.is_null() {
        // SAFETY: Category 8 - FFI boundary. The plugin supplied writable
        // pointer storage; both static strings are NUL-terminated.
        unsafe {
            linker_identifier.write(LINKER_IDENTIFIER.as_ptr().cast::<c_char>());
        }
    }
    if !linker_version.is_null() {
        // SAFETY: Category 8 - FFI boundary. The plugin supplied writable
        // pointer storage; both static strings are NUL-terminated.
        unsafe {
            linker_version.write(LINKER_VERSION.as_ptr().cast::<c_char>());
        }
    }
    if maximal_api_supported >= LAPI_V1 {
        LAPI_V1
    } else {
        0
    }
}

fn lto_symbol_from_plugin(symbol: &PluginSymbol) -> LtoSymbol {
    let name = match copy_c_string(symbol.name) {
        Some(name) => name,
        None => "<null>".to_string(),
    };
    LtoSymbol::new(
        name,
        copy_c_string(symbol.version),
        symbol.def,
        symbol.symbol_type,
        symbol.visibility,
        symbol.size,
        copy_c_string(symbol.comdat_key),
    )
}

fn copy_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Category 8 - FFI boundary. Gold plugin strings are
    // NUL-terminated and valid for the duration of the callback; this function
    // copies the bytes immediately into owned Rust memory.
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}
