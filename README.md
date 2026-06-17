# peony

An experimental ELF linker written in Rust, targeting x86-64 Linux. Peony is
designed to be fast, incremental, and drop-in compatible with the standard
`ld`/`cc` command line so it can be used directly as the linker for `rustc` and
`gcc`-based toolchains.

## Features

- **ELF x86-64** — produces `ET_EXEC`, `ET_DYN` (PIE), and shared-object
  (`-shared`) outputs
- **Shared libraries** — full `ET_DYN` shared object support: `.dynsym`,
  `.plt`, `.got.plt`, `DT_SONAME`, `--version-script` for cdylib exports
- **TLS** — General-Dynamic, Local-Dynamic, and Initial-Exec TLS models for
  both executables and shared objects
- **GOT / PLT** — synthesised `.got`, `.got.plt`, `.plt`, and `.plt.got`
  sections; IFUNC (`R_X86_64_IRELATIVE`) support
- **Dynamic relocations** — `R_X86_64_RELATIVE`, `GLOB_DAT`, `JUMP_SLOT`,
  `DTPMOD64 / DTPOFF64 / TPOFF64`
- **`--gc-sections`** — mark-and-sweep dead-section elimination rooted at the
  entry point and (for shared objects) all exported symbols
- **COMDAT deduplication** — eliminates duplicate C++ inline/template sections
  across translation units
- **Incremental linking** (`--incremental`) — fingerprint-based cache that
  skips re-linking when inputs are unchanged
- **Linker-synthesised symbols** — `_end`, `_edata`, `__bss_start`,
  `_GLOBAL_OFFSET_TABLE_`, `__executable_start`, `__dso_handle`, etc.
- **`--defsym`** — define absolute symbols on the command line
- **`--build-id`** — emit a `.note.gnu.build-id` section
- **Parallel** — work-stealing thread pool ([ws-deque]) for parallel section
  scanning and layout
- **`ld`-compatible CLI** — unknown flags are silently ignored so `cc`/`rustc`
  can invoke peony as a drop-in replacement

[ws-deque]: https://github.com/coleleavitt/ws-deque

## Crate layout

| Crate | Role |
|---|---|
| `peony` | Driver binary — CLI parsing, top-level link pipeline |
| `peony-object` | ELF parsing: `InputObject`, `InputSection`, `InputSymbol`, shared-object metadata |
| `peony-symbols` | Global symbol table — resolution, COMDAT, weak/import handling |
| `peony-layout` | Section/segment layout, address assignment, GC, TLS block placement |
| `peony-reloc` | Relocation scanning and application (static + dynamic) |
| `peony-emit` | ELF binary serialisation to disk |
| `peony-cache` | Incremental cache — fingerprinting, manifest read/write |

## Building

```sh
cargo build --release
```

Requires a recent nightly or stable Rust (1.73+).

## Usage

Peony is invoked like `ld`:

```sh
peony -o output input.o [input2.o ...] [-L dir] [-l lib] [flags]
```

### Key flags

| Flag | Description |
|---|---|
| `-o FILE` | Output file (default `a.out`) |
| `-e SYM` | Entry symbol (default `_start`) |
| `-L DIR` | Add library search directory |
| `-l NAME` | Link library `libNAME.{a,so}` |
| `--gc-sections` | Dead-strip unreachable sections |
| `--incremental` | Enable incremental link cache |
| `--build-id` | Emit `.note.gnu.build-id` |
| `-pie` / `-no-pie` | Position-independent executable |
| `-shared` | Produce a shared object (`ET_DYN`) |
| `-soname NAME` | Set `DT_SONAME` |
| `--version-script FILE` | Export/localise symbols per version script |
| `--defsym SYM=VAL` | Define an absolute symbol |
| `--threads N` | Worker thread count (0 = auto) |
| `-s` / `--strip-all` | Strip symbol table and debug sections |
| `-S` / `--strip-debug` | Strip debug sections but keep `.symtab` |

### Invoking from rustc

```sh
RUSTFLAGS="-C linker=/path/to/peony" cargo build
```

Or set `linker = "/path/to/peony"` in `.cargo/config.toml`.

## Testing

The test suite links real object files and compares output against reference
linkers (mold corpus, real-C objects, shared-library tests, TLS, relocations):

```sh
cargo test
```

## License

Licensed under either of [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
