//! Compact-index sizing and partitioning helpers.

pub(super) fn index_layout(totalidx: usize, ebase: usize) -> (usize, usize, usize) {
    let ebase_mod = ebase.checked_rem(32).unwrap_or_default();
    let remaining = 32_usize.saturating_sub(ebase_mod);
    let mut compacted_4b_initial = remaining.checked_div(4).unwrap_or_default() & 7;
    let compacted_2b;
    if compacted_4b_initial < totalidx {
        compacted_2b = totalidx
            .saturating_sub(compacted_4b_initial)
            .checked_div(16)
            .unwrap_or_default()
            .saturating_mul(16);
    } else {
        compacted_4b_initial = 0;
        compacted_2b = 0;
    }
    let compacted_4b_end = totalidx
        .saturating_sub(compacted_4b_initial)
        .saturating_sub(compacted_2b);
    (compacted_4b_initial, compacted_2b, compacted_4b_end)
}

pub(super) fn index_bytes(totalidx: usize, ebase: usize) -> usize {
    let (c4i, c2b, c4e) = index_layout(totalidx, ebase);
    c4i.div_ceil(2)
        .saturating_mul(8)
        .saturating_add(c2b.checked_div(16).unwrap_or_default().saturating_mul(32))
        .saturating_add(c4e.div_ceil(2).saturating_mul(8))
}
