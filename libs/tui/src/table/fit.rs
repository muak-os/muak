//! Column widths fitted to a terminal width.

use crate::table::Cell;

/// Narrowest column that still shows head, ellipsis and tail.
const MIN_WIDTH: usize = 3;

/// Returns column widths that fit `cols` display columns, eliding the widest
/// shrinkable columns down to their floors when the natural widths overflow.
#[must_use]
pub(crate) fn widths(headers: &[Cell], natural: &[usize], cols: usize) -> Vec<usize> {
    if minimum(headers, natural) > cols {
        return natural.to_vec();
    }

    let mut widths = natural.to_vec();
    let mut overflow = total(&widths).saturating_sub(cols);

    while overflow > 0 {
        let Some(index) = widest(&widths, headers) else {
            break;
        };
        let Some(width) = widths.get_mut(index) else {
            break;
        };
        let reduction = overflow.min(width.saturating_sub(floor(headers, index)));
        *width = width.saturating_sub(reduction);
        overflow = overflow.saturating_sub(reduction);
    }

    widths
}

fn minimum(headers: &[Cell], natural: &[usize]) -> usize {
    let cells = natural
        .iter()
        .enumerate()
        .map(|(index, width)| match index {
            0 => *width,
            _ => floor(headers, index).min(*width),
        })
        .fold(0, usize::saturating_add);

    cells.saturating_add(gaps(natural.len()))
}

fn total(widths: &[usize]) -> usize {
    let cells = widths.iter().copied().fold(0, usize::saturating_add);

    cells.saturating_add(gaps(widths.len()))
}

fn gaps(len: usize) -> usize {
    len.saturating_sub(1).saturating_mul(2)
}

fn widest(widths: &[usize], headers: &[Cell]) -> Option<usize> {
    widths
        .iter()
        .enumerate()
        .skip(1)
        .filter(|&(index, width)| *width > floor(headers, index))
        .max_by_key(|&(_, width)| *width)
        .map(|(index, _)| index)
}

fn floor(headers: &[Cell], index: usize) -> usize {
    let header = headers.get(index).map_or(0, Cell::width);

    header.max(MIN_WIDTH)
}
