//! A bounded source-text consumer illustrating folding, not terminal shaping.
use mdtui3::*;

#[derive(Default)]
struct Lines {
    output: String,
    rows: u64,
}
impl LayoutSink for Lines {
    fn begin_tile(&mut self, _: TileId) {
        self.rows = 0;
    }
    fn text(&mut self, run: RenderRun<'_>) {
        if !run.style.contains(Style::HIDDEN) {
            self.output.push_str(run.text);
            self.rows += run.text.bytes().filter(|&b| b == b'\n').count() as u64;
        }
    }
    fn object(&mut self, _: LinkId, _: &LinkInfo) {}
    fn end_tile(&mut self, _: TileId) -> u64 {
        self.rows
    }
}
impl FoldLayoutSink for Lines {
    fn placeholder(&mut self, placeholder: FoldPlaceholder) {
        use std::fmt::Write;
        writeln!(
            self.output,
            "[fold {:?}, source {}..{}]",
            placeholder.candidate.id,
            placeholder.candidate.hidden.start,
            placeholder.candidate.hidden.end
        )
        .unwrap();
        self.rows += 1;
    }
}
fn show(document: &Document, view: &mut FoldView) -> Result<(), LayoutError> {
    let mut pass = LayoutPass::folded(document, view, TileId(0), 24)?;
    let mut sink = Lines::default();
    let mut frames = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 8,
            tiles: 2,
            bytes: 32,
            objects: 4,
            ..Default::default()
        });
        frames += 1;
        if pass.advance_folded(document, view, &mut budget, &mut sink)?
            != Completion::BudgetExhausted
        {
            break;
        }
    }
    print!(
        "{}frames={frames}, visible rows={}\n\n",
        sink.output,
        view.heights().total_rows()
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut document = Document::new();
    document.append("# Guide\nintro\n## Details\n")?;
    document.append(&"hidden body\n".repeat(100_000))?;
    let mut view = FoldView::new(&document, 1)?;
    view.set_collapsed(&document, FoldId::Heading(HeadingId(1)), true)?;
    view.set_collapsed(&document, FoldId::Heading(HeadingId(0)), true)?;
    let update = document.append("more hidden input\n# Next\nvisible tail\n")?;
    view.apply(&document, &update)?;
    let update = document.finish();
    view.apply(&document, &update)?;
    println!("Parent collapsed:");
    show(&document, &mut view)?;
    view.set_collapsed(&document, FoldId::Heading(HeadingId(0)), false)?;
    println!("Parent expanded; child remains collapsed:");
    show(&document, &mut view)?;

    // A saved source position survives projection onto a fold placeholder.
    let anchor = FoldAnchor::new(
        &document,
        document.source().find("hidden body").unwrap() + 7,
    )?;
    let mut navigator = Navigator::new();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    });
    let projected = anchor.resolve(&document, &view, 24, &mut budget)?.unwrap();
    println!(
        "Saved source {}, projected to {:?}",
        anchor.source(),
        projected
    );
    navigator.apply_target(projected.target);
    // A search result or link destination can use the same exact source action.
    let mut navigation = FoldNavigation::new(
        &document,
        &view,
        &navigator,
        NavAction::Source(anchor.source()),
        FoldNavigationPolicy::Reveal,
        24,
    )?;
    let mut steps = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        steps += 1;
        let progress = navigation.advance(&document, &mut view, &mut budget)?;
        if let Some(change) = progress.change {
            println!("Opened path: {change:?}");
        }
        if progress.completion == Completion::Covered {
            let target = progress.target.unwrap();
            navigator.apply_target(target);
            println!("Source restored in {steps} steps: {target:?}");
            break;
        }
    }
    Ok(())
}
