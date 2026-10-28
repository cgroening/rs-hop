//! Pure grouping of Files-tab entries into ordered user sections, plus the
//! section-to-section jump helpers. No I/O and no UI: every input is plain
//! data, so the rules stay testable.

/// The label shown for entries without a user section.
pub const UNGROUPED: &str = "Ungrouped";

/// One section in display order: its label and the entry items it holds (the
/// caller's opaque indices, kept in their input order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionGroup {
    /// The section name (or [`UNGROUPED`] for the implicit trailing group).
    pub label: String,
    /// The items belonging to this section, in input order.
    pub items: Vec<usize>,
}

/// Groups `items` into the sections named by `order` (matched case-insensitively
/// to each item's section via `section_of`), keeping each section's items in
/// input order. Empty named sections are dropped; items with no section (or one
/// not in `order`) collect into a trailing [`UNGROUPED`] group.
pub fn group(
    order: &[String],
    items: &[usize],
    section_of: impl Fn(usize) -> Option<String>,
) -> Vec<SectionGroup> {
    let mut named: Vec<SectionGroup> = order
        .iter()
        .map(|name| SectionGroup {
            label: name.clone(),
            items: Vec::new(),
        })
        .collect();
    let mut ungrouped: Vec<usize> = Vec::new();
    for &item in items {
        match section_position(order, section_of(item).as_deref()) {
            Some(pos) => named[pos].items.push(item),
            None => ungrouped.push(item),
        }
    }
    let mut groups: Vec<SectionGroup> =
        named.into_iter().filter(|g| !g.items.is_empty()).collect();
    if !ungrouped.is_empty() {
        groups.push(SectionGroup {
            label: UNGROUPED.to_string(),
            items: ungrouped,
        });
    }
    groups
}

/// The index in `order` whose name matches `section` (case-insensitive).
fn section_position(order: &[String], section: Option<&str>) -> Option<usize> {
    let name = section?;
    order.iter().position(|o| o.eq_ignore_ascii_case(name))
}

/// The flattened entry items across `groups`, in display order.
pub fn flatten(groups: &[SectionGroup]) -> Vec<usize> {
    groups
        .iter()
        .flat_map(|g| g.items.iter().copied())
        .collect()
}

/// One cursor row of the grouped view: an entry, or the header of a collapsed
/// section (which stands in for its hidden entries). An expanded section's
/// header is purely visual and never a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// The entry with this (opaque, caller-owned) item index.
    Entry(usize),
    /// The collapsed section at this index into the groups.
    Collapsed(usize),
}

/// The cursor rows across `groups`, in display order: one [`Row::Collapsed`]
/// for each group `is_collapsed` reports by label, its entries otherwise.
pub fn rows(
    groups: &[SectionGroup],
    is_collapsed: impl Fn(&str) -> bool,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for (pos, group) in groups.iter().enumerate() {
        if is_collapsed(&group.label) {
            rows.push(Row::Collapsed(pos));
        } else {
            rows.extend(group.items.iter().map(|&item| Row::Entry(item)));
        }
    }
    rows
}

/// The row position at which each section starts: its collapsed header, or
/// its first entry.
pub fn row_starts(
    groups: &[SectionGroup],
    is_collapsed: impl Fn(&str) -> bool,
) -> Vec<usize> {
    let mut starts = Vec::with_capacity(groups.len());
    let mut pos = 0;
    for group in groups {
        starts.push(pos);
        pos += if is_collapsed(&group.label) {
            1
        } else {
            group.items.len()
        };
    }
    starts
}

/// The start of the section after the one holding `cursor`, wrapping from the
/// last section to the first. `None` when there are no sections.
pub fn next_start(starts: &[usize], cursor: usize) -> Option<usize> {
    let current = current_section(starts, cursor)?;
    Some(starts[(current + 1) % starts.len()])
}

/// The start of the section holding `cursor` when the cursor sits past it,
/// else the start of the previous section (wrapping from the first to the
/// last). `None` when there are no sections.
pub fn prev_start(starts: &[usize], cursor: usize) -> Option<usize> {
    let current = current_section(starts, cursor)?;
    if starts[current] < cursor {
        return Some(starts[current]);
    }
    let previous = (current + starts.len() - 1) % starts.len();
    Some(starts[previous])
}

/// The index of the section containing the entry at display position `cursor`:
/// the last start at or before it.
pub fn current_section(starts: &[usize], cursor: usize) -> Option<usize> {
    starts.iter().rposition(|&start| start <= cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<String> {
        vec!["Work".to_string(), "Personal".to_string()]
    }

    #[test]
    fn groups_by_order_with_ungrouped_last() {
        // items 0,1 -> Work; 2 -> Personal; 3 -> none.
        let section_of = |i: usize| match i {
            0 | 1 => Some("Work".to_string()),
            2 => Some("personal".to_string()), // case-insensitive match
            _ => None,
        };
        let groups = group(&order(), &[0, 1, 2, 3], section_of);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].label, "Work");
        assert_eq!(groups[0].items, vec![0, 1]);
        assert_eq!(groups[1].label, "Personal");
        assert_eq!(groups[1].items, vec![2]);
        assert_eq!(groups[2].label, UNGROUPED);
        assert_eq!(groups[2].items, vec![3]);
    }

    #[test]
    fn empty_named_sections_are_dropped() {
        let groups = group(&order(), &[0], |_| Some("Personal".to_string()));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Personal");
    }

    #[test]
    fn unknown_section_falls_into_ungrouped() {
        let groups = group(&order(), &[0], |_| Some("Misc".to_string()));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, UNGROUPED);
    }

    /// Three sections of sizes 2, 1, 3 labelled a, b, c.
    fn three_groups() -> Vec<SectionGroup> {
        let group = |label: &str, items: Vec<usize>| SectionGroup {
            label: label.to_string(),
            items,
        };
        vec![
            group("a", vec![0, 1]),
            group("b", vec![2]),
            group("c", vec![3, 4, 5]),
        ]
    }

    #[test]
    fn rows_list_every_entry_when_nothing_is_collapsed() {
        let rows = rows(&three_groups(), |_| false);
        let expected: Vec<Row> = (0..6).map(Row::Entry).collect();
        assert_eq!(rows, expected);
    }

    #[test]
    fn a_collapsed_section_becomes_one_header_row() {
        let rows = rows(&three_groups(), |label| label == "a");
        assert_eq!(
            rows,
            vec![
                Row::Collapsed(0),
                Row::Entry(2),
                Row::Entry(3),
                Row::Entry(4),
                Row::Entry(5),
            ],
        );
    }

    #[test]
    fn row_starts_count_a_collapsed_section_as_one_row() {
        let groups = three_groups();
        assert_eq!(row_starts(&groups, |_| false), vec![0, 2, 3]);
        assert_eq!(row_starts(&groups, |label| label == "a"), vec![0, 1, 2]);
        assert_eq!(current_section(&[0, 2, 3], 1), Some(0));
        assert_eq!(current_section(&[0, 2, 3], 4), Some(2));
    }

    #[test]
    fn next_start_moves_on_and_wraps() {
        let starts = [0, 2, 3];
        assert_eq!(next_start(&starts, 1), Some(2));
        assert_eq!(next_start(&starts, 2), Some(3));
        assert_eq!(next_start(&starts, 5), Some(0));
    }

    #[test]
    fn prev_start_goes_to_the_own_start_first() {
        let starts = [0, 2, 3];
        assert_eq!(prev_start(&starts, 5), Some(3));
        assert_eq!(prev_start(&starts, 3), Some(2));
    }

    #[test]
    fn prev_start_wraps_from_the_first_section() {
        assert_eq!(prev_start(&[0, 2, 3], 0), Some(3));
    }

    #[test]
    fn jumps_need_sections() {
        assert_eq!(next_start(&[], 0), None);
        assert_eq!(prev_start(&[], 0), None);
    }
}
