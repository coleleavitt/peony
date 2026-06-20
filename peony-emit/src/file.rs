use std::fs::OpenOptions;
use std::path::Path;

use memmap2::MmapMut;

pub(crate) fn open_output_map(
    output_path: &Path,
    file_size: u64,
    can_overwrite: bool,
) -> std::io::Result<MmapMut> {
    let file = if can_overwrite {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(output_path)?
    } else {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)?;
        f.set_len(file_size)?;
        f
    };
    // SAFETY: we hold the file open exclusively for the duration of the map.
    let mut mmap = unsafe { MmapMut::map_mut(&file) }?;
    if !can_overwrite {
        mmap.iter_mut().for_each(|b| *b = 0);
    }
    Ok(mmap)
}

pub(crate) fn chmod_executable(output_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(output_path) {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            if let Err(e) = std::fs::set_permissions(output_path, perm) {
                tracing::warn!("could not chmod output: {e}");
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = output_path;
    }
}
