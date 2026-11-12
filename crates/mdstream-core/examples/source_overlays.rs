//! Independent nested/crossing intent, pending growth and explicit reload.
use mdstream::*;

fn frame() -> Budget {
    Budget::new(WorkLimit {
        queries: 4,
        objects: 4,
        recovery: 4,
        ..Default::default()
    })
}
fn register(d: &Document, v: &mut FoldView, mut task: SourceFoldRegistration) -> FoldId {
    loop {
        if let Some(id) = task.advance(d, v, &mut frame()).unwrap().id {
            return id;
        }
    }
}
fn main() {
    let mut d = Document::new();
    d.append("> unfinished").unwrap();
    let mut v = FoldView::new(&d, 1).unwrap();
    let task = SourceFoldRegistration::stream(&d, &v, 2).unwrap();
    let tail = register(&d, &mut v, task);
    v.set_collapsed(&d, tail, true).unwrap();
    let update = d.append(" paragraph\n> more\n\noutside\n").unwrap();
    v.apply(&d, &update).unwrap();
    let task = SourceFoldRegistration::overlapping(&d, &v, Span::new(5, 14)).unwrap();
    let child = register(&d, &mut v, task);
    v.set_collapsed(&d, child, true).unwrap();
    v.set_collapsed(&d, tail, false).unwrap();
    assert!(v.is_collapsed(child));
    let FoldId::Source(id) = child else {
        unreachable!()
    };
    let saved = SourceFoldSnapshot::capture(&d, &v, id, SourceFoldHost::Document).unwrap();
    let mut reload = Document::new();
    reload.append(d.source()).unwrap();
    let mut after = FoldView::new(&reload, 1).unwrap();
    let mut recovery =
        SourceFoldRestore::new(&reload, &after, &saved, SourceFoldHost::Document).unwrap();
    let restored = loop {
        if let Some(id) = recovery
            .advance(&reload, &mut after, &mut frame())
            .unwrap()
            .id
        {
            break id;
        }
    };
    assert_ne!(restored, child);
    assert!(after.is_collapsed(restored));
    println!(
        "Reload mapping: {:?} -> {restored:?}",
        recovery.previous_id()
    );
    println!("Source cuts and local intent restored without modifying Markdown.");
}
