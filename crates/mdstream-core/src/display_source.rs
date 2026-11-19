//! Input-owned raw source facts. Semantic overlays remain in DisplayShapeSeed.
//! Queries share this owner and compose the existing Unicode transducers; they
//! never classify or replay an unrelated source prefix.
use crate::display_source_html::HtmlShape;
use crate::display_source_unicode::{GcbSummary, Properties, WidthSummary};
use crate::{Document, Span};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

const RUNS_PER_LEAF: usize = 256;
#[derive(Clone, Copy, Debug)]
pub struct DisplaySourceAtom {
    pub source: Span,
    pub scalar: char,
    pub raw: Properties,
    pub code: Properties,
    pub crlf: bool,
}
impl DisplaySourceAtom {
    fn displayed(self, code: bool) -> Option<(char, Properties)> {
        if self.scalar == '\t' || code && matches!(self.scalar, '\r' | '\n') {
            Some((
                ' ',
                if self.scalar == '\t' {
                    Properties::of(' ')
                } else {
                    self.code
                },
            ))
        } else if self.scalar.is_control() && !matches!(self.scalar, '\r' | '\n') {
            None
        } else {
            Some((self.scalar, self.raw))
        }
    }
    fn envelope(self, code: bool) -> Span {
        if code && self.crlf {
            Span::new(self.source.start - 1, self.source.end)
        } else {
            self.source
        }
    }
}
#[derive(Clone, Debug)]
pub struct DisplaySourceSummary {
    pub gcb: GcbSummary,
    pub width: WidthSummary,
    pub bytes: usize,
    pub scalars: usize,
    pub ascii: bool,
    pub first: Option<Span>,
    pub last: Option<Span>,
}
impl Default for DisplaySourceSummary {
    fn default() -> Self {
        Self {
            gcb: Default::default(),
            width: Default::default(),
            bytes: 0,
            scalars: 0,
            ascii: true,
            first: None,
            last: None,
        }
    }
}
impl DisplaySourceSummary {
    fn join(a: &Self, b: &Self, w: &mut DisplaySourceWork) -> Self {
        w.summary_joins += 1;
        w.gcb_state_transitions += 24;
        w.width_state_transitions += 132;
        Self {
            gcb: GcbSummary::join(&a.gcb, &b.gcb),
            width: WidthSummary::join(&a.width, &b.width),
            bytes: a.bytes + b.bytes,
            scalars: a.scalars + b.scalars,
            ascii: a.ascii && b.ascii,
            first: a.first.or(b.first),
            last: b.last.or(a.last),
        }
    }
    fn repeat(
        atom: DisplaySourceAtom,
        count: usize,
        code: bool,
        w: &mut DisplaySourceWork,
    ) -> Self {
        let Some((ch, properties)) = atom.displayed(code) else {
            return Self::default();
        };
        let mut power = Self {
            bytes: ch.len_utf8(),
            scalars: 1,
            ascii: ch.is_ascii(),
            first: Some(atom.envelope(code)),
            last: Some(atom.envelope(code)),
            ..Self::default()
        };
        power.gcb.push(properties);
        power.width.push(properties);
        w.gcb_state_transitions += 24;
        w.width_state_transitions += 132;
        let mut answer = Self::default();
        let mut count = count;
        while count != 0 {
            if count & 1 != 0 {
                answer = Self::join(&answer, &power, w);
            }
            count >>= 1;
            if count != 0 {
                power = Self::join(&power, &power, w);
            }
        }
        answer
    }
}
#[derive(Clone, Default)]
struct Pair {
    raw: DisplaySourceSummary,
    code: DisplaySourceSummary,
}
impl Pair {
    fn join(a: &Self, b: &Self, w: &mut DisplaySourceWork) -> Self {
        Self {
            raw: DisplaySourceSummary::join(&a.raw, &b.raw, w),
            code: DisplaySourceSummary::join(&a.code, &b.code, w),
        }
    }
}
#[derive(Clone, Copy)]
struct Run {
    atom: DisplaySourceAtom,
    start: usize,
    len: usize,
}
impl Run {
    fn at(self, at: usize) -> DisplaySourceAtom {
        let mut a = self.atom;
        let start = a.source.start + (at - self.start) * a.scalar.len_utf8();
        a.source = Span::new(start, start + a.scalar.len_utf8());
        a.crlf &= at == self.start;
        a
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplaySourceWork {
    pub decoded_bytes: usize,
    pub decoded_scalars: usize,
    pub classified_scalars: usize,
    pub input_nodes: usize,
    pub input_runs: usize,
    pub input_scratch_peak_bytes: usize,
    pub query_nodes: usize,
    pub query_runs: usize,
    pub summary_joins: usize,
    pub gcb_state_transitions: usize,
    pub width_state_transitions: usize,
    pub html_state_steps: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplaySourceStats {
    pub runs: usize,
    pub scalars: usize,
    pub capacity_bytes: usize,
    pub work: DisplaySourceWork,
}
struct SourceIndex {
    runs: Vec<Run>,
    scalars: usize,
    bytes: usize,
    nodes: Vec<Pair>,
    leaves: usize,
    html_scan: HtmlShape,
    html: BTreeMap<usize, Span>,
    work: DisplaySourceWork,
}
impl Default for SourceIndex {
    fn default() -> Self {
        Self {
            runs: Vec::new(),
            scalars: 0,
            bytes: 0,
            nodes: vec![Pair::default(); 2],
            leaves: 1,
            html_scan: Default::default(),
            html: Default::default(),
            work: Default::default(),
        }
    }
}
/// One shared current input index. A consumer must validate its document and
/// revision before reading it, exactly as for native semantic cursors.
#[derive(Clone, Default)]
pub struct DisplaySource(Arc<RwLock<SourceIndex>>);
impl Document {
    pub fn display_source(&self) -> DisplaySource {
        self.source_scalars.clone()
    }
    pub fn display_source_stats(&self) -> DisplaySourceStats {
        self.source_scalars.stats()
    }
}
impl DisplaySource {
    pub(crate) fn append(&mut self, text: &str, start: usize) {
        let mut s = self.0.write().unwrap();
        let closed = s.runs.len().saturating_sub(1) / RUNS_PER_LEAF;
        let mut text_runs: Vec<(usize, char, usize)> = Vec::new();
        for (at, ch) in text.char_indices() {
            if let Some(last) = text_runs.last_mut().filter(|last| last.1 == ch) {
                last.2 += 1;
            } else {
                text_runs.push((start + at, ch, 1));
            }
        }
        s.work.input_scratch_peak_bytes = s
            .work
            .input_scratch_peak_bytes
            .max(text_runs.capacity() * std::mem::size_of::<(usize, char, usize)>());
        s.work.decoded_bytes += text.len();
        for (at, ch, count) in text_runs {
            s.work.decoded_scalars += count;
            if s.runs.last().is_some_and(|r| r.atom.scalar == ch) {
                let r = s.runs.last_mut().unwrap();
                r.len += count;
            } else {
                let raw = Properties::of(ch);
                let code = if matches!(ch, '\r' | '\n') {
                    Properties::of(' ')
                } else {
                    raw
                };
                let crlf = ch == '\n' && s.runs.last().is_some_and(|r| r.atom.scalar == '\r');
                let scalar_start = s.scalars;
                s.runs.push(Run {
                    atom: DisplaySourceAtom {
                        source: Span::new(at, at + ch.len_utf8()),
                        scalar: ch,
                        raw,
                        code,
                        crlf,
                    },
                    start: scalar_start,
                    len: count,
                });
                s.work.classified_scalars += 1;
            }
            s.scalars += count;
            let mut steps = 0;
            let spans = s.html_scan.push_repeated(at, ch, count, &mut steps);
            s.work.html_state_steps += steps;
            for span in spans {
                s.html.insert(span.start, span);
            }
        }
        s.bytes = start + text.len();
        if let Some(span) = s.html_scan.open(s.bytes) {
            s.html.insert(span.start, span);
        }
        // The last run may still grow. Cache only complete, immutable pages;
        // queries compose at most one bounded open page from stored properties.
        let used = s.runs.len().saturating_sub(1) / RUNS_PER_LEAF;
        if used > s.leaves {
            s.grow(used.next_power_of_two());
        }
        for leaf in closed..used {
            s.finalize_leaf(leaf);
        }
    }
    pub fn len(&self) -> usize {
        self.0.read().unwrap().scalars
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn bytes(&self) -> usize {
        self.0.read().unwrap().bytes
    }
    pub fn atom(&self, at: usize) -> DisplaySourceAtom {
        let s = self.0.read().unwrap();
        s.runs[s.runs.partition_point(|r| r.start + r.len <= at)].at(at)
    }
    /// Homogeneous input run containing a source byte, O(log R), metadata only.
    /// Semantic consumers must still apply their existing overlays/atom rules.
    pub fn run_at_source(&self, byte: usize) -> Option<(char, Span)> {
        let s = self.0.read().unwrap();
        let r =
            *s.runs.get(s.runs.partition_point(|r| {
                r.atom.source.start + r.len * r.atom.scalar.len_utf8() <= byte
            }))?;
        Some((
            r.atom.scalar,
            Span::new(
                r.atom.source.start,
                r.atom.source.start + r.len * r.atom.scalar.len_utf8(),
            ),
        ))
    }
    pub fn position(&self, at: usize) -> usize {
        let s = self.0.read().unwrap();
        if at == s.scalars {
            s.bytes
        } else {
            s.runs[s.runs.partition_point(|r| r.start + r.len <= at)]
                .at(at)
                .source
                .start
        }
    }
    pub fn lower_bound(&self, byte: usize) -> usize {
        let s = self.0.read().unwrap();
        if byte >= s.bytes {
            return s.scalars + usize::from(byte > s.bytes);
        }
        let r = s.runs[s
            .runs
            .partition_point(|r| r.atom.source.start + r.len * r.atom.scalar.len_utf8() <= byte)];
        r.start + (byte - r.atom.source.start).div_ceil(r.atom.scalar.len_utf8())
    }
    pub fn summary(
        &self,
        lo: usize,
        hi: usize,
        code: bool,
        w: &mut DisplaySourceWork,
    ) -> DisplaySourceSummary {
        let s = self.0.read().unwrap();
        if lo == hi {
            return DisplaySourceSummary::default();
        }
        let first = s.runs.partition_point(|r| r.start + r.len <= lo);
        let end = s.runs.partition_point(|r| r.start < hi);
        let a = (first + usize::from(lo > s.runs[first].start)).div_ceil(RUNS_PER_LEAF);
        let last = s.runs[end - 1];
        let b = ((end - usize::from(hi < last.start + last.len)) / RUNS_PER_LEAF)
            .min(s.runs.len().saturating_sub(1) / RUNS_PER_LEAF);
        if a > b {
            return s.summarize_runs(first, end, lo, hi, code, w);
        }
        let mut answer = s.summarize_runs(first, a * RUNS_PER_LEAF, lo, hi, code, w);
        let middle = s.range(1, 0..s.leaves, a..b, code, w);
        answer = DisplaySourceSummary::join(&answer, &middle, w);
        let tail = s.summarize_runs(b * RUNS_PER_LEAF, end, lo, hi, code, w);
        DisplaySourceSummary::join(&answer, &tail, w)
    }
    pub fn html_state(&self) -> HtmlShape {
        self.0.read().unwrap().html_scan.clone()
    }
    /// First input-classified HTML span intersecting a source range, including
    /// an opener before the range. Two ordered metadata seeks, no source read.
    pub fn first_html_in(&self, source: Span) -> Option<Span> {
        if source.start >= source.end {
            return None;
        }
        let s = self.0.read().unwrap();
        s.html
            .range(..=source.start)
            .next_back()
            .map(|(_, &span)| span)
            .filter(|span| span.end > source.start)
            .or_else(|| {
                s.html
                    .range(source.start..source.end)
                    .next()
                    .map(|(_, &span)| span)
            })
    }
    pub fn next_html(&self, after: Option<usize>) -> Option<Span> {
        use std::ops::Bound::{Excluded, Unbounded};
        let s = self.0.read().unwrap();
        s.html
            .range((after.map_or(Unbounded, Excluded), Unbounded))
            .next()
            .map(|(_, &span)| span)
    }
    pub fn stats(&self) -> DisplaySourceStats {
        let s = self.0.read().unwrap();
        DisplaySourceStats {
            runs: s.runs.len(),
            scalars: s.scalars,
            capacity_bytes: std::mem::size_of::<SourceIndex>()
                + s.runs.capacity() * std::mem::size_of::<Run>()
                + s.nodes.capacity() * std::mem::size_of::<Pair>()
                + s.html.len() * (std::mem::size_of::<Span>() + std::mem::size_of::<usize>()),
            work: s.work,
        }
    }
}
impl SourceIndex {
    fn summarize_runs(
        &self,
        begin: usize,
        end: usize,
        lo: usize,
        hi: usize,
        code: bool,
        w: &mut DisplaySourceWork,
    ) -> DisplaySourceSummary {
        let mut answer = DisplaySourceSummary::default();
        let mut at = begin;
        while at < end {
            let r = self.runs[at];
            let start = lo.max(r.start);
            let finish = hi.min(r.start + r.len);
            let atom = r.at(start);
            let Some((ch, properties)) = atom.displayed(code) else {
                at += 1;
                w.query_runs += 1;
                continue;
            };
            let mut count = finish - start;
            let mut bytes = count * ch.len_utf8();
            let mut last = r.at(finish - 1).envelope(code);
            let mut ascii = ch.is_ascii();
            at += 1;
            w.query_runs += 1;
            // Adjacent source characters with the same machine transition can
            // share its power, including varied ASCII and non-ASCII text.
            while at < end {
                let next = self.runs[at];
                let a = next.at(lo.max(next.start));
                let Some((c, p)) = a.displayed(code) else {
                    break;
                };
                if p != properties {
                    break;
                }
                let b = hi.min(next.start + next.len);
                let n = b - lo.max(next.start);
                count += n;
                bytes += n * c.len_utf8();
                ascii &= c.is_ascii();
                last = next.at(b - 1).envelope(code);
                at += 1;
                w.query_runs += 1;
            }
            let mut group = DisplaySourceSummary::repeat(atom, count, code, w);
            group.bytes = bytes;
            group.ascii = ascii;
            group.first = Some(atom.envelope(code));
            group.last = Some(last);
            answer = DisplaySourceSummary::join(&answer, &group, w);
        }
        answer
    }
    fn record_input_work(&mut self, work: DisplaySourceWork) {
        self.work.input_nodes += work.input_nodes;
        self.work.input_runs += work.query_runs;
        self.work.summary_joins += work.summary_joins;
        self.work.gcb_state_transitions += work.gcb_state_transitions;
        self.work.width_state_transitions += work.width_state_transitions;
    }
    fn grow(&mut self, leaves: usize) {
        let mut nodes = vec![Pair::default(); leaves * 2];
        for leaf in 0..self.leaves {
            nodes[leaves + leaf] = std::mem::take(&mut self.nodes[self.leaves + leaf]);
        }
        self.nodes = nodes;
        self.leaves = leaves;
        let mut work = DisplaySourceWork::default();
        // Finalized leaves move intact. Rebuild only the hierarchy, never
        // classify or replay the source operands behind those summaries.
        for at in (1..leaves).rev() {
            self.nodes[at] = Pair::join(&self.nodes[at * 2], &self.nodes[at * 2 + 1], &mut work);
            work.input_nodes += 1;
        }
        self.record_input_work(work);
    }
    fn finalize_leaf(&mut self, leaf: usize) {
        let begin = leaf * RUNS_PER_LEAF;
        let end = (leaf + 1) * RUNS_PER_LEAF;
        let mut work = DisplaySourceWork::default();
        let mut at = self.leaves + leaf;
        self.nodes[at] = Pair {
            raw: self.summarize_runs(begin, end, 0, self.scalars, false, &mut work),
            code: self.summarize_runs(begin, end, 0, self.scalars, true, &mut work),
        };
        work.input_nodes += 1;
        while at > 1 {
            at /= 2;
            self.nodes[at] = Pair::join(&self.nodes[at * 2], &self.nodes[at * 2 + 1], &mut work);
            work.input_nodes += 1;
        }
        self.record_input_work(work);
    }
    fn range(
        &self,
        id: usize,
        extent: std::ops::Range<usize>,
        selected: std::ops::Range<usize>,
        code: bool,
        w: &mut DisplaySourceWork,
    ) -> DisplaySourceSummary {
        w.query_nodes += 1;
        if selected.end <= extent.start || extent.end <= selected.start {
            return DisplaySourceSummary::default();
        }
        if selected.start <= extent.start && extent.end <= selected.end {
            return if code {
                self.nodes[id].code.clone()
            } else {
                self.nodes[id].raw.clone()
            };
        }
        let mid = (extent.start + extent.end) / 2;
        let left = self.range(id * 2, extent.start..mid, selected.clone(), code, w);
        let right = self.range(id * 2 + 1, mid..extent.end, selected, code, w);
        DisplaySourceSummary::join(&left, &right, w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_appends_finalize_each_source_page_once_and_keep_the_query_tail_bounded() {
        for unit in ["x", "ab\né界"] {
            for repeats in [128, 512, 2048] {
                let text = unit.repeat(repeats);
                let mut source = DisplaySource::default();
                for (at, ch) in text.char_indices() {
                    source.append(ch.encode_utf8(&mut [0; 4]), at);
                }
                let stats = source.stats();
                let n = text.chars().count();
                assert_eq!(stats.work.decoded_scalars, n);
                assert_eq!(
                    stats.work.input_runs,
                    stats.runs.saturating_sub(1) / RUNS_PER_LEAF * RUNS_PER_LEAF * 2
                );
                let height = n.div_ceil(RUNS_PER_LEAF).next_power_of_two().ilog2() as usize;
                assert!(
                    stats.work.width_state_transitions
                        <= 264 * (n * (height + 1) + 2 * n.div_ceil(RUNS_PER_LEAF)),
                    "{stats:?}"
                );
                let mut bulk = DisplaySource::default();
                bulk.append(&text, 0);
                for code in [false, true] {
                    let streamed = source.summary(0, n, code, &mut DisplaySourceWork::default());
                    let bulk = bulk.summary(0, n, code, &mut DisplaySourceWork::default());
                    assert_eq!(streamed.gcb.out, bulk.gcb.out);
                    assert_eq!(streamed.gcb.breaks, bulk.gcb.breaks);
                    assert_eq!(streamed.width, bulk.width);
                    assert_eq!(
                        (
                            streamed.bytes,
                            streamed.scalars,
                            streamed.ascii,
                            streamed.first,
                            streamed.last
                        ),
                        (bulk.bytes, bulk.scalars, bulk.ascii, bulk.first, bulk.last)
                    );
                }
                println!(
                    "incremental native input: unit={unit:?} repeats={repeats} stats={stats:?}"
                );
            }
        }
    }

    #[test]
    fn indexed_source_ranges_match_sequential_transducers_and_source_atoms_after_append() {
        let source = DisplaySource::default();
        let mut source = source;
        let mut text = String::new();
        for batch in ["ab界é\r", "\n\t\0", "🇦🇧👩‍💻a\u{301}", &"ab界é".repeat(300)]
        {
            source.append(batch, text.len());
            text.push_str(batch);
            let scalars: Vec<_> = text.char_indices().collect();
            assert_eq!(source.len(), scalars.len());
            for (at, &(byte, ch)) in scalars.iter().enumerate() {
                assert_eq!(source.position(at), byte);
                assert_eq!(source.atom(at).scalar, ch);
                for interior in byte..byte + ch.len_utf8() {
                    assert_eq!(
                        source.lower_bound(interior),
                        at + usize::from(interior != byte)
                    );
                }
            }
            for lo in (0..scalars.len()).step_by(37).chain([0, scalars.len() / 2]) {
                for hi in [
                    lo,
                    (lo + 1).min(scalars.len()),
                    (lo + 259).min(scalars.len()),
                    scalars.len(),
                ] {
                    for code in [false, true] {
                        let mut expected = DisplaySourceSummary::default();
                        for at in lo..hi {
                            let a = source.atom(at);
                            if let Some((ch, p)) = a.displayed(code) {
                                expected.gcb.push(p);
                                expected.width.push(p);
                                expected.bytes += ch.len_utf8();
                                expected.scalars += 1;
                                expected.ascii &= ch.is_ascii();
                                expected.first = expected.first.or(Some(a.envelope(code)));
                                expected.last = Some(a.envelope(code));
                            }
                        }
                        let mut w = DisplaySourceWork::default();
                        let actual = source.summary(lo, hi, code, &mut w);
                        assert_eq!(actual.gcb.out, expected.gcb.out, "{lo}..{hi} code={code}");
                        assert_eq!(
                            actual.gcb.breaks, expected.gcb.breaks,
                            "{lo}..{hi} code={code}"
                        );
                        assert_eq!(actual.width, expected.width, "{lo}..{hi} code={code}");
                        assert_eq!(
                            (
                                actual.bytes,
                                actual.scalars,
                                actual.ascii,
                                actual.first,
                                actual.last
                            ),
                            (
                                expected.bytes,
                                expected.scalars,
                                expected.ascii,
                                expected.first,
                                expected.last
                            )
                        );
                        assert!(w.query_runs <= 2 * RUNS_PER_LEAF);
                    }
                }
            }
        }
        assert_eq!(source.stats().work.decoded_bytes, text.len());
    }
}
