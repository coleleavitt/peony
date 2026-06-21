use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;

use anyhow::{Context, Result};

use super::abi::{OnloadFn, RTLD_LOCAL, RTLD_NOW, dlclose, dlerror, dlopen, dlsym};

pub(super) struct DlHandle {
    handle: NonNull<std::ffi::c_void>,
}

impl DlHandle {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let c_path = path_to_cstring(path)?;
        // SAFETY: Category 8 - FFI boundary. `c_path` is a NUL-terminated C
        // string that lives for the call, and the flags are the POSIX dlopen ABI.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        let Some(handle) = NonNull::new(handle) else {
            anyhow::bail!(
                "could not open LTO plugin `{}`: {}",
                path.display(),
                dlerror_message()
            );
        };
        Ok(Self { handle })
    }

    pub(super) fn onload(&self) -> Result<OnloadFn> {
        let name = c"onload";
        // SAFETY: Category 8 - FFI boundary. `self.handle` came from a
        // successful dlopen and `name` is a valid C string for the dlsym call.
        let symbol = unsafe { dlsym(self.handle.as_ptr(), name.as_ptr()) };
        let Some(symbol) = NonNull::new(symbol) else {
            anyhow::bail!("could not find plugin `onload`: {}", dlerror_message());
        };
        // SAFETY: Category 8 - FFI boundary. The gold plugin ABI defines
        // `onload` with the `OnloadFn` signature, and the symbol was resolved
        // from the plugin shared object under that exact name.
        Ok(unsafe { std::mem::transmute::<*mut std::ffi::c_void, OnloadFn>(symbol.as_ptr()) })
    }
}

impl Drop for DlHandle {
    fn drop(&mut self) {
        // SAFETY: Category 8 - FFI boundary. `handle` is the non-null handle
        // returned by dlopen and is closed exactly once by this RAII owner.
        unsafe {
            dlclose(self.handle.as_ptr());
        }
    }
}

pub(super) fn path_to_cstring(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("path contains an interior NUL: `{}`", path.display()))
}

fn dlerror_message() -> String {
    // SAFETY: Category 8 - FFI boundary. dlerror returns either null or a
    // process-local NUL-terminated diagnostic string owned by the loader.
    let err = unsafe { dlerror() };
    if err.is_null() {
        return "unknown loader error".to_string();
    }
    // SAFETY: Category 8 - FFI boundary. Non-null dlerror output is a
    // NUL-terminated diagnostic string owned by the dynamic loader.
    unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned()
}
