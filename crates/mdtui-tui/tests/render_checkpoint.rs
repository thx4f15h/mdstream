use std::mem::size_of;

use mdtui_tui::render::checkpoint::{
    CheckpointBlockKind, CheckpointPosition, CodeRenderState, ListMarkerState, PendingBlockEvents,
    RecoveryPlan, RenderCheckpoint, RenderCheckpointStore, TableRenderState,
};
use mdtui3::{
    BlockId, Budget, Completion, Document, HeightIndex, LayoutPass, LayoutSink, LinkId, LinkInfo,
    RenderRun, TileId, WorkLimit,
};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

#[test]
fn default_and_position_constructors_are_explicit() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<RenderCheckpoint>();
    assert!(size_of::<RenderCheckpoint>() <= 64);

    let initial = RenderCheckpoint::default();
    assert_eq!(initial, RenderCheckpoint::line_start());
    assert_eq!(initial.position, CheckpointPosition::LineStart);
    assert!(!initial.has_unfinished_line());
    assert_eq!(initial.line_source_start, None);
    assert_eq!(initial.block, None);
    assert_eq!(initial.code, CodeRenderState::None);
    assert_eq!(initial.table, TableRenderState::None);
    assert!(initial.pending.is_empty());

    let line = RenderCheckpoint::line_mid(42, 17);
    assert_eq!(line.position, CheckpointPosition::LineMid);
    assert!(line.has_unfinished_line());
    assert_eq!(line.line_source_start, Some(42));
    assert_eq!(line.line_display_width, 17);
    assert_eq!(line.fold_column, 17);

    let block = RenderCheckpoint::block_inner(BlockId(9), CheckpointBlockKind::Code);
    assert_eq!(block.position, CheckpointPosition::BlockInner);
    assert_eq!(block.block, Some(BlockId(9)));
    assert_eq!(block.block_kind, CheckpointBlockKind::Code);
}

#[test]
fn tile_keys_support_insert_get_invalidate_and_clear() {
    let document = parse(&"alpha\n".repeat(300));
    assert!(document.tile_count() > 4);
    let mut store = RenderCheckpointStore::new(&document);
    store
        .insert(TileId(0), RenderCheckpoint::line_start())
        .unwrap();
    store
        .insert(TileId(2), RenderCheckpoint::line_mid(10, 3))
        .unwrap();
    store
        .insert(
            TileId(4),
            RenderCheckpoint::block_inner(BlockId(7), CheckpointBlockKind::Table),
        )
        .unwrap();
    assert_eq!(store.len(), 3);
    assert!(store.exact(TileId(0)).is_some());
    assert!(store.exact(TileId(2)).is_some());
    assert!(store.exact(TileId(4)).is_some());

    store.invalidate(&document, 2..5).unwrap();
    assert!(store.exact(TileId(0)).is_some());
    assert!(store.exact(TileId(2)).is_none());
    assert!(store.exact(TileId(4)).is_none());

    store.clear(&document).unwrap();
    assert!(store.is_empty());
    assert!(store.exact(TileId(0)).is_none());
}

#[test]
fn same_tile_multiple_events_and_list_state_are_summarized() {
    let document = parse(&"alpha\n".repeat(300));
    let mut store = RenderCheckpointStore::new(&document);
    let checkpoint = RenderCheckpoint {
        block: Some(BlockId(3)),
        block_kind: CheckpointBlockKind::ListItem,
        list_indent: 4,
        list_marker: ListMarkerState::Ordered,
        list_marker_width: 3,
        list_body_indent: 7,
        list_continuation_indent: 5,
        quote_prefix: true,
        code: CodeRenderState::InfoLabel,
        table: TableRenderState::Header,
        pending: PendingBlockEvents {
            spacing_rows: 1,
            code_label: true,
            thematic_break: true,
            group_start: true,
        },
        ..RenderCheckpoint::line_start()
    };
    store.insert(TileId(1), checkpoint).unwrap();
    assert_eq!(store.exact(TileId(1)), Some(&checkpoint));
    assert!(!checkpoint.pending.is_empty());
}

#[test]
fn unfinished_cross_tile_line_fields_survive_storage() {
    let document = parse(&"alpha\n".repeat(300));
    let mut store = RenderCheckpointStore::new(&document);
    let checkpoint = RenderCheckpoint::line_mid(777, 41);
    store.insert(TileId(3), checkpoint).unwrap();
    let stored = store.exact(TileId(3)).unwrap();

    assert!(stored.has_unfinished_line());
    assert_eq!(stored.line_source_start, Some(777));
    assert_eq!(stored.line_display_width, 41);
    assert_eq!(stored.fold_column, 41);
}

#[test]
fn recovery_plan_distinguishes_exact_previous_and_missing() {
    let document = parse(&"alpha\n".repeat(300));
    let mut store = RenderCheckpointStore::new(&document);
    store
        .insert(TileId(0), RenderCheckpoint::line_start())
        .unwrap();
    store
        .insert(TileId(4), RenderCheckpoint::line_start())
        .unwrap();

    assert_eq!(
        store.previous(TileId(2)),
        Some((TileId(0), &RenderCheckpoint::line_start()))
    );
    assert_eq!(
        store.plan_recovery(TileId(2)),
        RecoveryPlan::ReplayFrom {
            from: TileId(0),
            target: TileId(2),
        }
    );
    assert_eq!(
        store.plan_recovery(TileId(4)),
        RecoveryPlan::Exact(TileId(4))
    );
    assert_eq!(
        store.plan_recovery(TileId(6)),
        RecoveryPlan::ReplayFrom {
            from: TileId(4),
            target: TileId(6),
        }
    );

    let empty = RenderCheckpointStore::new(&document);
    assert_eq!(
        empty.plan_recovery(TileId(5)),
        RecoveryPlan::ApproximateLineStart(TileId(5))
    );
}

#[test]
fn checkpoint_after_tile_only_writes_an_existing_next_tile() {
    let short = parse("short\n");
    let mut short_store = RenderCheckpointStore::new(&short);
    assert_eq!(
        short_store
            .checkpoint_after_tile(TileId(0), RenderCheckpoint::line_start(), &short)
            .unwrap(),
        None
    );
    assert!(short_store.exact(TileId(1)).is_none());

    let long = parse(&"alpha\n".repeat(300));
    let last = TileId(long.tile_count() - 1);
    let mut store = RenderCheckpointStore::new(&long);
    store
        .checkpoint_after_tile(TileId(0), RenderCheckpoint::line_start(), &long)
        .unwrap();
    assert!(store.exact(TileId(1)).is_some());
    assert_eq!(
        store
            .checkpoint_after_tile(last, RenderCheckpoint::line_start(), &long)
            .unwrap(),
        None
    );
}

#[test]
fn append_and_finish_invalidate_old_checkpoints_and_tokens() {
    let mut appended = Document::new();
    appended.append(&"alpha\n".repeat(300)).unwrap();
    let mut store = RenderCheckpointStore::new(&appended);
    store
        .insert(TileId(0), RenderCheckpoint::line_start())
        .unwrap();
    let update = appended.append("beta\n").unwrap();
    let earliest = update
        .changes
        .iter()
        .map(|change| change.source.start)
        .min()
        .expect("append must report a layout change");
    let earliest_tile = appended.tile_at(earliest).unwrap();
    store
        .insert(earliest_tile, RenderCheckpoint::line_mid(12, 4))
        .unwrap();
    let token = store.token(earliest_tile).unwrap();
    assert!(store.validates(token));
    store.apply(&appended, &update).unwrap();
    assert!(store.exact(earliest_tile).is_none());
    assert!(!store.validates(token));

    let mut finished = Document::new();
    finished.append("```rust\nunclosed").unwrap();
    let mut finish_store = RenderCheckpointStore::new(&finished);
    finish_store
        .insert(TileId(0), RenderCheckpoint::line_start())
        .unwrap();
    let finish_update = finished.finish();
    finish_store.apply(&finished, &finish_update).unwrap();
    assert!(finish_store.exact(TileId(0)).is_none());
}

#[derive(Default)]
struct ProbeSink {
    current: Option<TileId>,
    target: Option<TileId>,
    begins: Vec<TileId>,
    target_begins: usize,
    state: RenderCheckpoint,
}

impl LayoutSink for ProbeSink {
    fn begin_tile(&mut self, tile: TileId) {
        self.current = Some(tile);
        self.begins.push(tile);
        if Some(tile) == self.target {
            self.target_begins += 1;
        }
    }

    fn text(&mut self, run: RenderRun<'_>) {
        debug_assert!(self.current.is_some());
        self.state.position = CheckpointPosition::LineMid;
        self.state.line_display_width = self
            .state
            .line_display_width
            .saturating_add(u32::try_from(run.text.len()).unwrap_or(u32::MAX));
    }

    fn object(&mut self, _: LinkId, _: &LinkInfo) {}

    fn end_tile(&mut self, _: TileId) -> u64 {
        self.current = None;
        0
    }
}

#[test]
fn earlier_checkpoint_replays_to_target_with_one_target_begin() {
    let document = parse(&"alpha\n".repeat(300));
    assert!(document.tile_count() > 4);
    let target = TileId(3);
    let mut store = RenderCheckpointStore::new(&document);
    let checkpoint = RenderCheckpoint {
        quote_prefix: true,
        pending: PendingBlockEvents {
            spacing_rows: 1,
            ..PendingBlockEvents::default()
        },
        ..RenderCheckpoint::line_mid(5, 8)
    };
    store.insert(TileId(0), checkpoint).unwrap();
    let RecoveryPlan::ReplayFrom {
        from,
        target: planned,
    } = store.plan_recovery(target)
    else {
        panic!("target must require replay");
    };
    assert_eq!(planned, target);
    assert!(from < target);

    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut pass =
        LayoutPass::from_checkpoint(&document, &heights, from, target, u64::MAX, usize::MAX)
            .unwrap()
            .unwrap();
    let mut sink = ProbeSink {
        target: Some(target),
        state: *store.exact(from).unwrap(),
        ..ProbeSink::default()
    };
    assert_eq!(
        pass.advance(&document, &mut heights, &mut unlimited(), &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.target_begins, 1);
    assert!(sink.begins.contains(&from));
    assert!(sink.begins.contains(&target));
    assert_eq!(sink.state.quote_prefix, checkpoint.quote_prefix);
    assert_eq!(sink.state.pending, checkpoint.pending);
}
