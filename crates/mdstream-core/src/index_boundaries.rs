//! Sparse boundary oracles: no huge source/leaf allocations and no copy of the
//! production segment-tree algorithm. Run unchanged on 32- and 64-bit targets.

use std::{collections::BTreeMap, ops::Range};

use crate::{
    Anchor, Budget, CheckpointIndex, Completion, HeightIndex, LayoutError, LinkId, Span, Style,
    TileId, WorkLimit, annotations::Annotations, fold_ranges::FoldRanges,
};

fn limits() -> Vec<usize> {
    let mut limits = vec![i32::MAX as usize, isize::MAX as usize];
    limits.dedup();
    limits
}

fn edges(end: usize) -> Vec<usize> {
    let mut points = vec![0, 1, end - 2, end - 1, end];
    for bit in 1..usize::BITS - 1 {
        let power = 1_usize << bit;
        if power < end {
            points.extend([power - 1, power, power + 1]);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

// A short, linear list of constant-height intervals, with wider coordinates
// and sums than either target's usize. Updates split intervals by intersection.
struct HeightModel(Vec<(Range<u128>, u64)>);

impl HeightModel {
    fn new(len: usize, height: u64) -> Self {
        Self(vec![(0..len as u128, height)])
    }

    fn change(&mut self, range: Range<usize>, f: impl Fn(u64) -> u64) {
        let range = range.start as u128..range.end as u128;
        self.0 = self
            .0
            .iter()
            .flat_map(|(old, height)| {
                let start = range.start.max(old.start);
                let end = range.end.min(old.end);
                if start >= end {
                    return vec![(old.clone(), *height)];
                }
                [
                    (old.start..start, *height),
                    (start..end, f(*height)),
                    (end..old.end, *height),
                ]
                .into_iter()
                .filter(|(r, _)| !r.is_empty())
                .collect()
            })
            .collect();
    }

    fn prefix(&self, end: u128) -> u128 {
        self.0
            .iter()
            .map(|(r, h)| r.end.min(end).saturating_sub(r.start) * u128::from(*h))
            .sum()
    }

    fn locate(&self, row: u128) -> Option<Anchor> {
        let mut top = 0;
        for (r, height) in &self.0 {
            let rows = (r.end - r.start) * u128::from(*height);
            if row < top + rows {
                let local = row - top;
                return Some(Anchor {
                    tile: TileId(usize::try_from(r.start + local / u128::from(*height)).unwrap()),
                    offset: u64::try_from(local % u128::from(*height)).unwrap(),
                });
            }
            top += rows;
        }
        None
    }

    fn check(&self, index: &HeightIndex) {
        let total = self.prefix(index.len() as u128);
        assert_eq!(index.total_rows(), total);
        let mut rows = vec![0, total / 2, total.saturating_sub(1), total, u128::MAX];
        for (r, height) in &self.0 {
            for at in [r.start, r.end - 1] {
                let tile = usize::try_from(at).unwrap();
                let top = self.prefix(at);
                assert_eq!(index.height(TileId(tile)), Some(*height));
                assert_eq!(index.prefix_rows(tile), Some(top));
                rows.extend([top.saturating_sub(1), top, top + u128::from(*height)]);
            }
        }
        for row in rows {
            let expected = self.locate(row);
            assert_eq!(index.locate(row), expected, "row {row}");
            if let Some(anchor) = expected {
                assert_eq!(index.row_of(anchor), Some(row));
            }
        }
    }
}

#[test]
fn index_boundaries_invalid_height_ids_are_rejected_before_arithmetic() {
    let mut index = HeightIndex::default();
    index.grow_to(3).unwrap();
    for invalid in [3, isize::MAX as usize, usize::MAX] {
        assert!(index.ticket(TileId(invalid)).is_none());
        assert_eq!(index.height(TileId(invalid)), None);
        assert_eq!(index.base_height(TileId(invalid)), None);
        assert_eq!(index.height_state(TileId(invalid)), None);
        assert_eq!(
            index.row_of(Anchor {
                tile: TileId(invalid),
                offset: u64::MAX
            }),
            None
        );
    }
    let before = (
        index.len(),
        index.total_rows(),
        index.node_count(),
        index.mutation_visits(),
    );
    for invalid in [isize::MAX as usize + 1, usize::MAX] {
        assert_eq!(index.grow_to(invalid), Err(LayoutError::InvalidRange));
        assert_eq!(index.invalidate(0..invalid), Err(LayoutError::InvalidRange));
        assert_eq!(index.expire(0..invalid), Err(LayoutError::InvalidRange));
        assert_eq!(index.suppress(0..invalid), Err(LayoutError::InvalidRange));
    }
    assert_eq!(
        (
            index.len(),
            index.total_rows(),
            index.node_count(),
            index.mutation_visits()
        ),
        before
    );
}

#[test]
fn index_boundaries_sparse_growth_crosses_powers_without_narrowing_rows() {
    for end in limits() {
        let mut index = HeightIndex::new(u64::MAX).unwrap();
        for len in edges(end) {
            index.grow_to(len).unwrap();
            assert_eq!(index.total_rows(), len as u128 * u128::from(u64::MAX));
            assert_eq!(index.prefix_rows(len), Some(index.total_rows()));
            if len > 0 {
                let anchor = Anchor {
                    tile: TileId(len - 1),
                    offset: u64::MAX - 1,
                };
                assert_eq!(index.locate(index.total_rows() - 1), Some(anchor));
                assert_eq!(index.row_of(anchor), Some(index.total_rows() - 1));
            }
            assert!(index.node_count() <= 2 * usize::BITS as usize + 1);
        }
        assert_eq!(index.grow_to(end - 1), Err(LayoutError::InvalidRange));
        assert_eq!(index.locate(index.total_rows()), None);
    }
}

#[test]
fn index_boundaries_height_mutations_match_wide_interval_model() {
    for end in limits() {
        let mut index = HeightIndex::new(u64::MAX).unwrap();
        index.grow_to(end).unwrap();
        let mut model = HeightModel::new(end, u64::MAX);
        let points = edges(end);
        let mut estimate = u64::MAX;
        for step in 0..160 {
            let a = points[(step * 17 + 3) % points.len()];
            let b = points[(step * 31 + 7) % points.len()];
            let range = a.min(b)..a.max(b);
            let visits = index.mutation_visits();
            let nodes = index.node_count();
            match step % 5 {
                0 => {
                    index.suppress(range.clone()).unwrap();
                    model.change(range, |_| 0);
                }
                1 => {
                    index.expire(range.clone()).unwrap();
                    model.change(range, |h| if h == 0 { estimate } else { h });
                }
                2 => {
                    index.invalidate(range.clone()).unwrap();
                    model.change(range, |_| estimate);
                }
                3 => {
                    let at = a.min(end - 1);
                    let height = [0, 1, u32::MAX as u64 + 1, u64::MAX][step % 4];
                    index
                        .measure(index.ticket(TileId(at)).unwrap(), height)
                        .unwrap();
                    model.change(at..at + 1, |_| height);
                }
                _ => {
                    estimate = if step % 2 == 0 { 3 } else { u64::MAX };
                    index.reset(estimate).unwrap();
                    model.change(0..end, |_| estimate);
                }
            }
            assert!(index.mutation_visits() - visits <= 16 * u64::from(usize::BITS));
            // Only the two boundary paths may split, at most two children per level.
            assert!(index.node_count() - nodes <= 4 * usize::BITS as usize);
            model.check(&index);
        }
    }
}

#[test]
fn index_boundaries_nested_masks_match_interval_union_and_keep_base_heights() {
    for end in limits() {
        let mut index = HeightIndex::new(u64::MAX).unwrap();
        index.grow_to(end).unwrap();
        let ranges = [1..end - 1, end / 2..end, end - 3..end - 1];
        let mut active = [false; 3];
        for (i, on) in [
            (0, true),
            (1, true),
            (2, true),
            (0, false),
            (1, false),
            (2, false),
        ] {
            let visits = index.mutation_visits();
            index.cover(ranges[i].clone(), on);
            active[i] = on;
            assert!(index.mutation_visits() - visits <= 16 * u64::from(usize::BITS));
            let mut model = HeightModel::new(end, u64::MAX);
            for (range, active) in ranges.iter().zip(active) {
                if active {
                    model.change(range.clone(), |_| 0);
                }
            }
            model.check(&index);
            assert_eq!(index.base_height(TileId(end - 2)), Some(u64::MAX));
            // Reset must preserve visibility and use only one tree operation.
            let visits = index.mutation_visits();
            index.reset(u64::MAX).unwrap();
            assert_eq!(index.mutation_visits() - visits, 1);
            model.check(&index);
        }
        assert!(index.node_count() < 16 * usize::BITS as usize);
    }
}

#[test]
fn index_boundaries_budgeted_viewport_resumes_at_last_sparse_tiles() {
    for end in limits() {
        let mut index = HeightIndex::new(u64::MAX).unwrap();
        index.grow_to(end).unwrap();
        index.suppress(0..end - 3).unwrap();
        let expected = index.viewport(0..u128::MAX, u64::MAX, 4);
        assert_eq!(
            expected.iter().map(|v| v.tile.0).collect::<Vec<_>>(),
            [end - 3, end - 2, end - 1]
        );
        let limit = WorkLimit {
            queries: 1,
            tiles: 1,
            ..Default::default()
        };
        let mut budget = Budget::new(limit);
        let mut out = Vec::new();
        let mut page = index.viewport_budgeted(0..u128::MAX, u64::MAX, &mut budget, &mut out);
        let mut actual = out.clone();
        assert_eq!(budget.used(), limit);
        for _ in 0..3 {
            let Some(cursor) = page.next.take() else {
                break;
            };
            let mut budget = Budget::new(limit);
            page = index
                .resume_viewport(cursor, &mut budget, &mut out)
                .unwrap();
            actual.extend_from_slice(&out);
            assert_eq!(budget.used(), limit);
        }
        assert!(page.next.is_none());
        assert_eq!(page.completion, Completion::EndOfDocument);
        assert_eq!(actual, expected);
        let mut zero = Budget::new(WorkLimit::default());
        let page = index.viewport_budgeted(u128::MAX - 1..u128::MAX, u64::MAX, &mut zero, &mut out);
        assert_eq!(page.completion, Completion::BudgetExhausted);
        assert!(out.is_empty());
    }
}

#[test]
fn index_boundaries_checkpoints_match_sparse_ordered_map_and_reject_extremes() {
    for end in limits() {
        let points = edges(end);
        let mut index = CheckpointIndex::new();
        let mut model = BTreeMap::new();
        for (serial, &at) in points.iter().filter(|&&at| at < end).enumerate() {
            let visits = index.mutation_visits();
            index.insert(TileId(at), serial).unwrap();
            model.insert(at, serial);
            assert!(index.mutation_visits() - visits <= u64::from(usize::BITS));
        }
        let token = index.token(TileId(end - 1)).unwrap();
        let nodes = index.node_count();
        for invalid in [isize::MAX as usize, usize::MAX] {
            assert_eq!(
                index.insert(TileId(invalid), 999),
                Err(LayoutError::InvalidRange)
            );
            assert!(index.token(TileId(invalid)).is_none());
        }
        assert_eq!(index.node_count(), nodes);
        assert!(index.validates(token));
        for step in 0..80 {
            let at = points[(step * 17) % points.len()].min(end - 1);
            if step % 3 == 0 {
                index.insert(TileId(at), step).unwrap();
                model.insert(at, step);
            } else {
                let bound = points[(step * 29 + 1) % points.len()];
                let range = at.min(bound)..at.max(bound);
                let visits = index.mutation_visits();
                index.invalidate(range.clone()).unwrap();
                model.retain(|&key, _| !range.contains(&key));
                assert!(index.mutation_visits() - visits <= 8 * u64::from(usize::BITS));
            }
            assert_eq!(index.len(), model.len());
            for query in points.iter().copied().chain([usize::MAX]) {
                let expected = model
                    .range(..=query)
                    .next_back()
                    .map(|(&k, v)| (TileId(k), v));
                assert_eq!(index.previous(TileId(query)), expected);
                assert_eq!(index.get(TileId(query)), model.get(&query));
                for distance in [0, 1, end, usize::MAX] {
                    assert_eq!(
                        index.previous_within(TileId(query), distance),
                        expected.filter(|(k, _)| query as u128 - k.0 as u128 <= distance as u128)
                    );
                }
            }
        }
        index.invalidate(0..usize::MAX).unwrap();
        assert!(index.is_empty());
        assert!(!index.validates(token));
        index.insert(TileId(end - 1), 42).unwrap();
        assert_eq!(
            index.previous(TileId(usize::MAX)),
            Some((TileId(end - 1), &42))
        );
        assert!(!index.validates(token));
        assert_eq!(index.len(), 1);
    }
}

#[test]
fn index_boundaries_annotation_overlays_match_sparse_write_log() {
    for end in limits() {
        let points = edges(end);
        let mut tree = Annotations::default();
        let mut writes = Vec::new();
        for serial in 0..48 {
            let a = points[(serial * 17 + 3) % points.len()];
            let b = if serial == 0 {
                end
            } else {
                points[(serial * 31 + 1) % points.len()]
            };
            let span = Span::new(a.min(b), a.max(b));
            let style = [Style::HIDDEN, Style::STRONG, Style::EMPHASIS][serial % 3];
            let link = (serial % 4 == 0).then_some(LinkId(serial));
            let revision = serial as u64 / 3 + 1;
            let visits = tree.visits;
            tree.apply(span, style, link, revision);
            assert!(tree.visits - visits <= 4 * u64::from(usize::BITS));
            if !span.is_empty() {
                writes.push((span, style, link, revision));
            }
            // An independent event partition allows exhaustive comparison of
            // every constant region, without allocating an array of end bytes.
            let mut boundaries = vec![0, end];
            for (s, _, _, _) in &writes {
                boundaries.extend([s.start, s.end]);
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut segments = tree.segments(Span::new(0, end));
            let mut cursor = 0;
            for (span, mark) in segments.by_ref() {
                assert_eq!(span.start, cursor);
                assert!(span.start < span.end);
                for at in boundaries
                    .iter()
                    .copied()
                    .filter(|&p| span.start <= p && p < span.end)
                    .chain([span.start, span.end - 1])
                {
                    let (mut s, mut l, mut r) = (Style::default(), None, 0);
                    for (range, style, link, revision) in &writes {
                        if range.range().contains(&at) {
                            s = s.union(*style);
                            l = link.or(l);
                            r = r.max(*revision);
                        }
                    }
                    assert_eq!((mark.style, mark.link, mark.revision), (s, l, r));
                    let point = tree.at(at);
                    assert_eq!((point.style, point.link, point.revision), (s, l, r));
                }
                cursor = span.end;
            }
            assert_eq!(cursor, end);
            assert!(segments.visits <= 8 * writes.len() as u64 * u64::from(usize::BITS) + 1);
            for pair in boundaries.windows(2) {
                let range = Span::new(pair[0], pair[1]);
                let newest = writes
                    .iter()
                    .filter(|(s, _, _, _)| s.start < range.end && range.start < s.end)
                    .map(|(_, _, _, r)| *r)
                    .max()
                    .unwrap_or(0);
                assert_eq!(tree.newest(range), newest);
                let visible = boundaries
                    .iter()
                    .copied()
                    .filter(|&p| p >= range.start && p < end)
                    .find(|&p| {
                        !writes.iter().any(|(s, style, _, _)| {
                            s.range().contains(&p) && style.contains(Style::HIDDEN)
                        })
                    });
                assert_eq!(tree.next_visible(range.start, end), visible);
            }
        }
        assert!(tree.node_count() < 8 * points.len() * usize::BITS as usize);
    }
}

#[test]
fn index_boundaries_fold_endpoints_and_open_sentinel_match_ordered_map() {
    for end in limits() {
        let points = edges(end);
        let mut tree = FoldRanges::default();
        let mut model = BTreeMap::new();
        for step in 0..128 {
            let id = points[(step * 17) % points.len()].min(end - 1);
            let stop = [0, end / 2, end, usize::MAX][step % 4];
            let visits = tree.visits();
            tree.set(id, stop);
            model.insert(id, stop);
            assert!(tree.visits() - visits <= u64::from(usize::BITS));
            for &before in &points {
                for offset in [
                    0,
                    end / 2 - 1,
                    end / 2,
                    end - 1,
                    end,
                    usize::MAX - 1,
                    usize::MAX,
                ] {
                    let visits = tree.visits();
                    let expected = model
                        .range(..before)
                        .find(|(_, stop)| **stop > offset)
                        .map(|(&id, _)| id);
                    assert_eq!(tree.covering(before, offset), expected);
                    assert!(tree.visits() - visits <= 4 * u64::from(usize::BITS));
                }
            }
        }
        assert!(tree.node_count() < 4 * points.len() * usize::BITS as usize);
    }
}

#[test]
fn index_boundaries_budget_charges_use_wide_oracle_and_fail_atomically() {
    fn cost(values: [usize; 6]) -> WorkLimit {
        WorkLimit {
            queries: values[0],
            tiles: values[1],
            bytes: values[2],
            objects: values[3],
            recovery: values[4],
            dependencies: values[5],
        }
    }
    for maximum in [u32::MAX as usize, usize::MAX] {
        let limits = [maximum, maximum - 1, maximum, maximum - 2, 7, 0];
        let mut budget = Budget::new(cost(limits));
        let mut used = [0_u128; 6];
        for values in [
            [maximum - 1, 0, maximum - 2, 0, 0, 0],
            [2, 0, 0, 0, 0, 0],
            [1, maximum - 1, 2, maximum - 2, 7, 1],
            [1, maximum - 1, 2, maximum - 2, 7, 0],
            [maximum; 6],
            [0; 6],
        ] {
            let expected = (0..6).all(|i| used[i] + values[i] as u128 <= limits[i] as u128);
            assert_eq!(budget.charge(cost(values)), expected);
            if expected {
                for i in 0..6 {
                    used[i] += values[i] as u128;
                }
            }
            assert_eq!(
                budget.used(),
                cost(used.map(|n| usize::try_from(n).unwrap()))
            );
            assert_eq!(
                budget.remaining(),
                cost(std::array::from_fn(|i| usize::try_from(
                    limits[i] as u128 - used[i]
                )
                .unwrap()))
            );
        }
    }
}

#[test]
fn index_boundaries_last_checkpoint_measurement_and_budgeted_drain() {
    for end in limits() {
        let mut heights = HeightIndex::default();
        heights.grow_to(end).unwrap();
        let mut checkpoints = CheckpointIndex::new();
        checkpoints
            .insert_for_layout(&mut heights, TileId(end - 2), 7)
            .unwrap();
        let ticket = heights
            .ticket_from(TileId(end - 1), &checkpoints, TileId(end - 2))
            .unwrap();
        heights.expire(0..end - 2).unwrap();
        heights
            .measure_from(ticket, u64::MAX, &checkpoints)
            .unwrap();
        assert_eq!(
            heights.total_rows(),
            (end - 1) as u128 + u128::from(u64::MAX)
        );
        let stale = heights
            .ticket_from(TileId(end - 1), &checkpoints, TileId(end - 2))
            .unwrap();
        assert_eq!(
            checkpoints
                .insert_for_layout(&mut heights, TileId(end - 2), 9)
                .unwrap(),
            Some(7)
        );
        assert_eq!(
            heights.measure_from(stale, 1, &checkpoints),
            Err(LayoutError::StaleMeasurement)
        );
        // Invalid input must not expire a valid token or allocate a path.
        let token = checkpoints.token(TileId(end - 2)).unwrap();
        assert_eq!(
            checkpoints.insert_for_layout(&mut heights, TileId(usize::MAX), 10),
            Err(LayoutError::InvalidRange)
        );
        assert!(checkpoints.validates(token));
        let (mut cursor, mut out) = (0, Vec::new());
        for _ in 0..=checkpoints.node_count() {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            });
            if checkpoints.drain_before(TileId(usize::MAX), &mut cursor, &mut budget, &mut out)
                == Completion::Covered
            {
                break;
            }
        }
        assert_eq!(cursor, checkpoints.node_count());
        assert_eq!(out, [9]);
        assert!(checkpoints.is_empty());
        assert!(!checkpoints.validates(token));
    }
}

#[test]
fn index_boundaries_extreme_source_windows_and_object_ids_do_not_wrap() {
    let mut doc = crate::Document::new();
    doc.append("# 中🙂\n\n> - [x] [a](/u) $x$ [^n]\n\na|b\n-|-\nc|d\n\n[^n]: note\n")
        .unwrap();
    doc.finish();
    assert!(!doc.tables().is_empty());
    assert!(!doc.maths().is_empty());
    assert!(!doc.links().is_empty());
    for at in [isize::MAX as usize, usize::MAX] {
        assert!(doc.tile(TileId(at)).is_none());
        assert!(doc.block(crate::BlockId(at)).is_none());
        assert!(doc.container(crate::ContainerId(at)).is_none());
        assert!(doc.headings().get(crate::HeadingId(at)).is_none());
        assert!(doc.table(crate::TableId(at)).is_none());
        assert!(doc.math(crate::MathId(at)).is_none());
        assert!(doc.footnote(crate::FootnoteId(at)).is_none());
        assert!(doc.link_info(LinkId(at)).is_none());
        assert!(doc.link_destination(LinkId(at)).is_none());
        assert_eq!(doc.tile_at(at), None);
        assert_eq!(doc.block_at(at), None);
        assert_eq!(doc.table_at(at), None);
        assert_eq!(doc.next_visible(at), None);
        assert_eq!(doc.next_drawable(at), None);
        assert_eq!(doc.tables()[0].row_at(at), None);
        for span in [Span::new(0, at), Span::new(at, at), Span::new(at, 0)] {
            assert!(doc.text(span).is_none());
            assert!(doc.runs_in(span).is_none());
            assert!(doc.display_runs_in(span).is_none());
            assert!(doc.display_cursor(span).is_none());
            assert!(doc.maths_in(span).is_none());
        }
        // Window lookup deliberately clips rather than validating spans.
        assert_eq!(doc.tiles_overlapping(Span::new(0, at)), 0..doc.tile_count());
        assert_eq!(
            doc.tiles_overlapping(Span::new(at, at)),
            doc.tile_count()..doc.tile_count()
        );
    }
    for at in edges(doc.source().len()) {
        let expected =
            (0..doc.tile_count()).find(|&i| doc.tile(TileId(i)).unwrap().range().contains(&at));
        assert_eq!(doc.tile_at(at), expected.map(TileId));
    }
}
