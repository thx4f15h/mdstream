use mdstream::*;
#[test]
fn raw_run_metadata_preserves_utf8_bounds_and_append_ownership_without_decode_work() {
    let mut d = Document::new();
    d.append("a   界界").unwrap();
    let raw = d.display_source();
    let before = raw.stats().work;
    assert_eq!(raw.run_at_source(1), Some((' ', Span::new(1, 4))));
    assert_eq!(raw.run_at_source(5), Some(('界', Span::new(4, 10))));
    assert_eq!(raw.run_at_source(10), None);
    assert_eq!(raw.stats().work, before);
    d.append("界").unwrap();
    assert_eq!(raw.run_at_source(7), Some(('界', Span::new(4, 13))));
}
#[test]
fn native_gap_metadata_has_real_predecessor_containing_and_successor_boundaries() {
    let mut d = Document::new();
    d.append("\\(a\\) text \\(b\\) end\n").unwrap();
    d.finish();
    let first = d.math_object_at_or_after(0).unwrap();
    let second = d.math_object_at_or_after(first.source.end).unwrap();
    assert_eq!(first.source, Span::new(0, 5));
    assert_eq!(second.source, Span::new(11, 16));
    assert_eq!(d.math_object_before(0), None);
    assert_eq!(d.math_object_before(11), Some(first));
    assert_eq!(d.math_object_before(12), Some(second));
    assert_eq!(d.math_object_at_or_after(2), Some(first));
    assert_eq!(d.math_object_at_or_after(11), Some(second));
    assert_eq!(d.math_object_at_or_after(16), None);
}
