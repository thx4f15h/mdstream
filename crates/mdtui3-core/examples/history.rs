//! Source bookmarks and transactional back/forward navigation without a UI.
use mdtui3::*;

fn travel(
    doc: &Document,
    view: &mut FoldView,
    history: &mut NavigationHistory,
    nav: &mut Navigator,
    direction: HistoryDirection,
) -> Result<(), LayoutError> {
    let Some(mut task) = history.navigation(doc, view, direction, 4)? else {
        println!("{direction:?}: end of history");
        return Ok(());
    };
    let mut frames = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let progress = task.advance(doc, view, history, &mut budget)?;
        frames += 1;
        if let Some(target) = progress.target {
            nav.apply_target(target);
            println!(
                "{direction:?}: source={}, saved offset={}, history={}, frames={frames}",
                target.source.unwrap(),
                task.bookmark().viewport_offset(),
                history.position(),
            );
            return Ok(());
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Guide\n## Details\n```rs\nlet value = 1;\n```\n# Next\ntail\n")?;
    let source = doc.source().find("let value").unwrap();
    let saved = Bookmark::new(&doc, source, 2)?;
    let mut history = NavigationHistory::new(saved);
    let mut view = FoldView::new(&doc, 1)?;
    let mut nav = Navigator::new();

    let next = doc.headings().get(HeadingId(2)).unwrap().source.start;
    let mut jump = Bookmark::new(&doc, next, 0)?.navigation(&doc, &view, 4)?;
    let target = jump
        .advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))?
        .target
        .unwrap();
    nav.apply_target(target);
    history.push(Bookmark::new(&doc, target.source.unwrap(), 0)?)?;

    for id in [
        FoldId::Heading(HeadingId(0)),
        FoldId::Heading(HeadingId(1)),
        saved.target().unwrap(),
    ] {
        view.set_collapsed(&doc, id, true)?;
    }
    let update = doc.append("streaming tail\n")?;
    view.apply(&doc, &update)?;
    view.reset(3)?;
    travel(
        &doc,
        &mut view,
        &mut history,
        &mut nav,
        HistoryDirection::Back,
    )?;
    assert_eq!(history.current(), saved);
    travel(
        &doc,
        &mut view,
        &mut history,
        &mut nav,
        HistoryDirection::Forward,
    )?;
    travel(
        &doc,
        &mut view,
        &mut history,
        &mut nav,
        HistoryDirection::Forward,
    )?;
    Ok(())
}
