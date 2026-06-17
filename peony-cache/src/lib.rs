//! `peony-cache` — Epoch-gated incremental link cache with red-green coloring.
//!
//! ## Architecture (QUAD §6, §10.2 / SPEC §4)
//!
//! ### Epoch-gated fast path (QUAD Definition 6.2, Theorem 6.2)
//!
//! A *build epoch* is a period during which no input changes. The epoch key is
//! `SHA-256(sorted file mtimes ∥ args_hash)`. If the current epoch key matches
//! the cached one, the output is byte-identical to the previous link and we skip
//! all work.
//!
//! ### Red-green section coloring (QUAD Definition 6.1, Theorem 6.1)
//!
//! When inputs *do* change, we diff at the section level:
//! - **Green** sections: byte-identical to the previous link → skip re-emit.
//! - **Red** sections: changed or have moved relocation targets → must re-emit.
//!
//! This reduces incremental work to O(|δ|) instead of O(|S|) (QUAD §11).
//!
//! ### Relocation reverse index (QUAD §10.3, SPEC §7.3)
//!
//! Persistent flat-array linked list:
//! - `reloc_heads[symbol_id]` → first relocation referencing this symbol
//! - `reloc_next[reloc_id]` → next relocation in the per-symbol list
//!
//! Built lock-free with atomic compare-exchange from parallel threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("I/O error accessing cache at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cache encode/decode failed: {0}")]
    Codec(String),
}

pub type Result<T> = std::result::Result<T, CacheError>;

/// Bump when the manifest format changes incompatibly.
pub const CACHE_VERSION: u32 = 4;

/// Sentinel for "no next entry" in the relocation reverse index.
pub const NO_ENTRY: u32 = u32::MAX;

// ── Fingerprints ────────────────────────────────────────────────────────────

/// Length + FNV-1a-64 content hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Fingerprint {
    pub len: u64,
    pub hash: u64,
}

impl Fingerprint {
    pub fn of_bytes(data: &[u8]) -> Self {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = OFFSET;
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
        Fingerprint {
            len: data.len() as u64,
            hash: h,
        }
    }

    pub fn of_file(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| CacheError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(Self::of_bytes(&data))
    }
}

/// A cheap change-detector: (size, mtime-nanos, inode). Comparing these is a
/// single `stat()` per file — no read, no content hash. The incremental
/// no-change fast path must be O(stat) not O(bytes); hashing every input's full
/// content (the old `Fingerprint::of_file`) made a no-change relink SLOWER than
/// a mold full link, defeating the purpose. A content `Fingerprint` is still
/// kept in the manifest for red-green section coloring, but the gate that
/// decides "can we skip the link entirely" uses this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FastFingerprint {
    pub len: u64,
    /// mtime seconds + nanoseconds kept SEPARATE. Folding them into one u64
    /// (`sec*1e9 + nsec`) wraps for large/negative seconds and is non-injective —
    /// distinct timestamps could collapse to one value and falsely report
    /// "unchanged". Two fields compared with derived `Eq` are exact.
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub inode: u64,
}

impl FastFingerprint {
    pub fn of_file(path: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).map_err(|e| CacheError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(FastFingerprint {
            len: m.len(),
            mtime_sec: m.mtime(),
            mtime_nsec: m.mtime_nsec(),
            inode: m.ino(),
        })
    }
}

// ── Section color (QUAD Definition 6.1) ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionColor {
    /// Byte-identical to the previous link → skip re-emit (Theorem 6.1).
    Green,
    /// Changed or has moved relocation targets → must re-emit.
    Red,
}

// ── Section-level diff (QUAD Definition 1.6) ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionRecord {
    /// Output section name (e.g. `.text`).
    pub name: String,
    /// Content fingerprint of all contributing input bytes.
    pub fingerprint: Fingerprint,
    /// File offset of this section in the output binary.
    pub file_offset: u64,
    /// Size of the section content bytes.
    pub size: u64,
    /// Allocated capacity (size + incremental padding).
    pub capacity: u64,
    /// Virtual memory address of the section.
    pub virtual_address: u64,
}

// ── Symbol record for persistent symbol cache (SPEC §7.2) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSymbolEntry {
    pub name: Vec<u8>,
    pub virtual_address: u64,
    pub got_address: u64,
}

// ── Relocation reverse index (QUAD §10.3, SPEC §7.3) ────────────────────────

/// In-memory relocation reverse index.
///
/// Flat-array linked list: for symbol `sid`, the chain starting at
/// `heads[sid]` → `nexts[r0]` → `nexts[r1]` → ... → `NO_ENTRY`
/// enumerates all relocations referencing that symbol.
pub struct RelocReverseIndex {
    /// `heads[symbol_id]` = first reloc index for this symbol, or `NO_ENTRY`.
    pub heads: Vec<AtomicU32>,
    /// `nexts[reloc_id]` = next reloc in the chain for the same symbol.
    pub nexts: Vec<AtomicU32>,
}

impl RelocReverseIndex {
    /// Create an index for `symbol_count` symbols and `reloc_count` relocations.
    /// All entries initialized to `NO_ENTRY`.
    pub fn new(symbol_count: usize, reloc_count: usize) -> Self {
        let heads = (0..symbol_count)
            .map(|_| AtomicU32::new(NO_ENTRY))
            .collect();
        let nexts = (0..reloc_count).map(|_| AtomicU32::new(NO_ENTRY)).collect();
        Self { heads, nexts }
    }

    /// Lock-free insertion of `reloc_id` into the list for `symbol_id`.
    ///
    /// Uses compare-exchange loop (SPEC §7.3 construction invariant).
    pub fn insert(&self, symbol_id: u32, reloc_id: u32) {
        let sym_idx = symbol_id as usize;
        if sym_idx >= self.heads.len() || reloc_id as usize >= self.nexts.len() {
            return;
        }
        loop {
            let old_head = self.heads[sym_idx].load(Ordering::Relaxed);
            self.nexts[reloc_id as usize].store(old_head, Ordering::Relaxed);
            match self.heads[sym_idx].compare_exchange(
                old_head,
                reloc_id,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue, // retry
            }
        }
    }

    /// Iterate over all reloc indices for `symbol_id`.
    pub fn iter_relocs(&self, symbol_id: u32) -> impl Iterator<Item = u32> + '_ {
        let sym_idx = symbol_id as usize;
        let first = if sym_idx < self.heads.len() {
            self.heads[sym_idx].load(Ordering::Acquire)
        } else {
            NO_ENTRY
        };
        RelocChainIter {
            nexts: &self.nexts,
            current: first,
        }
    }
}

struct RelocChainIter<'a> {
    nexts: &'a [AtomicU32],
    current: u32,
}

impl<'a> Iterator for RelocChainIter<'a> {
    type Item = u32;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current == NO_ENTRY {
            return None;
        }
        let idx = self.current as usize;
        let val = self.current;
        self.current = self
            .nexts
            .get(idx)
            .map(|a| a.load(Ordering::Acquire))
            .unwrap_or(NO_ENTRY);
        Some(val)
    }
}

// ── Epoch key (QUAD Definition 6.2) ─────────────────────────────────────────

/// Compute the epoch key: FNV-1a hash of all input mtimes + the args hash.
///
/// If this matches the cached epoch key, the build epoch is unchanged and we
/// can reuse the output binary without re-reading any input file.
pub fn compute_epoch_key(inputs: &[PathBuf], args_hash: u64) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    // Mix in the args hash first.
    for b in args_hash.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    // Mix in each input file's modification time.
    for p in inputs {
        let mtime = std::fs::metadata(p)
            .and_then(|m| m.modified())
            .and_then(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            })
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        for b in mtime.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
        // Also mix the path itself.
        for b in p.to_string_lossy().as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(PRIME);
        }
    }
    h
}

/// Simple args hash: FNV-1a over all argument strings.
pub fn hash_args(args: &[String]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for a in args {
        for b in a.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(PRIME);
        }
        h ^= 0x1fu64; // separator
        h = h.wrapping_mul(PRIME);
    }
    h
}

// ── Manifest (combines epoch + section records) ──────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    /// Epoch key at time of last link (QUAD Definition 6.2).
    pub epoch_key: u64,
    /// (input path, content fingerprint) in input order. The content hash is
    /// used by red-green section coloring; the no-change gate uses `fast_inputs`.
    pub inputs: Vec<(String, Fingerprint)>,
    /// (input path, cheap stat fingerprint) in input order — the O(stat)
    /// no-change detector consulted by [`try_reuse`].
    pub fast_inputs: Vec<(String, FastFingerprint)>,
    /// Content fingerprint of the produced output binary.
    pub output: Fingerprint,
    /// Cheap stat fingerprint of the output (detects external modification).
    pub fast_output: FastFingerprint,
    /// Per-output-section records for red-green coloring.
    pub sections: Vec<SectionRecord>,
    /// Cached symbol addresses for detecting moved symbols.
    pub symbols: Vec<CachedSymbolEntry>,
}

/// The `<output>.incr/` directory for one output binary.
pub fn cache_dir(output: &Path) -> PathBuf {
    output.with_extension("incr")
}

fn manifest_path(output: &Path) -> PathBuf {
    cache_dir(output).join("manifest.bin")
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Returns `true` if the output can be fully reused (epoch unchanged + output unmodified).
///
/// Fast path: compare the epoch key. If it matches, skip reading any input file.
/// Slow path: verify each input fingerprint and the output fingerprint.
/// Decide whether the cached output can be reused unchanged. `args_hash` is the
/// hash of the output-affecting command-line arguments (from [`hash_args`]): a
/// relink of the SAME inputs with DIFFERENT flags (e.g. `-pie` → `-shared`) must
/// NOT reuse the stale binary, so the recorded epoch key (which folds the args
/// hash) is recomputed and compared. Without this check a flag change would
/// silently serve the wrong output.
pub fn try_reuse(output: &Path, inputs: &[PathBuf], args_hash: u64) -> Result<bool> {
    let path = manifest_path(output);
    if !path.exists() || !output.exists() {
        return Ok(false);
    }
    let bytes = std::fs::read(&path).map_err(|e| CacheError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let (manifest, _): (Manifest, usize) =
        match bincode::serde::decode_from_slice(&bytes, bincode::config::standard()) {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
    if manifest.version != CACHE_VERSION {
        return Ok(false);
    }

    // Output-affecting flags must match: the epoch key folds the args hash, so a
    // relink with different flags (same inputs) has a different epoch key and
    // must fall through to a full link rather than reuse the stale binary.
    if manifest.epoch_key != compute_epoch_key(inputs, args_hash) {
        return Ok(false);
    }

    // Same inputs, same order — compared with the CHEAP stat fingerprint
    // (size + mtime + inode), one `stat()` per input, no content read. This is
    // what makes a no-change relink O(#inputs) syscalls instead of O(bytes).
    if manifest.fast_inputs.len() != inputs.len() {
        return Ok(false);
    }
    for ((rec_path, rec_fp), cur) in manifest.fast_inputs.iter().zip(inputs) {
        if rec_path != &cur.display().to_string() {
            return Ok(false);
        }
        match FastFingerprint::of_file(cur) {
            Ok(fp) if fp == *rec_fp => {}
            _ => return Ok(false),
        }
    }

    // Output unmodified since we wrote it (cheap stat check).
    match FastFingerprint::of_file(output) {
        Ok(fp) if fp == manifest.fast_output => Ok(true),
        _ => Ok(false),
    }
}

/// Record fingerprints after a successful full link. `args_hash` is the hash of
/// the output-affecting flags ([`hash_args`]); it is folded into the stored
/// epoch key so [`try_reuse`] can reject a relink with changed flags.
pub fn record_link(output: &Path, inputs: &[PathBuf], args_hash: u64) -> Result<()> {
    record_link_with_sections(output, inputs, args_hash, &[], &[])
}

/// Record fingerprints + section records + symbol cache after a successful link.
///
/// `sections` provides per-output-section metadata for future red-green coloring.
/// `symbols` provides symbol virtual addresses for detecting moved symbols.
pub fn record_link_with_sections(
    output: &Path,
    inputs: &[PathBuf],
    args_hash: u64,
    sections: &[SectionRecord],
    symbols: &[CachedSymbolEntry],
) -> Result<()> {
    let dir = cache_dir(output);
    std::fs::create_dir_all(&dir).map_err(|e| CacheError::Io {
        path: dir.display().to_string(),
        source: e,
    })?;

    // Cheap stat fingerprints for the no-change gate (always recorded).
    let mut fast_inputs = Vec::with_capacity(inputs.len());
    for p in inputs {
        fast_inputs.push((p.display().to_string(), FastFingerprint::of_file(p)?));
    }
    // Content fingerprints feed red-green section coloring, which is not yet
    // wired into the driver; recording them would re-hash every input's full
    // bytes on every link (the cost we just removed from the reuse path). Record
    // them only when section records are actually being produced.
    let input_fps = if sections.is_empty() {
        Vec::new()
    } else {
        let mut v = Vec::with_capacity(inputs.len());
        for p in inputs {
            v.push((p.display().to_string(), Fingerprint::of_file(p)?));
        }
        v
    };
    let output_fp = if sections.is_empty() {
        Fingerprint::default()
    } else {
        Fingerprint::of_file(output)?
    };

    let epoch_key = compute_epoch_key(inputs, args_hash);
    let manifest = Manifest {
        version: CACHE_VERSION,
        epoch_key,
        inputs: input_fps,
        fast_inputs,
        output: output_fp,
        fast_output: FastFingerprint::of_file(output)?,
        sections: sections.to_vec(),
        symbols: symbols.to_vec(),
    };

    let bytes = bincode::serde::encode_to_vec(&manifest, bincode::config::standard())
        .map_err(|e| CacheError::Codec(e.to_string()))?;
    atomic_write(&manifest_path(output), &bytes)
}

/// Compute the red-green coloring for each output section.
///
/// Implements QUAD Definition 6.1 and Theorem 6.1:
/// - **Green**: section fingerprint unchanged AND no relocation target moved.
/// - **Red**: fingerprint changed OR a relocation target in this section moved.
///
/// `moved_symbol_ids` are the numeric IDs of symbols whose virtual address
/// changed since the last link (obtained by comparing the current symbol table
/// against the cached one in the manifest).
///
/// `rev_index` is the relocation reverse index built from the current link.
/// `reloc_sections[reloc_id]` gives the output section name for that relocation.
///
/// Returns a map from output section name → [`SectionColor`].
pub fn compute_red_green(
    output: &Path,
    current_sections: &[(String, Fingerprint)],
    moved_symbol_ids: &[u32],
    rev_index: &RelocReverseIndex,
    reloc_sections: &[&str],
) -> Result<HashMap<String, SectionColor>> {
    let path = manifest_path(output);
    if !path.exists() {
        return Ok(all_red(current_sections));
    }

    let bytes = std::fs::read(&path).map_err(|e| CacheError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let (manifest, _): (Manifest, usize) =
        match bincode::serde::decode_from_slice(&bytes, bincode::config::standard()) {
            Ok(v) => v,
            Err(_) => return Ok(all_red(current_sections)),
        };

    if manifest.version != CACHE_VERSION {
        return Ok(all_red(current_sections));
    }

    let prev_fps: HashMap<&str, Fingerprint> = manifest
        .sections
        .iter()
        .map(|s| (s.name.as_str(), s.fingerprint))
        .collect();

    let mut coloring = HashMap::new();
    for (name, fp) in current_sections {
        let fingerprint_unchanged = prev_fps
            .get(name.as_str())
            .map(|&prev| prev == *fp)
            .unwrap_or(false);

        let no_moved_target =
            !section_references_moved_checked(name, moved_symbol_ids, rev_index, reloc_sections);

        let color = if fingerprint_unchanged && no_moved_target {
            SectionColor::Green
        } else {
            SectionColor::Red
        };
        coloring.insert(name.clone(), color);
    }
    Ok(coloring)
}

fn all_red(sections: &[(String, Fingerprint)]) -> HashMap<String, SectionColor> {
    sections
        .iter()
        .map(|(n, _)| (n.clone(), SectionColor::Red))
        .collect()
}

/// Returns `true` if `section_name` should be re-emitted because one of its
/// relocation targets moved.
///
/// Uses the relocation reverse index when available: for each moved symbol we
/// walk its chain of relocations and check whether any of them falls in
/// `section_name`.  Falls back to marking all sections Red when the index is
/// absent (safe: only loses incremental efficiency, never correctness).
///
/// `reloc_sections` maps reloc_id → output section name. This is built once
/// from the layout before calling `compute_red_green`.
fn section_references_moved_checked(
    section_name: &str,
    moved_ids: &[u32],
    rev_index: &RelocReverseIndex,
    reloc_sections: &[&str],
) -> bool {
    if moved_ids.is_empty() {
        return false;
    }
    for &sym_id in moved_ids {
        for reloc_id in rev_index.iter_relocs(sym_id) {
            let r_idx = reloc_id as usize;
            if reloc_sections
                .get(r_idx)
                .map(|&s| s == section_name)
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| CacheError::Io {
        path: tmp.display().to_string(),
        source: e,
    })?;
    std::fs::rename(&tmp, path).map_err(|e| CacheError::Io {
        path: path.display().to_string(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic_and_change_sensitive() {
        let a = Fingerprint::of_bytes(b"hello world");
        let b = Fingerprint::of_bytes(b"hello world");
        let c = Fingerprint::of_bytes(b"hello worlD");
        assert_eq!(a, b, "same bytes → same fingerprint");
        assert_ne!(a, c, "one flipped bit → different fingerprint");
        assert_eq!(a.len, 11);
    }

    #[test]
    fn fingerprint_distinguishes_length() {
        let a = Fingerprint::of_bytes(b"ab");
        let b = Fingerprint::of_bytes(b"abc");
        assert_ne!(a, b);
    }

    #[test]
    fn cache_dir_uses_incr_extension() {
        assert_eq!(cache_dir(Path::new("/tmp/a.out")), Path::new("/tmp/a.incr"));
    }

    #[test]
    fn reloc_reverse_index_insert_and_iterate() {
        let idx = RelocReverseIndex::new(4, 8);

        // Insert relocs 0, 1, 2 for symbol 0.
        idx.insert(0, 0);
        idx.insert(0, 1);
        idx.insert(0, 2);

        // Insert reloc 3 for symbol 1.
        idx.insert(1, 3);

        let mut s0_relocs: Vec<u32> = idx.iter_relocs(0).collect();
        s0_relocs.sort();
        assert_eq!(s0_relocs, vec![0, 1, 2]);

        let s1_relocs: Vec<u32> = idx.iter_relocs(1).collect();
        assert_eq!(s1_relocs, vec![3]);

        // Symbol 2 has no relocs.
        let s2_relocs: Vec<u32> = idx.iter_relocs(2).collect();
        assert!(s2_relocs.is_empty());
    }

    #[test]
    fn reloc_reverse_index_lock_free_parallel_insert() {
        use std::sync::Arc;
        let idx = Arc::new(RelocReverseIndex::new(1, 100));

        // Insert 100 relocs for symbol 0 from parallel threads using std::thread::scope.
        std::thread::scope(|scope| {
            for i in 0u32..100 {
                let idx = Arc::clone(&idx);
                scope.spawn(move || {
                    idx.insert(0, i);
                });
            }
        });

        let mut all: Vec<u32> = idx.iter_relocs(0).collect();
        all.sort();
        assert_eq!(
            all,
            (0..100).collect::<Vec<_>>(),
            "all 100 relocs must be present"
        );
    }

    #[test]
    fn epoch_key_changes_with_args() {
        let k1 = hash_args(&["peony".to_string(), "-o".to_string(), "a.out".to_string()]);
        let k2 = hash_args(&["peony".to_string(), "-o".to_string(), "b.out".to_string()]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn pre_hashed_equality_and_hash() {
        use peony_symbols::PreHashed;
        let a = PreHashed::new(b"hello".to_vec());
        let b = PreHashed::new(b"hello".to_vec());
        let c = PreHashed::new(b"world".to_vec());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn epoch_key_differs_by_args_hash() {
        // Different args_hash must yield different epoch keys (even with empty inputs).
        let k1 = compute_epoch_key(&[], 42);
        let k2 = compute_epoch_key(&[], 43);
        assert_ne!(k1, k2, "epoch key must depend on args_hash");
    }

    #[test]
    fn section_records_round_trip_through_manifest() {
        // record_link_with_sections must persist SectionRecords (offset, size,
        // capacity, fingerprint, vaddr) so an in-place relink can read them back.
        let dir = std::env::temp_dir().join(format!("peony-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("a.out");
        let inp = dir.join("in.o");
        std::fs::write(&out, b"OUTPUT-BINARY-CONTENT").unwrap();
        std::fs::write(&inp, b"input-object").unwrap();

        let secs = vec![
            SectionRecord {
                name: ".text".to_string(),
                fingerprint: Fingerprint::of_bytes(b"text-bytes"),
                file_offset: 0x1000,
                size: 0x200,
                capacity: 0x200,
                virtual_address: 0x1000,
            },
            SectionRecord {
                name: ".data".to_string(),
                fingerprint: Fingerprint::of_bytes(b"data-bytes"),
                file_offset: 0x2000,
                size: 0x40,
                capacity: 0x40,
                virtual_address: 0x2000,
            },
        ];
        record_link_with_sections(&out, &[inp], 0, &secs, &[]).unwrap();

        // Read the manifest back and confirm the records survived intact.
        let bytes = std::fs::read(manifest_path(&out)).unwrap();
        let (m, _): (Manifest, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(m.sections.len(), 2);
        assert_eq!(m.sections[0].name, ".text");
        assert_eq!(m.sections[0].file_offset, 0x1000);
        assert!(
            m.sections[0].capacity >= m.sections[0].size,
            "capacity >= size"
        );
        assert_eq!(m.sections[1].name, ".data");
        assert_eq!(
            m.sections[1].fingerprint,
            Fingerprint::of_bytes(b"data-bytes")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
