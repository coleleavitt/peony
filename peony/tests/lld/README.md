This directory is a verbatim reference copy of LLVM lld's ELF test corpus.

- Source: `/home/cole/CLionProjects/forks/llvm-project/lld/test/ELF`
- Upstream commit: `a255c1ed36a1d06f79bd2633ba9f8d900153007c`
- Copied files: 2,385 total, including 339 `Inputs/` fixtures

The files under `ELF/` are intentionally inert fixtures. Peony does not claim
that every lld test passes. `../lld_corpus.rs` checks that the copied corpus is
complete, while Peony-specific `tests/*.rs` files port selected lld behaviors
into executable regressions.
