//! Small list-navigation helper shared by the list view and modal widgets.

/// Moves `cursor` by `delta` within `len` items, wrapping at both ends
/// (cyclic): one step past the last item lands on the first and vice versa.
/// `len == 0` yields 0.
pub fn cycle(cursor: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (cursor as isize + delta).rem_euclid(len as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_both_ends() {
        assert_eq!(cycle(2, 3, 1), 0);
        assert_eq!(cycle(0, 3, -1), 2);
        assert_eq!(cycle(0, 3, 1), 1);
        assert_eq!(cycle(0, 0, 1), 0);
    }
}
