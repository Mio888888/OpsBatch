//! Broadcast terminal window (M5-12): one terminal per host, input typed anywhere goes to every connected host.

pub mod window;

pub const MAX_HOSTS: usize = 16;
pub const MAX_CONCURRENT_CONNECTS: usize = 4;

/// Columns for the grid: explicit 1–4, or 0 = auto (ceil(sqrt(n))).
pub fn effective_columns(selected: usize, panes: usize) -> usize {
    if selected == 0 {
        ((panes as f64).sqrt().ceil() as usize).max(1)
    } else {
        selected
    }
}

/// Split panes into grid rows.
pub fn rows<T: Clone>(items: &[T], cols: usize) -> Vec<Vec<T>> {
    items.chunks(cols.max(1)).map(|c| c.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_columns_follow_the_square_root() {
        assert_eq!(effective_columns(0, 1), 1);
        assert_eq!(effective_columns(0, 4), 2);
        assert_eq!(effective_columns(0, 5), 3);
        assert_eq!(effective_columns(0, 16), 4);
        assert_eq!(effective_columns(3, 16), 3);
        assert_eq!(effective_columns(0, 0), 1);
    }

    #[test]
    fn rows_chunk_by_columns() {
        let r = rows(&[1, 2, 3, 4, 5], 2);
        assert_eq!(r, vec![vec![1, 2], vec![3, 4], vec![5]]);
    }
}
