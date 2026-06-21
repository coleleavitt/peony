use std::ffi::CString;
use std::fs::File;
use std::os::raw::{c_int, c_ulonglong, c_void};
use std::os::unix::io::AsRawFd;
use std::path::Path;

use anyhow::{Context, Result};

use super::abi::PluginInputFile;
use super::callbacks;
use super::loader::{DlHandle, path_to_cstring};

pub(super) struct LtoSymbol {
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) def: u8,
    pub(super) symbol_type: u8,
    pub(super) visibility: c_int,
    pub(super) size: c_ulonglong,
    pub(super) comdat_key: Option<String>,
}

impl LtoSymbol {
    pub(super) fn new(
        name: String,
        version: Option<String>,
        def: u8,
        symbol_type: u8,
        visibility: c_int,
        size: c_ulonglong,
        comdat_key: Option<String>,
    ) -> Self {
        Self {
            name,
            version,
            def,
            symbol_type,
            visibility,
            size,
            comdat_key,
        }
    }

    pub(super) fn display_name(&self) -> String {
        match &self.version {
            Some(version) => format!("{}@@{}", self.name, version),
            None => self.name.clone(),
        }
    }
}

pub(super) struct LtoInput {
    name: CString,
    file: File,
    pub(super) bytes: Vec<u8>,
}

impl LtoInput {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening `{}`", path.display()))?;
        let bytes = std::fs::read(path).with_context(|| format!("reading `{}`", path.display()))?;
        Ok(Self {
            name: path_to_cstring(path)?,
            file,
            bytes,
        })
    }

    pub(super) fn plugin_file(&self) -> Result<PluginInputFile> {
        Ok(PluginInputFile {
            name: self.name.as_ptr(),
            fd: self.file.as_raw_fd(),
            offset: 0,
            filesize: c_ulonglong::try_from(self.bytes.len())
                .context("LTO input is too large for the plugin ABI")?,
            handle: std::ptr::from_ref(self).cast_mut().cast::<c_void>(),
        })
    }
}

pub(super) fn claim_symbols(
    plugin_path: &Path,
    output: &Path,
    input: &Path,
) -> Result<Vec<LtoSymbol>> {
    callbacks::reset_state()?;

    let library = DlHandle::open(plugin_path)?;
    let onload = library.onload()?;
    let output_name = path_to_cstring(output)?;
    let mut transfer = callbacks::transfer_vector(&output_name);

    // SAFETY: Category 8 - FFI boundary. `transfer` is a NUL-terminated gold
    // transfer vector whose callback pointers refer to `extern "C"` functions
    // with ABI-compatible signatures and live for the onload call.
    callbacks::check_status(unsafe { onload(transfer.as_mut_ptr()) }, "plugin onload")?;
    let claim_file_hook = callbacks::claim_file_hook()?;

    let lto_input = LtoInput::open(input)?;
    let plugin_file = lto_input.plugin_file()?;
    let mut claimed = 0;
    // SAFETY: Category 8 - FFI boundary. `plugin_file` points at `lto_input`,
    // whose CString, file descriptor, and byte buffer outlive this synchronous
    // claim_file call; `claimed` is valid writable out-parameter storage.
    callbacks::check_status(
        unsafe { claim_file_hook(&plugin_file, std::ptr::addr_of_mut!(claimed)) },
        "plugin claim_file",
    )?;
    if claimed == 0 {
        anyhow::bail!("LTO plugin did not claim `{}`", input.display());
    }

    callbacks::take_symbols()
}
