//! Drag and drop: where a dragged row would land, and the order that results.
//!
//! The window owns the pointer; this owns the arithmetic. Given the rows'
//! positions and the pointer, [`drop_slot`] says between which two rows the
//! dragged one would be inserted, and [`moved`] gives the order afterwards.
//! Keeping it here means the same rule serves a todo list, the tabs of a pane
//! and the tasks of a plan, and that it can be tested without a mouse.

/// A row's vertical extent in the list, in the list's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    /// Top edge.
    pub top: f32,
    /// Height.
    pub height: f32,
}

impl Extent {
    fn middle(&self) -> f32 {
        self.top + self.height / 2.0
    }
}

/// The gap a dragged row would be dropped into: 0 is before the first row,
/// `rows.len()` after the last.
///
/// The gap changes as the pointer crosses the middle of a row, which is where a
/// person expects it: half a row of movement is enough to swap with a
/// neighbour, whatever the rows' heights.
pub fn drop_slot(rows: &[Extent], pointer_y: f32) -> usize {
    rows.iter().take_while(|row| pointer_y > row.middle()).count()
}

/// The order after moving the row at `from` into gap `slot`.
///
/// Dropping a row into the gap on either side of itself changes nothing, so
/// `None` means "leave it" and the caller need not send a write.
pub fn moved<T: Clone>(items: &[T], from: usize, slot: usize) -> Option<Vec<T>> {
    if from >= items.len() || slot > items.len() || slot == from || slot == from + 1 {
        return None;
    }
    let mut out = items.to_vec();
    let item = out.remove(from);
    // Removing the row shifts every gap after it up by one.
    out.insert(if slot > from { slot - 1 } else { slot }, item);
    Some(out)
}

/// A sort key between two others, so one row can be moved by writing one row.
///
/// Positions are stored as numbers. A row dropped between two others takes the
/// number halfway between theirs; dropped at an end, it takes a whole step past
/// it. Nothing else is renumbered, which is what makes a reorder a single
/// optimistic write that cannot conflict with another row's.
pub fn key_between(before: Option<f64>, after: Option<f64>) -> f64 {
    match (before, after) {
        (None, None) => 1.0,
        (Some(b), None) => b.floor() + 1.0,
        (None, Some(a)) => a.ceil() - 1.0,
        (Some(b), Some(a)) => (a + b) / 2.0,
    }
}

/// Whether two neighbouring keys have grown too close to put another between
/// them. After roughly fifty moves into the same gap the caller renumbers the
/// list once with [`renumber`].
pub fn needs_renumber(before: f64, after: f64) -> bool {
    (after - before).abs() < 1e-9
}

/// Fresh keys for a whole list: 1, 2, 3…
pub fn renumber(count: usize) -> Vec<f64> {
    (1..=count).map(|n| n as f64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(heights: &[f32]) -> Vec<Extent> {
        let mut top = 0.0;
        heights
            .iter()
            .map(|h| {
                let e = Extent { top, height: *h };
                top += h;
                e
            })
            .collect()
    }

    #[test]
    fn the_gap_changes_at_the_middle_of_a_row() {
        let list = rows(&[40.0, 40.0, 40.0]);
        assert_eq!(drop_slot(&list, -5.0), 0);
        assert_eq!(drop_slot(&list, 19.0), 0);
        assert_eq!(drop_slot(&list, 21.0), 1);
        assert_eq!(drop_slot(&list, 61.0), 2);
        assert_eq!(drop_slot(&list, 500.0), 3);
    }

    #[test]
    fn rows_of_different_heights_still_swap_at_their_own_middle() {
        let list = rows(&[20.0, 100.0, 20.0]);
        assert_eq!(drop_slot(&list, 60.0), 1, "above the tall row's middle");
        assert_eq!(drop_slot(&list, 80.0), 2);
    }

    #[test]
    fn moving_a_row() {
        let items = ["a", "b", "c", "d"];
        assert_eq!(moved(&items, 0, 4).unwrap(), ["b", "c", "d", "a"]);
        assert_eq!(moved(&items, 3, 0).unwrap(), ["d", "a", "b", "c"]);
        assert_eq!(moved(&items, 1, 3).unwrap(), ["a", "c", "b", "d"]);
        assert_eq!(moved(&items, 2, 1).unwrap(), ["a", "c", "b", "d"]);
    }

    #[test]
    fn dropping_a_row_beside_itself_is_not_a_move() {
        let items = ["a", "b", "c"];
        assert_eq!(moved(&items, 1, 1), None);
        assert_eq!(moved(&items, 1, 2), None);
        assert_eq!(moved(&items, 9, 0), None);
        assert_eq!(moved(&items, 0, 9), None);
    }

    #[test]
    fn a_key_lands_between_its_neighbours() {
        assert_eq!(key_between(None, None), 1.0);
        assert_eq!(key_between(Some(3.0), None), 4.0);
        assert_eq!(key_between(Some(3.5), None), 4.0);
        assert_eq!(key_between(None, Some(1.0)), 0.0);
        assert_eq!(key_between(Some(1.0), Some(2.0)), 1.5);
        let mut lo = 1.0;
        let hi = 2.0;
        let mut moves = 0;
        while !needs_renumber(lo, hi) {
            lo = key_between(Some(lo), Some(hi));
            assert!(lo > 1.0 && lo < hi);
            moves += 1;
        }
        assert!(moves >= 25, "{moves} moves into one gap before a renumber");
        assert_eq!(renumber(3), [1.0, 2.0, 3.0]);
    }
}
