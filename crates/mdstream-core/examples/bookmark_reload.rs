//! Explicit host mapping of one bookmark; no automatic reload or disk format.
#[path = "support/bookmark_reload.rs"]
#[allow(dead_code)]
mod host;

use host::{Binding, MapStep, PointMap, Restore, Saved, Version, host_step};
use mdstream::*;

fn main() {
    let mut old = Document::new();
    old.append("# Same\nalpha\n# Same\n中文\n").unwrap();
    old.finish();
    let old_version = Version {
        resource: 7,
        content: 1,
    };
    let old_bookmark = Bookmark::new(&old, 20, 9).unwrap();
    let saved = Saved::capture(&old, Binding::new(&old, old_version), old_bookmark).unwrap();

    let mut new = Document::new();
    new.append("intro\n# Same\nalpha\n# Same\n中文\n").unwrap();
    new.finish();
    let new_version = Version {
        resource: 7,
        content: 2,
    };
    let mut view = FoldView::new(&new, 1).unwrap();
    let containing = new.fold_candidate_at(26).unwrap().id;
    view.set_collapsed(&new, containing, true).unwrap();
    assert!(matches!(
        old_bookmark.navigation(&new, &view, 3),
        Err(LayoutError::WrongDocument)
    ));

    // Supplied by the host's editor/source-map: old byte 20 -> new byte 26.
    // No title lookup or automatic diff is performed by this prototype.
    let mut task = Restore::new(
        saved,
        &new,
        Binding::new(&new, new_version),
        PointMap {
            from: old_version,
            source: 20,
            to: new_version,
            steps: &[MapStep::Working, MapStep::Exact(26)],
        },
        3,
    )
    .unwrap();
    // A lazy host history selects only this destination; it keeps its cursor
    // unchanged until the source navigation succeeds. Other entries are untouched.
    let mut active_slot = 1;
    assert!(
        task.advance(&new, &mut view, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .is_none()
    );
    for frame in 1..=16 {
        let mut budget = Budget::new(host_step());
        let result = task.advance(&new, &mut view, &mut budget).unwrap();
        println!(
            "frame={frame} map_cursor={} work={:?} located={}",
            task.map_position(),
            budget.used(),
            result.is_some()
        );
        if let Some((bookmark, target)) = result {
            active_slot = 0;
            assert_eq!(bookmark.anchor().document_id(), new.id());
            assert_eq!(target.source, Some(26));
            println!(
                "slot={active_slot} source={} saved_offset={} top_if_local_row_12={}",
                bookmark.anchor().source(),
                bookmark.viewport_offset(),
                bookmark.viewport_top(12, 3)
            );
            return;
        }
        assert_eq!(active_slot, 1);
    }
    panic!("restore did not finish within the hand-calculated bound");
}
