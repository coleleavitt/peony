use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::args::Args;

#[path = "lto.rs"]
mod lto;

const LTO_DUMP_ENV: &str = "PEONY_LTO_DUMP_SYMBOLS";

/// GCC passes `-plugin .../liblto_plugin.so` on many links. Most of those links
/// still contain normal ELF objects and should run through Peony. If an input is
/// an actual GCC/LLVM LTO object, though, Peony cannot run the plugin itself, so
/// hand the original `ld` argv to GNU ld.bfd and let the real plugin produce
/// native code.
pub(crate) fn maybe_handoff_lto_plugin(args: &Args) -> Result<bool> {
    if std::env::var_os(LTO_DUMP_ENV).is_some() {
        let input = args
            .inputs
            .iter()
            .find(|path| input_looks_like_lto(path))
            .ok_or_else(|| anyhow::anyhow!("{LTO_DUMP_ENV}=1 but no LTO input was detected"))?;
        lto::dump_symbols(args.plugin.as_deref(), &args.output, input)?;
        return Ok(true);
    }
    if let Some(plugin) = &args.plugin {
        let has_lto_input = args.inputs.iter().any(|p| input_looks_like_lto(p));
        if has_lto_input {
            let linker =
                system_gnu_linker().ok_or_else(|| anyhow::anyhow!("cannot find GNU ld for LTO"))?;
            tracing::info!(
                plugin = %plugin.display(),
                linker = %linker.display(),
                "handing LTO link to GNU ld"
            );
            let status = std::process::Command::new(&linker)
                .args(&args.raw_args)
                .status()
                .with_context(|| format!("running `{}` for LTO handoff", linker.display()))?;
            if status.success() {
                return Ok(true);
            }
            std::process::exit(status.code().unwrap_or(1));
        }
        tracing::info!(
            plugin = %plugin.display(),
            "plugin present but no LTO input detected; linking objects directly"
        );
    }
    Ok(false)
}

fn input_looks_like_lto(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes.starts_with(b"BC\xc0\xde")
        || bytes.starts_with(&[0xde, 0xc0, 0x17, 0x0b])
        || bytes.windows(b".gnu.lto_".len()).any(|w| w == b".gnu.lto_")
        || bytes.windows(b"__gnu_lto".len()).any(|w| w == b"__gnu_lto")
}

fn system_gnu_linker() -> Option<PathBuf> {
    ["/usr/bin/ld.bfd", "/usr/bin/ld"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

pub(crate) fn reject_unsupported_flags(args: &Args) -> Result<()> {
    if args.unsupported_flags.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "unsupported output-affecting linker flag(s): {}",
        args.unsupported_flags.join(", ")
    );
}

pub(crate) fn maybe_handoff_relocatable(args: &Args) -> Result<bool> {
    if !args.relocatable {
        return Ok(false);
    }
    let linker =
        system_gnu_linker().ok_or_else(|| anyhow::anyhow!("cannot find GNU ld for -r link"))?;
    tracing::info!(
        linker = %linker.display(),
        "handing relocatable -r link to GNU ld"
    );
    let status = std::process::Command::new(&linker)
        .args(&args.raw_args)
        .status()
        .with_context(|| format!("running `{}` for -r handoff", linker.display()))?;
    if status.success() {
        return Ok(true);
    }
    std::process::exit(status.code().unwrap_or(1));
}
