//! NTFS data runs ("mapping pairs"): the compressed list of which clusters
//! hold a non-resident attribute's data.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub vcn: u64,
    pub len: u64,
    /// `None` = sparse (reads as zeros, nothing stored on disk).
    pub lcn: Option<u64>,
}

/// Decodes a runlist starting at virtual cluster `start_vcn`. `None` if it
/// is malformed (oversized fields, a run before cluster 0, zero length).
pub fn decode_runlist(b: &[u8], start_vcn: u64) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    let mut vcn = start_vcn;
    let mut lcn: i64 = 0;
    while let Some(&head) = b.get(i) {
        if head == 0 {
            return Some(runs);
        }
        let len_n = (head & 0x0F) as usize;
        let off_n = (head >> 4) as usize;
        if len_n == 0 || len_n > 8 || off_n > 8 {
            return None;
        }
        let len = le_uint(b.get(i + 1..i + 1 + len_n)?);
        if len == 0 {
            return None;
        }
        let this_lcn = if off_n == 0 {
            None
        } else {
            lcn = lcn.checked_add(le_int(b.get(i + 1 + len_n..i + 1 + len_n + off_n)?))?;
            if lcn < 0 {
                return None;
            }
            Some(lcn as u64)
        };
        runs.push(Run { vcn, len, lcn: this_lcn });
        vcn = vcn.checked_add(len)?;
        i += 1 + len_n + off_n;
    }
    // Ran off the end of the attribute without a terminator.
    None
}

fn le_uint(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

fn le_int(b: &[u8]) -> i64 {
    let mut v = le_uint(b) as i64;
    let bits = b.len() * 8;
    if bits < 64 && b.last().is_some_and(|&x| x & 0x80 != 0) {
        v -= 1i64 << bits;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_relative_negative_and_sparse_runs() {
        // 0x18 clusters at LCN 0x5634, then 0x10 at 0x5634-0x10, then 0x20 sparse.
        let b = [0x21, 0x18, 0x34, 0x56, 0x11, 0x10, 0xF0, 0x01, 0x20, 0x00];
        let runs = decode_runlist(&b, 0).unwrap();
        assert_eq!(
            runs,
            vec![
                Run { vcn: 0, len: 0x18, lcn: Some(0x5634) },
                Run { vcn: 0x18, len: 0x10, lcn: Some(0x5624) },
                Run { vcn: 0x28, len: 0x20, lcn: None },
            ]
        );
    }

    #[test]
    fn rejects_malformed_runlists() {
        assert!(decode_runlist(&[0x21, 0x18, 0x34], 0).is_none()); // truncated
        assert!(decode_runlist(&[0x11, 0x05, 0x80, 0x00], 0).is_none()); // LCN < 0
        assert!(decode_runlist(&[0x11, 0x05, 0x10], 0).is_none()); // no terminator
        assert!(decode_runlist(&[0x01, 0x00, 0x00], 0).is_none()); // zero length
    }
}
