use peony_layout::{Layout, SecSource};
use peony_object::elf;

/// Write the `.note.gnu.build-id` note HEADER only, leaving the 16-byte
/// descriptor zeroed. The descriptor is filled by [`finalize_build_id`] after
/// the whole output image is written — it hashes the emitted bytes (≈4MB), not
/// the much larger scattered input set (≈18.5MB incl. discarded/debug sections),
/// which is what `ld`/lld/mold do and is ~4× less data hashed.
pub(crate) fn write_build_id(buf: &mut [u8], off: u64) {
    let off = off as usize;
    if off + 32 > buf.len() {
        return;
    }
    buf[off..off + 4].copy_from_slice(&4u32.to_le_bytes()); // namesz = len("GNU\0")
    buf[off + 4..off + 8].copy_from_slice(&16u32.to_le_bytes()); // descsz = hash len
    buf[off + 8..off + 12].copy_from_slice(&elf::NT_GNU_BUILD_ID.to_le_bytes());
    buf[off + 12..off + 16].copy_from_slice(b"GNU\0");
    // Descriptor zeroed; filled in by finalize_build_id once the image is final.
    buf[off + 16..off + 32].copy_from_slice(&[0u8; 16]);
}

/// Fixed hash block (256 KiB). Block boundaries depend only on `buf.len()`, so
/// the per-block hashes can be computed in parallel and folded in index order
/// for a build-id that is identical regardless of `--threads`.
const BUILD_ID_BLOCK: usize = 256 * 1024;

/// Fill the `.note.gnu.build-id` descriptor with a content hash of the FINAL
/// output image. Must run after every byte (section data, relocations, headers)
/// is written and while the descriptor is still zero. Hashes the contiguous
/// buffer in parallel fixed-size blocks, then folds the block digests in index
/// order — so the result is deterministic across thread counts.
pub(crate) fn finalize_build_id(buf: &mut [u8], layout: &Layout) {
    let Some(off) = build_id_descriptor_offset(layout) else {
        return;
    };
    if off + 16 > buf.len() {
        return;
    }
    let digest = build_id_hash(buf);
    buf[off..off + 16].copy_from_slice(&digest);
}

/// File offset of the build-id note's 16-byte descriptor, or `None` if the
/// output has no build-id section.
fn build_id_descriptor_offset(layout: &Layout) -> Option<usize> {
    layout
        .output_sections
        .iter()
        .find(|s| s.source == SecSource::NoteBuildId)
        .map(|s| s.sh_offset as usize + 16)
}

/// A deterministic 128-bit hash over the contiguous output buffer, computed in
/// parallel fixed-size blocks and folded in block order. The descriptor region
/// is zero at hash time (filled afterwards), so the hash is stable.
fn build_id_hash(buf: &[u8]) -> [u8; 16] {
    use rayon::prelude::*;
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    // Per-block double-FNV; blocks are independent so this parallelizes. Each
    // block's (h1,h2) is seeded with its index so identical blocks at different
    // positions do not cancel.
    let block_hash = |(idx, chunk): (usize, &[u8])| -> (u64, u64) {
        let mut h1 = OFFSET ^ (idx as u64).wrapping_mul(PRIME);
        let mut h2 = 0x9e37_79b9_7f4a_7c15u64 ^ (idx as u64);
        for &b in chunk {
            h1 = (h1 ^ b as u64).wrapping_mul(PRIME);
            h2 = (h2.wrapping_add(b as u64)).wrapping_mul(PRIME) ^ (h2 >> 29);
        }
        (h1, h2)
    };

    // Small outputs: hash serially (avoids touching the pool for a handful of
    // blocks). Large outputs: hash blocks in parallel. Either way the fold is in
    // index order, so the digest is identical regardless of thread count.
    let blocks: Vec<(u64, u64)> = if buf.len() >= 4 * BUILD_ID_BLOCK {
        buf.par_chunks(BUILD_ID_BLOCK)
            .enumerate()
            .map(block_hash)
            .collect()
    } else {
        buf.chunks(BUILD_ID_BLOCK)
            .enumerate()
            .map(block_hash)
            .collect()
    };

    let mut h1 = OFFSET;
    let mut h2 = 0x9e37_79b9_7f4a_7c15u64;
    for (b1, b2) in blocks {
        h1 = (h1 ^ b1).wrapping_mul(PRIME);
        h2 = (h2.wrapping_add(b2)).wrapping_mul(PRIME) ^ (h2 >> 29);
    }
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&h1.to_le_bytes());
    out[8..16].copy_from_slice(&h2.to_le_bytes());
    out
}
