//! Fold an open paragraph, follow a million-byte append, and resume wrapping.
use mdstream::*;

#[derive(Default)]
struct Sink {
    text: String,
    column: usize,
    rows: u64,
    endings: usize,
}
impl Sink {
    fn push(&mut self, ch: char) {
        if ch == '\n' {
            self.rows += 1;
            self.column = 0;
        } else {
            if self.column == 12 {
                self.text.push('\n');
                self.rows += 1;
                self.column = 0;
            }
            self.column += 1;
        }
        self.text.push(ch);
    }
}
impl LayoutSink for Sink {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            for ch in run.text.chars() {
                self.push(ch);
            }
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Sink {
    fn placeholder(&mut self, p: FoldPlaceholder) {
        for ch in "[...]".chars() {
            self.push(ch);
        }
        if p.paragraph.is_some() {
            // The terminating line belongs to this placeholder's owner tile.
            self.push('\n');
        }
    }
    fn paragraph_end(&mut self, _: BlockId) {
        // Also works when a local pass starts inside the hidden suffix and
        // never emits the placeholder. This callback adds no measured height.
        self.column = 0;
        self.endings += 1;
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
    doc.append("A long paragraph: body\n")?;
    let mut view = FoldView::new(&doc, 1)?;
    let mut registration = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 18)?;
    let id = loop {
        if let Some(id) = registration.advance(&doc, &mut view, &mut frame())?.id {
            break id;
        }
    };
    view.set_collapsed(&doc, id, true)?;
    let update = doc.append(&"x".repeat(1_000_000))?;
    view.apply(&doc, &update)?;
    assert!(view.fold_candidate(&doc, id)?.unwrap().open);
    let update = doc.append("\n\n")?;
    view.apply(&doc, &update)?;
    let candidate = view.fold_candidate(&doc, id)?.unwrap();
    assert!(!candidate.open);
    let update = doc.append("Next paragraph starts here.\n")?;
    view.apply(&doc, &update)?;
    assert_eq!(view.fold_candidate(&doc, id)?, Some(candidate));
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
    assert_eq!(sink.endings, 1);
    assert!(bytes < 64 && !sink.text.contains("xxx"));
    assert!(sink.text.contains("Next paragra\nph starts he\nre."));
    println!(
        "{}frames={frames}, charged bytes={bytes}, paragraph endings={}",
        sink.text, sink.endings
    );
    Ok(())
}
