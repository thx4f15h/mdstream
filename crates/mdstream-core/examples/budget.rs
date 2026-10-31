//! Minimal layout consumer. Counts Unicode scalars, not terminal display columns;
//! production shaping must supply its own bounded LayoutSink implementation.
use mdstream::*;

#[derive(Default)]
struct Lines {
    column: usize,
    rows: u64,
}
impl LayoutSink for Lines {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        if run.style.contains(Style::HIDDEN) {
            return;
        }
        for ch in run.text.chars() {
            if ch == '\n' {
                self.column = 0;
                self.rows += 1;
            } else {
                self.column += 1;
                if self.column == 40 {
                    self.column = 0;
                    self.rows += 1;
                }
            }
        }
    }
    fn object(&mut self, _: LinkId, info: &LinkInfo) {
        if info.kind == LinkKind::Image && info.label.is_empty() {
            self.rows += 1;
        }
    }
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut document = Utf8Document::new();
    let mut heights = HeightIndex::default();
    let mut batch = Update::default();
    for chunk in "# Budgeted viewport\n\n中文🙂 with **streaming** input.\n![](image.png)\n"
        .as_bytes()
        .chunks(3)
    {
        batch.merge(document.append(chunk)?)?;
    }
    batch.merge(document.finish())?;
    let doc = document.document();
    heights.apply(doc, &batch)?;
    let mut pass = LayoutPass::new(doc, &heights, TileId(0), 24, false)?;
    let mut sink = Lines::default();
    for frame in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 12,
            tiles: 4,
            bytes: 16,
            objects: 8,
            recovery: 4,
            dependencies: 4,
        });
        let completion = pass.advance(doc, &mut heights, &mut budget, &mut sink)?;
        println!(
            "frame={frame} source={} status={completion:?} used={:?}",
            pass.source_position(),
            budget.used()
        );
        if completion != Completion::BudgetExhausted {
            break;
        }
    }
    Ok(())
}
