//! Native metadata callbacks, including a fully hidden folded object.
use mdstream::*;
#[derive(Default)]
struct Sink {
    math: Vec<MathObjectMeta>,
    placeholders: usize,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, _: RenderRun<'_>) {}
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn math_object(&mut self, meta: MathObjectMeta) {
        self.math.push(meta)
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, _: FoldPlaceholder) {
        self.placeholders += 1;
    }
}
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
#[test]
fn actual_native_callbacks_include_an_empty_owner_and_stay_once_per_object() {
    let mut d = Document::new();
    d.append("\\(\\)\n\n\\[\nx\n\\]\n").unwrap();
    d.finish();
    let mut heights = HeightIndex::default();
    heights.rebuild(&d).unwrap();
    let mut pass = LayoutPass::new(&d, &heights, TileId(0), u64::MAX, true).unwrap();
    let mut sink = Sink::default();
    for _ in 0..1000 {
        let mut b = tiny();
        let completion = pass.advance(&d, &mut heights, &mut b, &mut sink).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if completion == Completion::EndOfDocument {
            break;
        }
    }
    assert_eq!(sink.math.len(), 2);
    assert_eq!(sink.math[0].delimiter, MathDelimiter::Paren);
    assert!(sink.math[0].content.is_empty());
    assert_eq!(sink.math[1].delimiter, MathDelimiter::Bracket);
    for meta in sink.math {
        assert_eq!(d.math_object(meta.id), Some(meta));
    }
}
#[test]
fn folded_native_pass_delivers_placeholder_without_hidden_math_callback() {
    let mut d = Document::new();
    d.append("# A\n\\[\n\\frac{a}{b}\n\\]\n# B\ntail\n")
        .unwrap();
    d.finish();
    let mut view = FoldView::<()>::with_state(&d, 1).unwrap();
    let id = FoldId::Heading(HeadingId(0));
    view.set_collapsed(&d, id, true).unwrap();
    let mut pass = LayoutPass::folded(&d, &view, TileId(0), u64::MAX).unwrap();
    let mut sink = Sink::default();
    for _ in 0..2000 {
        if pass
            .advance_folded(&d, &mut view, &mut tiny(), &mut sink)
            .unwrap()
            == Completion::EndOfDocument
        {
            break;
        }
    }
    assert_eq!(sink.placeholders, 1);
    assert!(sink.math.is_empty());
    assert!(view.is_collapsed(id));
}
