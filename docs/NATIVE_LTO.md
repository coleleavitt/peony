# Native LTO — peony as the linker-plugin host

**Status:** design / not started. Companion: [`SHARED_INPUT_POOL.md`](SHARED_INPUT_POOL.md).

## Goal

Today an `-flto` / `-Clinker-plugin-lto` link drops out of peony entirely:
`maybe_handoff_lto_plugin` (`peony/src/handoff.rs`) sniffs the inputs
(`input_looks_like_lto`: LLVM `BC\xc0\xde`, the bitcode wrapper, or GCC
`.gnu.lto_` / `__gnu_lto`) and, on a hit, re-execs the *entire* argv through
`/usr/bin/ld.bfd`. peony does nothing for that link — no incremental, no daemon,
no byte-identity guarantee.

The goal is to make peony **the LTO plugin host itself**: `dlopen` the compiler's
plugin (`LLVMgold.so`, or GCC's `liblto_plugin.so`), drive the claim → resolve →
codegen → reingest dance, and feed the resulting *native* objects back through
peony's normal layout/reloc/emit pipeline. The handoff stays only as the
fallback for cases we choose not to support.

**Reference implementation:** `research/mold/src/lto-unix.cc` (+ `lto.h`) is a
complete, readable plugin host and the model for this work. The plugin API
itself is the gold/GNU `ld_plugin` ABI; mold's `lto.h` is a clean transcription
of it. Read both before starting — the design below maps mold's structure onto
peony's crates.

## Why this is hard (what's different from a normal link)

1. **IR objects have no sections.** A bitcode `.o` carries only a symbol table,
   surfaced through the plugin — there is nothing for `peony-layout` to place
   until *after* codegen. IR inputs are parsed "symbols-only" via the plugin,
   not via `peony-object`'s ELF parser.
2. **Codegen happens mid-link.** The plugin compiles all the IR together and
   hands back *new* native ELF objects whose sections and relocations must then
   flow through the rest of the pipeline. So the plugin step has to run **after**
   global symbol resolution (resolution decides which IR defs are kept vs.
   internalized) but **before** reloc-scan / layout.
3. **The API is inverted and stateful.** Callbacks don't return values — they
   "return" by calling *other* callbacks (`claim_file` → `add_symbols`;
   `all_symbols_read` → `add_input_file`). peony must expose ~25 `extern "C"`
   trampolines plus a transfer vector, and thread global state through them
   (the plugin is single-threaded by design — `lto-unix.cc:101`).
4. **Determinism vs. byte-identity.** LLVM codegen is deterministic given fixed
   inputs and thread count, but temp-file paths and ThinLTO parallelism mean the
   *whole* LTO link is not trivially reproducible run-to-run. peony's
   byte-identity gate therefore applies to the **post-codegen native link given
   identical codegen output**, not across codegen re-runs. Say this honestly in
   the docs/tests; don't claim more.

## Where it lives

Mirror mold: one orchestration+FFI module, `peony/src/lto.rs`, alongside the
existing `handoff.rs` in the driver. It owns the `dlopen`, the transfer vector,
the `extern "C"` trampolines, and the IR-symbol ingestion; the actual symbol
bookkeeping calls into `peony-symbols`, and the native objects it produces are
parsed by `peony-object` exactly like any other input. Promote to a `peony-lto`
crate only if it outgrows a module. **Do not** leak FFI types past the module
boundary — convert `PluginSymbol` → peony's own symbol model at the edge.

## The plugin protocol, mapped to peony

The linker is the *host*: it `dlopen`s the plugin, calls `onload(transfer_vector)`
once, and the plugin registers its three hooks back. Then the host calls
`claim_file` per IR input and `all_symbols_read` once at the end. (mold:
`load_lto_plugin` builds the vector at `lto-unix.cc:463`; `onload` at `:508`.)

| plugin callback (host provides) | what peony does |
|---|---|
| `message(level, fmt, …)` | route to `tracing` (info/warn) or bail (error/fatal) |
| `register_{claim_file,all_symbols_read,cleanup}_hook` | stash the three fn ptrs the plugin hands back |
| `add_symbols(handle, n, syms[])` | called *from inside* `claim_file`; copy the `PluginSymbol[]` into the IR object's symbol list |
| `get_symbols_v2/v3(handle, n, syms[])` | called *from inside* `all_symbols_read`; fill each `syms[i].resolution` with an `LDPR_*` code (the crux — see below) |
| `add_input_file(path)` | parse `path` as a normal ELF object (`peony-object`) and append it to `objects`; its defs supersede the IR defs |
| `get_view(handle, *out)` / `get_input_file` / `release_input_file` | hand the plugin the mmap'd bytes of the IR input |
| `get_api_version(…)` | advertise peony + negotiate V1; gate the v3-API requirement here |
| section-iteration / wrap / segment stubs | no-op returning `LDPS_OK` (mold stubs all of these) |

Transfer-vector entries peony *sends* to the plugin (not callbacks):
`LDPT_LINKER_OUTPUT` (EXEC/PIE/DYN — drives whole-program assumptions),
`LDPT_OUTPUT_NAME`, and one `LDPT_OPTION` per `-plugin-opt=…` (O-level, `mcpu`,
`jobs=`, ThinLTO cache dir, …). Terminate with `LDPT_NULL`.

## The resolution mapping (the crux)

`get_symbols` is how peony *teaches the plugin its resolution* so the backend
knows which IR definitions it may inline-and-drop vs. must keep as real symbols.
Get this wrong and LTO either deletes live code or fails to optimize. peony must
reproduce mold's decision tree (`lto-unix.cc:295`), expressed in peony-symbols
terms. For each IR symbol, given the globally-resolved owner:

```
no resolving definition                         -> LDPR_UNDEF
resolved to THIS IR object (it prevails):
    referenced by any non-IR (regular) object   -> LDPR_PREVAILING_DEF          (must keep)
    else if dynamically exported                -> LDPR_PREVAILING_DEF_IRONLY_EXP (v3) / _DEF (v2)
    else                                         -> LDPR_PREVAILING_DEF_IRONLY  (free to internalize)
resolved to a shared object                      -> LDPR_RESOLVED_DYN
resolved to another IR object                    -> RESOLVED_IR (undef here) / PREEMPTED_IR
resolved to a regular object                     -> RESOLVED_EXEC (undef here) / PREEMPTED_REG
```

Two pieces of new symbol state are required to compute this:

- **`defined_in_ir` bit** — set when a symbol's prevailing def came from a
  claimed IR object.
- **`referenced_by_regular_obj` bit** — before `all_symbols_read`, walk every
  *non-IR* object's referenced globals and set this on any IR-defined symbol
  they touch (mold: `run_lto_plugin`, `lto-unix.cc:696`). Also force-set it for
  `--undefined`, `--wrap` partners, and dynamic-list/exported names, so the
  backend can't internalize something the dynamic symbol table needs.

`is_exported` already exists in peony's dynamic-export computation (for
`-shared` / `--export-dynamic`); reuse it.

## Pipeline integration (concrete seams)

All line numbers are `peony/src/main.rs` as of this writing.

1. **`handoff.rs`** — `maybe_handoff_lto_plugin` stops early-exiting when a
   plugin *and* a real plugin host are available; it returns "handle natively."
   Keep the `ld.bfd` re-exec only for the descoped cases (no v3 API, GCC offload,
   `--plugin` absent). Add `args.plugin_opt: Vec<String>` (collect
   `-plugin-opt=` / `--plugin-opt`).

2. **New `lto` phase, inserted between `parse+resolve` (`:278`) and
   `weak-got-ids` (`:310`):**
   1. Partition inputs into IR (`input_looks_like_lto`) and regular.
   2. Parse regular objects normally (already happens).
   3. `dlopen` + `onload` once. For each IR input: build a `PluginInputFile`
      (name, fd, offset, filesize, handle = a fresh peony IR-object), call
      `claim_file`; inside, `add_symbols` populates that object's symbol list.
      Convert each `PluginSymbol` → peony symbol (`to_elf_sym` analogue,
      `lto-unix.cc:513`: `def`→bind/shndx, `symbol_type`→`STT_*`,
      `visibility`→`STV_*`, carry `size` and `comdat_key`).
   4. Run global resolution over IR ∪ regular (existing resolver).
   5. Set `referenced_by_regular_obj` (above).
   6. Call `all_symbols_read`; service `get_symbols_v3` with the resolution
      codes; the backend compiles and calls `add_input_file(path)` one or more
      times. Parse each native `.o` fully and append to `objects`.
   7. Drop the IR objects' *definitions* in favor of the native ones and
      re-resolve so the native defs win (their symbols carry the real
      addresses/sizes; the IR placeholders had none).
   8. Register `cleanup_hook` to run at link end (removes backend temp files).
3. **Everything after is unchanged:** `weak-got-ids` → `reloc-scan` (`:317`) →
   layout (`:628`) → `finalize_symbols` (`:685`) → `emit` (`:846`). The native
   objects are ordinary ELF from here on.

## Phasing

- **P0 — FFI spike.** `dlopen("LLVMgold.so")`, `onload` with a minimal vector,
  `claim_file` one `rustc -Clinker-plugin-lto` bitcode `.o`, print its symbols.
  Proves plugin discovery + the callback inversion end-to-end. No linking.
- **P1 — MVP (LLVM, v3 API, no archives).** Whole-set fat LTO of plain objects:
  ingest IR symbols, resolve, `get_symbols_v3`, codegen, reingest, finish the
  link. **Gate:** a Rust `hello` and a 3-CU program built with
  `-Clinker-plugin-lto` link with peony and *run*; differential symtab vs.
  `ld -plugin LLVMgold.so`.
- **P2 — archives + GCC.** Lazy IR extraction from `.rlib`/`.a`; the
  `get_symbols_v3` "unclaim a non-included member" path (set every resolution to
  `LDPR_PREEMPTED_REG`, return `LDPS_NO_SYMS` — `lto-unix.cc:288`). Add GCC's
  `liblto_plugin.so`. **Require v3** (LLVM any; GCC ≥ 12) — see descopes.
- **P3 — production surface.** ThinLTO (many `add_input_file` results + a module
  cache dir via `-plugin-opt`), `-shared`/`-pie` outputs, COMDAT from IR
  (`comdat_key`), `-plugin-opt` passthrough (O-level, `mcpu`, `jobs`), `--wrap`.
- **P4 — incremental/daemon synergy** (below).

## Risks & descopes

- **Descope the GCC v0/v1 restart hack.** Old plugins (GCC < 12) lack
  `get_symbols_v3`, so a member claimed from an archive can't be "unclaimed" if
  it turns out unused; mold works around this by `execv`-ing *itself* with
  `--:ignore-ir-file=…` flags (`restart_process`, `lto-unix.cc:324`). Skip it
  entirely: negotiate the API in `get_api_version`, and if v3 is unavailable,
  fall back to the existing `ld.bfd` handoff. Large complexity win, negligible
  real-world loss (LLVM always supports v3; modern GCC does too).
- **Descope GCC `-foffload` objects** (GPU `.gnu.offload_lto_` sections,
  `lto-unix.cc:725`) → handoff. Irrelevant to Rust toolchains.
- **Byte-identity scope.** Document that the guarantee is over the native link
  *given* codegen output, not across codegen runs (LLVM temp paths + ThinLTO
  threading). The build-id already hashes the final image with a zeroed
  descriptor, and temp paths never enter the image, so a *fixed* codegen result
  links reproducibly.
- **Keep the handoff as a safety net** for every unsupported branch, so this
  feature can never regress a link that works today.
- **Thread-safety:** serialize `claim_file` (V0 is not reentrant,
  `lto-unix.cc:606`); codegen itself is the plugin's own (possibly parallel)
  business.

## Incremental + daemon synergy (P4)

Layout reuse can't span an LTO codegen change — the whole program is recompiled,
so addresses move. But two real wins remain:

- **Unchanged-IR caching.** If no IR input changed (only a regular `.o`, or
  nothing), the backend's output is identical. Cache the native `.o`s keyed by
  the set of IR fingerprints (or lean on LLVM's ThinLTO module cache dir), then
  the *post-codegen* link still takes peony's existing incremental fast path.
- **Resident plugin.** The daemon (`peony/src/daemon.rs`) keeps the plugin
  `dlopen`'d and the ThinLTO module cache warm across links, so only the modules
  that actually changed get recompiled. That — not full-program fat LTO — is the
  practical "fast LTO relink" story, and it composes with
  [`SHARED_INPUT_POOL.md`](SHARED_INPUT_POOL.md) for the regular inputs.

## Validation

- New `peony/tests/lto.rs`: `rustc -Clinker-plugin-lto` for a `hello`, a
  multi-CU program, and a mixed Rust + C (`-flto`) case; link with peony; assert
  exit codes; differential symtab/section diff vs. `ld -plugin`.
- Archive case: an `.rlib` with IR members, some unused (exercises the unclaim
  path).
- `cargo check --workspace && cargo test --workspace`; confirm non-LTO links and
  benches are untouched (the `lto` phase is skipped when no IR input is present —
  same predicate as `input_looks_like_lto`).
