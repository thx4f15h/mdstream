//! Fold a multi-leaf container paragraph and resume at its scope boundary.
use mdtui3::*;

#[derive(Default)]
struct Sink {
    text: String,
    rows: u64,
    ends: usize,
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.text.push_str(run.text);
            self.rows += run.text.bytes().filter(|&b| b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        assert_eq!(p.paragraph, Some(BlockId(0)));
        self.text.push_str("[...]\n");
        self.rows += 1;
    }
    fn paragraph_end(&mut self, _: BlockId) {
        self.ends += 1;
    }
}
fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 4,
        objects: 2,
        tiles: 1,
        bytes: 8,
        ..Default::default()
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> - Preview: first line\n>   second line\n")?;
    let mut view = FoldView::new(&doc, 1)?;
    let mut task = SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 13)?;
    let id = loop {
        if let Some(id) = task.advance(&doc, &mut view, &mut frame())?.id {
            break id;
        }
    };
    view.set_collapsed(&doc, id, true)?;
    let update = doc.append(&format!(">   {}", "x".repeat(1_000_000)))?;
    view.apply(&doc, &update)?;
    assert!(view.fold_candidate(&doc, id)?.unwrap().open);
    let mut update = doc.append("\n> - Following item\n")?;
    update.merge(doc.finish())?;
    view.apply(&doc, &update)?;
    assert!(!view.fold_candidate(&doc, id)?.unwrap().open);
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20)?;
    let mut sink = Sink::default();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = frame();
        let status = pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)?;
        bytes += budget.used().bytes;
        if status == Completion::EndOfDocument {
            break;
        }
        assert!(frames < 100);
    }
    assert_eq!(sink.text, "Preview: [...]\nFollowing item\n");
    assert_eq!(sink.ends, 1);
    assert!(bytes < 64);
    println!(
        "{}frames={frames}, charged bytes={bytes}, paragraph endings={}",
        sink.text, sink.ends
    );
    Ok(())
}
