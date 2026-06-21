use peony_symbols::SymbolTable;

/// Standard linker-provided symbols (PROVIDE semantics: only define a name an
/// input *referenced but left undefined*; never override a real definition).
const LINKER_SYMS: &[&str] = &[
    "_GLOBAL_OFFSET_TABLE_",
    "__executable_start",
    "__ehdr_start",
    "__bss_start",
    "_edata",
    "edata",
    "_end",
    "end",
    // `__dso_handle` identifies this DSO for `__cxa_atexit`/`__cxa_finalize`.
    // The runtime only uses its ADDRESS as an opaque handle, so the image base
    // is a valid, stable definition. crtbegin normally provides it, but when
    // linking without crt (e.g. a cdylib) `libc_nonshared.a` references it
    // undefined, so the linker must synthesise it.
    "__dso_handle",
];

#[derive(Debug, Clone)]
pub(crate) enum ProvidedSymbol {
    Fixed(&'static str),
    SectionStart { name: String, section: String },
    SectionStop { name: String, section: String },
}

impl ProvidedSymbol {
    fn name(&self) -> &str {
        match self {
            ProvidedSymbol::Fixed(name) => name,
            ProvidedSymbol::SectionStart { name, .. }
            | ProvidedSymbol::SectionStop { name, .. } => name,
        }
    }
}

fn linker_symbol_addr(name: &str, layout: &peony_layout::Layout) -> u64 {
    match name {
        "_GLOBAL_OFFSET_TABLE_" => layout.got_base,
        "__executable_start" | "__ehdr_start" | "__dso_handle" => layout.image_base,
        "__bss_start" => layout.bss_start,
        "_edata" | "edata" => layout.edata,
        "_end" | "end" => layout.end,
        _ => 0,
    }
}

fn section_bound_addr(section: &str, layout: &peony_layout::Layout, stop: bool) -> u64 {
    layout
        .output_sections
        .iter()
        .find(|out| out.name == section || out.name.strip_prefix('.') == Some(section))
        .map(|out| {
            if stop {
                out.sh_addr + out.sh_size
            } else {
                out.sh_addr
            }
        })
        .unwrap_or(0)
}

fn c_identifier_suffix(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn start_stop_symbol(name: &[u8]) -> Option<ProvidedSymbol> {
    let name = std::str::from_utf8(name).ok()?;
    if let Some(section) = name.strip_prefix("__start_")
        && c_identifier_suffix(section)
    {
        return Some(ProvidedSymbol::SectionStart {
            name: name.to_string(),
            section: section.to_string(),
        });
    }
    if let Some(section) = name.strip_prefix("__stop_")
        && c_identifier_suffix(section)
    {
        return Some(ProvidedSymbol::SectionStop {
            name: name.to_string(),
            section: section.to_string(),
        });
    }
    None
}

/// Pre-define well-known linker-synthesized symbols (e.g. `_end`, `_start`)
/// as absolute symbols at address 0; their real addresses are patched by the
/// layout pass. Returns the list of names registered.
pub(crate) fn predefine_linker_symbols(symbols: &mut SymbolTable) -> Vec<ProvidedSymbol> {
    let mut provided = Vec::new();
    for &name in LINKER_SYMS {
        if let Some(r) = symbols.lookup(name.as_bytes())
            && r.defined_in.is_none()
        {
            symbols.define_absolute(name.as_bytes(), 0);
            provided.push(ProvidedSymbol::Fixed(name));
        }
    }
    let dynamic: Vec<ProvidedSymbol> = symbols
        .iter()
        .filter(|(_, r)| r.defined_in.is_none())
        .filter_map(|(name, _)| start_stop_symbol(name))
        .collect();
    for sym in dynamic {
        symbols.define_absolute(sym.name().as_bytes(), 0);
        provided.push(sym);
    }
    provided
}

/// Fill in the real addresses of the provided linker symbols after layout.
pub(crate) fn set_linker_addresses(
    symbols: &mut SymbolTable,
    layout: &peony_layout::Layout,
    provided: &[ProvidedSymbol],
) {
    for sym in provided {
        let addr = match sym {
            ProvidedSymbol::Fixed(name) => linker_symbol_addr(name, layout),
            ProvidedSymbol::SectionStart { section, .. } => {
                section_bound_addr(section, layout, false)
            }
            ProvidedSymbol::SectionStop { section, .. } => {
                section_bound_addr(section, layout, true)
            }
        };
        if let Some(r) = symbols.lookup_mut(sym.name().as_bytes()) {
            r.value = addr;
            r.virtual_address = addr;
        }
    }
}
