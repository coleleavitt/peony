use std::path::{Path, PathBuf};

use anyhow::Result;

#[path = "lto/abi.rs"]
mod abi;
#[path = "lto/callbacks.rs"]
mod callbacks;
#[path = "lto/loader.rs"]
mod loader;
#[path = "lto/session.rs"]
mod session;

use session::{LtoSymbol, claim_symbols};

pub(super) fn dump_symbols(plugin: Option<&Path>, output: &Path, input: &Path) -> Result<()> {
    let plugin_path = plugin
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("LLVMgold.so"));
    let symbols = claim_symbols(&plugin_path, output, input)?;
    print_symbols(&plugin_path, input, &symbols);
    Ok(())
}

fn print_symbols(plugin: &Path, input: &Path, symbols: &[LtoSymbol]) {
    println!("plugin: {}", plugin.display());
    println!("input: {}", input.display());
    println!("claimed: yes");
    println!("symbols: {}", symbols.len());
    for symbol in symbols {
        println!(
            "{:<12} {:<10} {:<10} size={:<5} {}{}",
            symbol_def(symbol.def),
            symbol_type(symbol.symbol_type),
            symbol_visibility(symbol.visibility),
            symbol.size,
            symbol.display_name(),
            comdat_suffix(symbol.comdat_key.as_deref())
        );
    }
}

fn symbol_def(value: u8) -> &'static str {
    match value {
        0 => "def",
        1 => "weakdef",
        2 => "undef",
        3 => "weakundef",
        4 => "common",
        _ => "unknown-def",
    }
}

fn symbol_type(value: u8) -> &'static str {
    match value {
        1 => "function",
        2 => "variable",
        _ => "unknown",
    }
}

fn symbol_visibility(value: std::os::raw::c_int) -> &'static str {
    match value {
        0 => "default",
        1 => "protected",
        2 => "internal",
        3 => "hidden",
        _ => "unknown",
    }
}

fn comdat_suffix(key: Option<&str>) -> String {
    match key {
        Some(key) => format!(" comdat={key}"),
        None => String::new(),
    }
}
