//! P07 executable map for `docs/tui-render-todo.md` §4.1:
//! - §4.1.1: `empty_info_fence_is_skipped_by_layout_pass`
//! - §4.1.2: `raw_runs_differ_from_display_and_cursor`
//! - §4.1.3: `width_change_requires_reset_and_clear`
//! - §4.1.4: `append_invalidates_from_earliest_layout_tile_to_end`
//! - §4.1.5: `heading_anchors_stale_after_append`
//! - §4.1.6: `checkpoint_payload_ownership_survives_until_drain_and_compaction`
//! - §4.1.7: `sidebar_excerpt_is_budgeted_and_truncated`
//! - §4.1.8: `style_bit_union_loses_nesting_and_math_is_flagged`
//! - §4.1.9: `end_tile_tracks_tile_boundaries_not_markdown_lines`
//! - §4.1.10: `checkpoint_keys_are_tile_ids`, `checkpoint_replay_emits_target_once`
//! - §4.1.11: `display_cursor_stale_rules`
//! - §4.1.12: `drain_before_keeps_future_checkpoints`
//! - §4.1.13: `link_destination_stays_raw_and_fragments_decode_only_percent`
//! - §4.1.14: P04 `screen` tests pin content-area clipping; this file's
//!   width-state probe covers the explicit reset/clear adaptation.
//!
//! Status: all probes pass on 2026-10-31; the P10 gate is open.

use std::cell::Cell as DropCell;
use std::rc::Rc;

use mdtui3::{
    BlockKind, Budget, CheckpointIndex, Completion, DisplayExcerpt, DisplayStep, Document,
    ExcerptLimits, ExcerptStep, FragmentStep, HeadingAnchors, HeadingId, HeightIndex, HeightState,
    LayoutError, LayoutPass, LayoutSink, LinkId, LinkInfo, RenderRun, Span, Style, TILE_BYTES,
    TileId, WorkLimit,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Begin(TileId),
    Text(TileId, String),
    Object(TileId, LinkId),
    End(TileId, u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndSnapshot {
    tile: TileId,
    bytes: usize,
    newlines: usize,
}

#[derive(Default)]
struct RecordingSink {
    current: Option<TileId>,
    events: Vec<Event>,
    begins: Vec<TileId>,
    ends: Vec<EndSnapshot>,
    delivered_bytes: usize,
    delivered_newlines: usize,
    rows_per_tile: u64,
}

impl RecordingSink {
    fn visible_text(&self) -> String {
        let mut text = String::new();
        for event in &self.events {
            if let Event::Text(_, event_text) = event {
                text.push_str(event_text);
            }
        }
        text
    }

    fn events_for(&self, tile: TileId) -> Vec<Event> {
        self.events
            .iter()
            .filter(|event| match event {
                Event::Begin(event_tile)
                | Event::Text(event_tile, _)
                | Event::Object(event_tile, _)
                | Event::End(event_tile, _) => *event_tile == tile,
            })
            .cloned()
            .collect()
    }
}

impl LayoutSink for RecordingSink {
    fn begin_tile(&mut self, tile: TileId) {
        self.current = Some(tile);
        self.begins.push(tile);
        self.events.push(Event::Begin(tile));
    }

    fn text(&mut self, run: RenderRun<'_>) {
        let tile = self.current.expect("text must follow begin_tile");
        self.delivered_bytes += run.text.len();
        self.delivered_newlines += run.text.bytes().filter(|byte| *byte == b'\n').count();
        self.events.push(Event::Text(tile, run.text.to_owned()));
    }

    fn object(&mut self, id: LinkId, _: &LinkInfo) {
        let tile = self.current.expect("object must follow begin_tile");
        self.events.push(Event::Object(tile, id));
    }

    fn end_tile(&mut self, tile: TileId) -> u64 {
        self.ends.push(EndSnapshot {
            tile,
            bytes: self.delivered_bytes,
            newlines: self.delivered_newlines,
        });
        self.events.push(Event::End(tile, self.rows_per_tile));
        self.current = None;
        self.rows_per_tile
    }
}

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn display_visible(document: &Document, span: Span) -> String {
    document
        .display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn cursor_visible(document: &Document, span: Span) -> String {
    let mut cursor = document.display_cursor(span).unwrap();
    let mut text = String::new();
    loop {
        let mut budget = unlimited();
        match cursor.next(document, &mut budget).unwrap() {
            DisplayStep::Run(run) => {
                if !run.style.contains(Style::HIDDEN) {
                    text.push_str(run.text);
                }
            }
            DisplayStep::Complete => return text,
            DisplayStep::BudgetExhausted => panic!("unlimited display budget exhausted"),
        }
    }
}

fn build_anchors(document: &Document) -> HeadingAnchors {
    let mut index = HeadingAnchors::new(document);
    let mut budget = unlimited();
    assert_eq!(
        index.advance(document, &mut budget).unwrap(),
        Completion::EndOfDocument
    );
    index
}

fn resolve(index: &HeadingAnchors, document: &Document, fragment: &str) -> FragmentStep {
    let mut lookup = index.fragment(fragment).unwrap();
    loop {
        let mut budget = unlimited();
        match lookup.advance(document, &mut budget).unwrap() {
            FragmentStep::BudgetExhausted => continue,
            result => return result,
        }
    }
}

fn visible_run_style(document: &Document, needle: &str) -> Style {
    let mut styles = document
        .runs_in(Span::new(0, document.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN) && run.text == needle)
        .map(|run| run.style);
    let style = styles
        .next()
        .unwrap_or_else(|| panic!("missing visible run {needle:?}"));
    assert!(styles.next().is_none(), "duplicate visible run {needle:?}");
    style
}

#[test]
fn empty_info_fence_is_skipped_by_layout_pass() {
    let document = parse("```rust\n```\n");
    let code = document
        .blocks()
        .iter()
        .find(|block| matches!(block.kind, BlockKind::Code { .. }))
        .expect("info-only fence must produce a code block");
    let BlockKind::Code { info } = &code.kind else {
        unreachable!()
    };
    assert_eq!(info.map(|span| document.text(span).unwrap()), Some("rust"));
    assert_eq!(document.next_drawable(0), None);

    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut pass = LayoutPass::new(&document, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = RecordingSink::default();
    assert_eq!(
        pass.advance(&document, &mut heights, &mut unlimited(), &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert!(sink.events.is_empty());
    assert_eq!(heights.height(TileId(0)), Some(0));
    assert_eq!(heights.height_state(TileId(0)), Some(HeightState::Measured));

    let control = parse("visible text\n");
    let mut control_heights = HeightIndex::default();
    control_heights.rebuild(&control).unwrap();
    let mut control_pass =
        LayoutPass::new(&control, &control_heights, TileId(0), u64::MAX, false).unwrap();
    let mut control_sink = RecordingSink::default();
    control_pass
        .advance(
            &control,
            &mut control_heights,
            &mut unlimited(),
            &mut control_sink,
        )
        .unwrap();
    assert!(!control_sink.events.is_empty());
}

#[test]
fn raw_runs_differ_from_display_and_cursor() {
    let source = "a &amp; b\n\n`  a  \t`\n";
    let document = parse(source);
    let whole = Span::new(0, source.len());
    let raw: String = document
        .runs_in(whole)
        .unwrap()
        .map(|run| run.text)
        .collect();
    assert_eq!(raw, source);

    let displayed = display_visible(&document, whole);
    assert_ne!(displayed, raw);
    assert!(displayed.contains("a & b"));
    assert!(!displayed.contains("&amp;"));
    assert_eq!(cursor_visible(&document, whole), displayed);

    let code_start = source.find('`').unwrap();
    let code_end = source.rfind('`').unwrap() + 1;
    let code_span = Span::new(code_start, code_end);
    let raw_code: String = document
        .runs_in(code_span)
        .unwrap()
        .map(|run| run.text)
        .collect();
    let display_code = display_visible(&document, code_span);
    assert_ne!(raw_code, display_code);
    assert!(raw_code.contains("  "));
    assert!(raw_code.contains('\t'));
    assert!(!display_code.contains('`'));
}

#[test]
fn width_change_requires_reset_and_clear() {
    let document = parse(&"line\n".repeat(200));
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&document);
    let tile = TileId(0);
    let ticket = heights.ticket(tile).unwrap();
    heights.measure(ticket, 7).unwrap();
    checkpoints.insert(tile, 11).unwrap();
    let token = checkpoints.token(tile).unwrap();

    heights.reset(3).unwrap();
    assert_eq!(heights.height(tile), Some(3));
    assert_eq!(heights.height_state(tile), Some(HeightState::Estimated));
    assert_eq!(
        heights.measure(ticket, 7),
        Err(LayoutError::StaleMeasurement)
    );
    assert_eq!(checkpoints.get(tile), Some(&11));

    checkpoints.clear();
    assert!(!checkpoints.validates(token));
    assert_eq!(checkpoints.get(tile), None);
    assert_eq!(heights.reset(0), Err(LayoutError::ZeroEstimate));
}

#[test]
fn sidebar_excerpt_is_budgeted_and_truncated() {
    let document = parse("# A very long heading with **bold** content for sidebar budgets\n");
    let heading = document.headings().entries()[0].content;
    let mut excerpt = DisplayExcerpt::new(
        &document,
        heading,
        ExcerptLimits {
            source_bytes: 32,
            display_bytes: 6,
            lines: 1,
        },
    )
    .unwrap();
    let mut empty = Budget::new(WorkLimit::default());
    assert_eq!(
        excerpt.next(&document, &mut empty),
        Ok(ExcerptStep::BudgetExhausted)
    );
    assert_eq!(empty.used(), WorkLimit::default());

    let mut output = String::new();
    loop {
        let mut budget = unlimited();
        match excerpt.next(&document, &mut budget).unwrap() {
            ExcerptStep::Run(run) => output.push_str(run.text),
            ExcerptStep::Complete(_) => break,
            ExcerptStep::BudgetExhausted => panic!("unlimited excerpt budget exhausted"),
        }
    }
    assert!(output.len() <= 6);
    assert!(excerpt.source_position() <= heading.start + 32);
}

#[test]
fn append_invalidates_from_earliest_layout_tile_to_end() {
    let mut document = Document::new();
    document
        .append(&format!("{}\n\nTarget line\n", "p".repeat(700)))
        .unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut checkpoints = CheckpointIndex::new();
    checkpoints.rebuild(&document);
    let old_len = document.tile_count();
    assert!(old_len >= 3);

    let mut tickets = Vec::new();
    for index in 0..old_len {
        let tile = TileId(index);
        let ticket = heights.ticket(tile).unwrap();
        heights
            .measure(ticket, u64::try_from(index % 3 + 1).unwrap())
            .unwrap();
        checkpoints.insert(tile, index).unwrap();
        tickets.push(ticket);
    }
    let beyond = TileId(old_len + 32);
    checkpoints.insert(beyond, 999).unwrap();

    let update = document.append("===\n").unwrap();
    let first_changed = update
        .changes
        .iter()
        .map(|change| change.source.start)
        .min()
        .expect("append must report a layout change");
    let earliest = document.tile_at(first_changed).unwrap();
    assert!(earliest.0 > 0 && earliest.0 < old_len);

    heights.apply(&document, &update).unwrap();
    checkpoints.apply(&document, &update).unwrap();
    for index in 0..old_len {
        let tile = TileId(index);
        if index < earliest.0 {
            assert_eq!(heights.height_state(tile), Some(HeightState::Measured));
            assert_eq!(checkpoints.get(tile), Some(&index));
        } else {
            assert_eq!(heights.height_state(tile), Some(HeightState::Expired));
            assert_eq!(checkpoints.get(tile), None);
        }
    }
    assert_eq!(checkpoints.get(beyond), None);
    assert_eq!(
        heights.measure(tickets[earliest.0], 9),
        Err(LayoutError::StaleMeasurement)
    );
}

#[test]
fn heading_anchors_stale_after_append() {
    let mut document = Document::new();
    document.append("# First\n").unwrap();
    let mut index = build_anchors(&document);
    assert!(index.is_complete());
    document.append("\n# Second\n").unwrap();
    assert_eq!(
        index.get(&document, HeadingId(0)),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(
        index.advance(&document, &mut unlimited()),
        Err(LayoutError::StaleContinuation)
    );
}

#[derive(Debug)]
struct DropPayload {
    value: usize,
    drops: Rc<DropCell<usize>>,
}

impl Drop for DropPayload {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn checkpoint_payload_ownership_survives_until_drain_and_compaction() {
    let drops = Rc::new(DropCell::new(0));
    let mut index = CheckpointIndex::new();
    for value in 0..8 {
        index
            .insert(
                TileId(value),
                DropPayload {
                    value,
                    drops: drops.clone(),
                },
            )
            .unwrap();
    }
    let capacity = index.arena_stats().capacity_bytes;
    index.invalidate(0..8).unwrap();
    assert_eq!(index.len(), 0);
    assert_eq!(index.arena_stats().capacity_bytes, capacity);
    assert_eq!(drops.get(), 0);

    let mut out = Vec::new();
    let mut cursor = 0;
    assert_eq!(
        index.drain_before(TileId(8), &mut cursor, &mut unlimited(), &mut out),
        Completion::Covered
    );
    assert_eq!(out.len(), 8);
    assert_eq!(drops.get(), 0);
    assert_eq!(index.arena_stats().capacity_bytes, capacity);
    out.sort_unstable_by_key(|payload| payload.value);
    out.clear();
    assert_eq!(drops.get(), 8);

    let compaction_drops = Rc::new(DropCell::new(0));
    let mut compacted = CheckpointIndex::new();
    for value in 0..4 {
        compacted
            .insert(
                TileId(value),
                DropPayload {
                    value,
                    drops: compaction_drops.clone(),
                },
            )
            .unwrap();
    }
    compacted.clear();
    let before = compacted.arena_stats();
    assert!(before.capacity_bytes > 0);
    let mut task = compacted.into_compaction();
    assert!(task.stats().source.capacity_bytes > 0);
    let mut retired = Vec::new();
    assert_eq!(
        task.advance(&mut unlimited(), &mut retired),
        Completion::Covered
    );
    assert_eq!(retired.len(), 4);
    assert_eq!(compaction_drops.get(), 0);
    assert_eq!(task.stats().source.capacity_bytes, 0);
    assert!(task.stats().retired > 0);
    drop(retired);
    assert_eq!(compaction_drops.get(), 4);
    assert!(task.finish().is_ok());
}

#[test]
fn style_bit_union_loses_nesting_and_math_is_flagged() {
    let outer_strong = parse("**a ~~b~~**");
    let outer_strike = parse("~~a **b**~~");
    let strong_style = visible_run_style(&outer_strong, "b");
    let strike_style = visible_run_style(&outer_strike, "b");
    assert!(strong_style.contains(Style::STRONG));
    assert!(strong_style.contains(Style::STRIKE));
    assert_eq!(strong_style, strike_style);

    let math = parse("**$x$**");
    let math_style = visible_run_style(&math, "x");
    assert!(math_style.contains(Style::STRONG));
    assert!(math_style.contains(Style::MATH));
}

#[test]
fn end_tile_tracks_tile_boundaries_not_markdown_lines() {
    let source = format!("{}\n", "a".repeat(TILE_BYTES * 2 + 5));
    let document = parse(&source);
    assert!(document.tile_count() >= 3);
    let first = document.tile(TileId(0)).unwrap();
    let last = document.tile(TileId(document.tile_count() - 1)).unwrap();
    assert!(!document.text(first).unwrap().contains('\n'));
    assert!(document.text(last).unwrap().contains('\n'));
    assert!(first.end < source.len());

    let mut heights = HeightIndex::default();
    heights.rebuild(&document).unwrap();
    let mut pass = LayoutPass::new(&document, &heights, TileId(0), u64::MAX, false).unwrap();
    let mut sink = RecordingSink::default();
    assert_eq!(
        pass.advance(&document, &mut heights, &mut unlimited(), &mut sink)
            .unwrap(),
        Completion::EndOfDocument
    );
    assert_eq!(sink.visible_text(), source);
    assert_eq!(
        sink.begins,
        (0..document.tile_count()).map(TileId).collect::<Vec<_>>()
    );
    assert_eq!(
        sink.ends.iter().map(|end| end.tile).collect::<Vec<_>>(),
        sink.begins
    );
    assert_eq!(sink.ends[0].bytes, first.len());
    assert_eq!(sink.ends[0].newlines, 0);
    assert!(sink.ends.last().unwrap().newlines >= 1);
    assert!(sink.ends[0].bytes < source.len());
}

#[test]
fn checkpoint_keys_are_tile_ids() {
    // Type-level fact: CheckpointIndex::insert takes TileId, not BlockId or byte offsets.
    fn insert_with_tile_key<S>(index: &mut CheckpointIndex<S>, tile: TileId, value: S) {
        index.insert(tile, value).unwrap();
    }

    let mut index = CheckpointIndex::new();
    insert_with_tile_key(&mut index, TileId(9), "value");
    assert_eq!(index.get(TileId(9)), Some(&"value"));
    assert_eq!(index.previous(TileId(8)), None);
}

#[test]
fn display_cursor_stale_rules() {
    let mut document = Document::new();
    document.append("alpha\n").unwrap();
    let span = Span::new(0, document.source().len());
    let mut cursor = document.display_cursor(span).unwrap();
    document.append("").unwrap();
    assert!(!matches!(
        cursor.next(&document, &mut unlimited()),
        Err(LayoutError::StaleContinuation)
    ));
    document.append("beta\n").unwrap();
    assert_eq!(
        cursor.next(&document, &mut unlimited()),
        Err(LayoutError::StaleContinuation)
    );

    let mut finishing = Document::new();
    finishing.append("text\n").unwrap();
    let finish_span = Span::new(0, finishing.source().len());
    let mut finish_cursor = finishing.display_cursor(finish_span).unwrap();
    finishing.finish();
    assert_eq!(
        finish_cursor.next(&finishing, &mut unlimited()),
        Err(LayoutError::StaleContinuation)
    );

    let mut repeated = finishing.display_cursor(finish_span).unwrap();
    finishing.finish();
    assert!(!matches!(
        repeated.next(&finishing, &mut unlimited()),
        Err(LayoutError::StaleContinuation)
    ));
}

#[test]
fn drain_before_keeps_future_checkpoints() {
    let mut index = CheckpointIndex::new();
    for tile in [0, 2, 4, 6, 8] {
        index.insert(TileId(tile), tile).unwrap();
    }
    let mut cursor = 0;
    let mut out = Vec::new();
    assert_eq!(
        index.drain_before(TileId(4), &mut cursor, &mut unlimited(), &mut out),
        Completion::Covered
    );
    out.sort_unstable();
    assert_eq!(out, [0, 2]);
    assert_eq!(index.get(TileId(0)), None);
    assert_eq!(index.get(TileId(2)), None);
    assert_eq!(index.get(TileId(4)), Some(&4));
    assert_eq!(index.get(TileId(6)), Some(&6));
    assert_eq!(index.get(TileId(8)), Some(&8));
    assert_eq!(index.previous(TileId(7)), Some((TileId(6), &6)));
    assert_eq!(index.previous(TileId(9)), Some((TileId(8), &8)));
}

#[test]
fn checkpoint_replay_emits_target_once() {
    let document = parse(&"alpha\n".repeat(300));
    assert!(document.tile_count() > 4);
    let target = TileId(3);

    let mut direct_heights = HeightIndex::default();
    direct_heights.rebuild(&document).unwrap();
    let mut direct_pass =
        LayoutPass::new(&document, &direct_heights, TileId(0), u64::MAX, false).unwrap();
    let mut direct_sink = RecordingSink::default();
    assert_eq!(
        direct_pass
            .advance(
                &document,
                &mut direct_heights,
                &mut unlimited(),
                &mut direct_sink,
            )
            .unwrap(),
        Completion::EndOfDocument
    );

    let mut checkpoints = CheckpointIndex::new();
    checkpoints.insert(TileId(0), 0).unwrap();
    let (from, _) = checkpoints.previous(target).unwrap();
    assert!(from < target);

    let mut replay_heights = HeightIndex::default();
    replay_heights.rebuild(&document).unwrap();
    let mut replay_pass = LayoutPass::from_checkpoint(
        &document,
        &replay_heights,
        from,
        target,
        u64::MAX,
        usize::MAX,
    )
    .unwrap()
    .unwrap();
    let mut replay_sink = RecordingSink::default();
    assert_eq!(
        replay_pass
            .advance(
                &document,
                &mut replay_heights,
                &mut unlimited(),
                &mut replay_sink,
            )
            .unwrap(),
        Completion::EndOfDocument
    );

    let target_begins = replay_sink
        .begins
        .iter()
        .filter(|tile| **tile == target)
        .count();
    assert_eq!(target_begins, 1);
    assert_eq!(
        replay_sink.events_for(target),
        direct_sink.events_for(target)
    );
}

#[test]
fn link_destination_stays_raw_and_fragments_decode_only_percent() {
    let document = parse(concat!(
        "# a\n\n",
        "# a_b\n\n",
        "# a&b\n\n",
        "[entity](#&#x61;)\n",
        "[amp](#a\\&b)\n",
        "[underscore](#a\\_b)\n",
        "[percent](#%61)\n",
    ));
    assert_eq!(document.link_destination(LinkId(0)), Some("#&#x61;"));
    assert_eq!(document.link_destination(LinkId(1)), Some("#a\\&b"));
    assert_eq!(document.link_destination(LinkId(2)), Some("#a\\_b"));
    assert_eq!(document.link_destination(LinkId(3)), Some("#%61"));

    let index = build_anchors(&document);
    assert!(matches!(
        resolve(&index, &document, "#%61"),
        FragmentStep::Resolved(_)
    ));
    assert!(matches!(
        resolve(&index, &document, "#a_b"),
        FragmentStep::Resolved(_)
    ));
    assert_eq!(
        resolve(&index, &document, "#&#x61;"),
        FragmentStep::NotFound
    );
    assert_eq!(
        resolve(&index, &document, "#a%5C_b"),
        FragmentStep::NotFound
    );
    assert_eq!(
        resolve(&index, &document, "#a%5C%26b"),
        FragmentStep::NotFound
    );
    assert_eq!(
        resolve(&index, &document, "#a&amp;b"),
        FragmentStep::NotFound
    );
}
