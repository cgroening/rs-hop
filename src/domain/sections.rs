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

/// The display position (into the flattened list) at which each section starts.
pub fn section_starts(groups: &[SectionGroup]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(groups.len());
    let mut pos = 0;
    for group in groups {
        starts.push(pos);
        pos += group.items.len();
    }
    starts
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

    #[test]
    fn starts_and_jump_move_between_sections() {
        // Three sections of sizes 2,1,3 -> starts at 0,2,3.
        let groups = vec![
            SectionGroup {
                label: "a".to_string(),
                items: vec![0, 1],
            },
            SectionGroup {
                label: "b".to_string(),
                items: vec![2],
            },
            SectionGroup {
                label: "c".to_string(),
                items: vec![3, 4, 5],
            },
        ];
        let starts = section_starts(&groups);
        assert_eq!(starts, vec![0, 2, 3]);
        assert_eq!(current_section(&starts, 1), Some(0));
        assert_eq!(current_section(&starts, 4), Some(2));
    }
}
