pub fn count_fdes(data: &[u8]) -> usize {
    let (cies, fdes, terms, leftover) = scan_eh_frame(data);
    tracing::trace!(
        len = data.len(),
        cies,
        fdes,
        terms,
        leftover,
        "count_fdes: scanned input .eh_frame"
    );
    fdes
}

pub fn ends_with_eh_terminator(data: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            return pos + 4 == data.len();
        }
        if len == 0xffff_ffff {
            return false;
        }
        let rec_start = pos + 4;
        if rec_start + len > data.len() {
            return false;
        }
        pos = rec_start + len;
    }
    false
}

pub fn scan_eh_frame(data: &[u8]) -> (usize, usize, usize, usize) {
    let mut pos = 0usize;
    let (mut cies, mut fdes, mut terms) = (0usize, 0usize, 0usize);
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            terms += 1;
            pos += 4;
            continue;
        }
        if len == 0xffff_ffff {
            break;
        }
        let rec_start = pos + 4;
        if rec_start + 4 > data.len() || rec_start + len > data.len() {
            break;
        }
        let cie_ptr = u32::from_le_bytes(data[rec_start..rec_start + 4].try_into().unwrap());
        if cie_ptr == 0 {
            cies += 1;
        } else {
            fdes += 1;
        }
        pos = rec_start + len;
    }
    (cies, fdes, terms, data.len().saturating_sub(pos))
}

pub fn iter_fdes(data: &[u8]) -> Vec<(usize, usize, i64)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            pos += 4;
            continue;
        }
        if len == 0xffff_ffff {
            break;
        }
        let rec_start = pos + 4;
        if rec_start + 8 > data.len() || rec_start + len > data.len() {
            break;
        }
        let cie_ptr = u32::from_le_bytes(data[rec_start..rec_start + 4].try_into().unwrap());
        if cie_ptr != 0 {
            let pcbegin_off = rec_start + 4;
            let rel =
                i32::from_le_bytes(data[pcbegin_off..pcbegin_off + 4].try_into().unwrap()) as i64;
            out.push((pos, pcbegin_off, rel));
        }
        pos = rec_start + len;
    }
    out
}
