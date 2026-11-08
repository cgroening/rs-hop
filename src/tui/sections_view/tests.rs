use super::*;

#[test]
fn pad_fills_or_truncates() {
    assert_eq!(pad("ab", 5), "ab   ");
    assert_eq!(pad("abcdef", 4), "abc…");
}

#[test]
fn name_field_spans_pads_the_name() {
    // The slug is its own column now; the name field is a single padded
    // span sized to the name column width.
    let spans = name_field_spans("hop", 10);
    assert_eq!(spans.len(), 1);
}
