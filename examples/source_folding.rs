//! Register a host selection, append outside it, then restore its source anchor.
use mdtui3::*;

#[derive(Default)]
struct Sink {
    text: String,
    rows: u64,
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
    fn placeholder(&mut self, _: FoldPlaceholder) {
        self.text.push_str("[host selection]\n");
        self.rows += 1;
    }
}
fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 4,
        objects: 4,
        bytes: 16,
        tiles: 2,
        ..Default::default()
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Report\nvisible prefix\n\n")?;
    let start = doc.source().len();
    doc.append(&"x".repeat(1_000_000))?;
    doc.append("\n\n")?; // Locally close the paragraph; append can still continue.
    let end = doc.source_fold_limit();
    let mut view = FoldView::new(&doc, 1)?;
    let anchor = FoldAnchor::new(&doc, start + 100)?;
    let mut registration = SourceFoldRegistration::new(&doc, &view, Span::new(start, end))?;
    let mut registration_frames = 0;
    let id = loop {
        registration_frames += 1;
        if let Some(id) = registration.advance(&doc, &mut view, &mut frame())?.id {
            break id;
        }
    };
    view.set_collapsed(&doc, id, true)?;
    let update = doc.append("appended outside the selection\n")?;
    view.apply(&doc, &update)?;
    assert_eq!(view.fold_candidate(&doc, id)?.unwrap().hidden.end, end);
    assert_eq!(
        anchor
            .resolve(&doc, &view, 20, &mut frame())?
            .unwrap()
            .hidden_by,
        Some(id)
    );
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
    assert!(!sink.text.contains("xxx") && sink.text.contains("appended outside"));
    assert!(bytes < 100);
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(anchor.source()),
        FoldNavigationPolicy::Reveal,
        20,
    )?;
    let mut navigation_frames = 0;
    loop {
        navigation_frames += 1;
        let p = navigation.advance(&doc, &mut view, &mut frame())?;
        if p.completion == Completion::Covered {
            assert_eq!(p.target.unwrap().source, Some(anchor.source()));
            break;
        }
    }
    assert!(!view.is_collapsed(id));
    println!(
        "{}registration frames={registration_frames}, layout frames={frames}, charged bytes={bytes}, navigation frames={navigation_frames}",
        sink.text
    );
    Ok(())
}
