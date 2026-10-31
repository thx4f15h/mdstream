use mdstream::*;

#[derive(Default)]
struct Sink {
    text: String,
    placeholders: usize,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {}
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.text.push_str(run.text);
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        1
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, _: FoldPlaceholder) {
        self.placeholders += 1;
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        heading_levels: [true; 6],
        ..Default::default()
    });
    for chunk in ["> **Streaming\n", "> heading**\n", "> ===\n"] {
        let update = doc.append(chunk)?;
        view.apply(&doc, &update)?;
        println!(
            "new headings {:?}, changed blocks {:?}",
            update.new_headings, update.changed_blocks
        );
    }
    let title = doc.headings().entries()[0];
    assert_eq!(title.block, BlockId(0));
    assert_eq!(doc.blocks().len(), 2);
    assert!(title.source.end > doc.blocks()[0].source.end);
    let update = doc.append(&format!("> {}\n", "hidden".repeat(200_000)))?;
    view.apply(&doc, &update)?;
    let update = doc.finish();
    view.apply(&doc, &update)?;
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 10)?;
    let mut sink = Sink::default();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            tiles: 1,
            ..Default::default()
        });
        let done = pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)?;
        bytes += budget.used().bytes;
        if done == Completion::EndOfDocument {
            break;
        }
        assert!(frames < 100);
    }
    assert_eq!(sink.text, "Streaming\nheading\n");
    assert_eq!(sink.placeholders, 1);
    assert_eq!(bytes, title.source.len());
    println!(
        "{} physical leaves; title {:?}; {frames} frames/{bytes} source bytes, hidden body skipped",
        doc.blocks().len(),
        title.source
    );
    Ok(())
}
