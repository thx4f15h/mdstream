//! Keep a header and selected rows visible while folding a fixed body interval.
use mdstream::*;

#[derive(Default)]
struct Sink {
    text: String,
    rows: u64,
    placeholders: usize,
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
    fn placeholder(&mut self, placeholder: FoldPlaceholder) {
        let FoldId::TableRows(range) = placeholder.candidate.id else {
            panic!()
        };
        self.text
            .push_str(&format!("[rows {}..{}]\n", range.start, range.end));
        self.rows += 1;
        self.placeholders += 1;
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("| Name |\n| - |\n| first |\n")?;
    doc.append(&format!("| {}", "x".repeat(1_000_000)))?;
    let mut view = FoldView::new(&doc, 1)?;
    let rows = view.insert_table_rows(&doc, TableId(0), 2..3)?;
    view.set_collapsed(&doc, rows, true)?;
    // Completing the selected row grows its source range, but later rows remain visible.
    let update = doc.append(" end |\n| appended |\n")?;
    view.apply(&doc, &update)?;
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)?;
    view.set_collapsed(&doc, FoldId::Table(TableId(0)), false)?;
    assert!(view.is_collapsed(rows));
    let update = doc.finish();
    view.apply(&doc, &update)?;
    let mut pass = LayoutPass::folded(&doc, &view, TileId(0), 20)?;
    let mut sink = Sink::default();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 4,
            objects: 4,
            bytes: 16,
            tiles: 2,
            ..Default::default()
        });
        let completion = pass.advance_folded(&doc, &mut view, &mut budget, &mut sink)?;
        bytes += budget.used().bytes;
        if completion == Completion::EndOfDocument {
            break;
        }
        assert!(frames < 100);
    }
    assert_eq!(sink.placeholders, 1);
    assert!(sink.text.contains("first") && sink.text.contains("appended"));
    assert!(!sink.text.contains('x'));
    assert!(bytes < 100);
    println!("{}frames={frames}, charged bytes={bytes}", sink.text);
    Ok(())
}
