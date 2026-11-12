//! Dependency-free, versioned test driver. Never linked into the parser library.
#[path = "snapshot.rs"]
mod semantic;

use mdstream::{BLOCK_LOOKAHEAD, Document, LinkTarget, Span, Style};
use semantic::{Snapshot, snapshot};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub const SEEDS: [u64; 4] = [0, 1, 0x1234_5678, 0xdead_beef_cafe_babe];
pub const VERSION: &str = "mdstream-syntax-fuzz-v1";

// Wrapping arithmetic and modulo before usize conversion keep seeds portable.
struct Rng(u64);
impl Rng {
    fn pick(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((z ^ (z >> 31)) % n as u64) as usize
    }

    fn inline(&mut self, depth: usize) -> String {
        let atoms = ["text", "中🙂é", " &amp; ", "\\*", "\t", " ", "&#x1f642;"];
        if depth == 0 {
            return atoms[self.pick(atoms.len())].to_owned();
        }
        let inner = self.inline(depth - 1);
        match self.pick(12) {
            0 => format!("*{inner}*"),
            1 => format!("**{inner}**"),
            2 => format!("~~{inner}~~"),
            3 => format!("` {inner} `"),
            4 => format!("[{inner}](<a b> 'title')"),
            5 => format!("![{inner}][ref]"),
            6 => format!("${inner}$"),
            7 => "[^note] [ref]".into(),
            8 => "<https://example.test/a>".into(),
            9 => format!("[open *{inner}"),
            10 => format!("{inner}  \ncontinued\\\nend"),
            _ => inner,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    None,
    LoseStyle,
    BadRange,
    SplitCrLf,
}

impl Fault {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LoseStyle => "lose-style",
            Self::BadRange => "bad-range",
            Self::SplitCrLf => "split-crlf",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "none" => Ok(Self::None),
            "lose-style" => Ok(Self::LoseStyle),
            "bad-range" => Ok(Self::BadRange),
            "split-crlf" => Ok(Self::SplitCrLf),
            _ => Err(format!("unknown fault: {s}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub seed: u64,
    pub index: usize,
    pub fault: Fault,
    pub source: String,
    /// Absolute UTF-8 byte ends, nondecreasing; repeats exercise empty appends.
    /// The remaining suffix is always appended, then finish is called twice.
    pub cuts: Vec<usize>,
}

impl Case {
    pub fn validate(&self) -> Result<(), String> {
        let mut start = 0;
        for &end in &self.cuts {
            if end < start || !self.source.is_char_boundary(end) {
                return Err(format!("invalid chunk endpoint {end} after {start}"));
            }
            start = end;
        }
        Ok(())
    }

    pub fn encode(&self) -> String {
        let cuts = self
            .cuts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{VERSION}\nseed={}\ncase={}\nfault={}\ncuts={cuts}\n\n{}",
            self.seed,
            self.index,
            self.fault.name(),
            self.source
        )
    }

    pub fn decode(text: &str) -> Result<Self, String> {
        let (header, source) = text.split_once("\n\n").ok_or("missing header separator")?;
        let mut lines = header.split('\n');
        if lines.next() != Some(VERSION) {
            return Err("unsupported case version".into());
        }
        let mut field = |prefix: &str| -> Result<&str, String> {
            lines
                .next()
                .and_then(|line| line.strip_prefix(prefix))
                .ok_or_else(|| format!("missing {prefix}"))
        };
        let seed = field("seed=")?.parse().map_err(|_| "invalid seed")?;
        let index = field("case=")?.parse().map_err(|_| "invalid case index")?;
        let fault = Fault::parse(field("fault=")?)?;
        let raw_cuts = field("cuts=")?;
        let cuts = if raw_cuts.is_empty() {
            Vec::new()
        } else {
            raw_cuts
                .split(',')
                .map(|s| s.parse().map_err(|_| "invalid cut".to_owned()))
                .collect::<Result<Vec<_>, _>>()?
        };
        if lines.next().is_some() {
            return Err("extra header fields".into());
        }
        let case = Self {
            seed,
            index,
            fault,
            source: source.into(),
            cuts,
        };
        case.validate()?;
        Ok(case)
    }
}

/// Twelve rotating block families, recursive inline productions, nested explicit
/// prefixes, malformed tails, LF/CRLF and classification-window boundary lengths.
/// No standard HTML expectations are derived from this generator.
pub fn generate(seed: u64, index: usize) -> Case {
    let mut rng = Rng(seed ^ (index as u64).wrapping_mul(0xd1342543de82ef95));
    let mut source = String::new();
    for part in 0..3 {
        let body = rng.inline(2);
        let block = match (index + part) % 12 {
            0 => format!("{} {body} ### \t\n", "#".repeat(1 + rng.pick(6))),
            1 => format!(" {body} \t\nsecond\n===\n"),
            2 => format!("{body}\ncontinued **strong**\n"),
            3 => format!("```rs\n{body}\n```\n"),
            4 => format!("{body}|b\n:--|--:\n`x\\|y`|**z**\n"),
            5 => format!("- [x] {body}\n  - [ ] nested\n- sibling\n"),
            6 => format!("[ref] ![alt][ref]\n\n[ref]: /late 'title'\n{body}\n"),
            7 => format!("[^note]\n\n[^note]: {body}\n    continuation\n"),
            8 => format!("$$\n{body}\n$$\n"),
            9 => format!("    {body}\n    code\n"),
            10 => format!(
                "*open\n{}end*|b\n-|-\n",
                "x".repeat(BLOCK_LOOKAHEAD + rng.pick(7) - 3)
            ),
            _ => format!("{body}\n[unfinished](<bad\n&amp; `open $open\n"),
        };
        let prefix = ["", "> ", "> > ", "> - "][rng.pick(4)];
        let continuation = if prefix == "> - " { ">   " } else { prefix };
        for (line, text) in block.split_inclusive('\n').enumerate() {
            source.push_str(if line == 0 { prefix } else { continuation });
            source.push_str(text);
        }
        source.push('\n');
    }
    if rng.pick(2) == 0 {
        source = source.replace('\n', "\r\n");
    }
    if rng.pick(3) == 0 {
        // Retain arbitrary incomplete syntax without cutting inside UTF-8.
        let ends: Vec<_> = source.char_indices().map(|(i, _)| i).collect();
        source.truncate(ends[rng.pick(ends.len())]);
    }
    let cuts = match index % 3 {
        0 => source.char_indices().map(|(i, _)| i).collect(),
        1 => source
            .char_indices()
            .filter_map(|(i, _)| (rng.pick(7) == 0).then_some(i))
            .collect(),
        _ => source
            .char_indices()
            .filter_map(|(i, c)| "\r\n*_`[]|$>\t".contains(c).then_some(i))
            .collect(),
    };
    Case {
        seed,
        index,
        source,
        cuts,
        fault: Fault::None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Stable property/phase signature; reducers must preserve this signature.
    pub property: String,
    pub detail: String,
}

fn require(ok: bool, property: &str, detail: impl std::fmt::Debug) -> Result<(), Failure> {
    if ok {
        Ok(())
    } else {
        Err(Failure {
            property: property.into(),
            detail: format!("{detail:?}"),
        })
    }
}

fn span(source: &str, range: Span) -> Result<(), Failure> {
    require(source.get(range.range()).is_some(), "range.utf8", range)
}

fn within(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.start <= inner.end && inner.end <= outer.end
}

fn invariants(s: &Snapshot, finished: bool) -> Result<(), Failure> {
    let source = &s.source;
    let mut end = 0;
    for block in &s.blocks {
        span(source, block.source)?;
        require(block.source.start == end, "blocks.coverage", block)?;
        end = block.source.end;
        if let Some(id) = block.container {
            require(
                s.containers
                    .get(id.0)
                    .is_some_and(|c| within(c.source, block.source)),
                "blocks.container",
                block,
            )?;
        }
    }
    if finished {
        require(end == source.len(), "blocks.eof", end)?;
    }
    let mut end = 0;
    for range in &s.tiles {
        span(source, *range)?;
        require(
            range.start == end && !range.is_empty(),
            "tiles.coverage",
            range,
        )?;
        end = range.end;
    }
    require(end == source.len(), "tiles.eof", end)?;
    for (i, c) in s.containers.iter().enumerate() {
        span(source, c.source)?;
        if let Some(id) = c.parent {
            require(id.0 < i, "container.parent_order", c)?;
            let p = &s.containers[id.0];
            require(
                p.depth + 1 == c.depth && within(p.source, c.source),
                "container.parent_range",
                c,
            )?;
        } else {
            require(c.depth == 1, "container.root_depth", c)?;
        }
    }
    for h in &s.headings {
        span(source, h.source)?;
        span(source, h.content)?;
        require(
            (1..=6).contains(&h.level) && h.block.0 < s.blocks.len() && within(h.source, h.content),
            "heading.structure",
            h,
        )?;
    }
    for t in &s.tables {
        span(source, t.source)?;
        span(source, t.delimiter)?;
        require(
            t.block.0 < s.blocks.len() && !t.columns.is_empty() && within(t.source, t.delimiter),
            "table.structure",
            t,
        )?;
        let mut end = t.source.start;
        for row in t.rows() {
            span(source, row.source)?;
            require(
                within(t.source, row.source)
                    && end <= row.source.start
                    && row.cells.len() <= t.columns.len(),
                "table.row",
                row,
            )?;
            end = row.source.end;
            let mut cell_end = row.source.start;
            for cell in &row.cells {
                span(source, cell.source)?;
                require(
                    within(row.source, cell.source) && cell_end <= cell.source.start,
                    "table.cell",
                    cell,
                )?;
                cell_end = cell.source.end;
            }
        }
    }
    for m in &s.maths {
        span(source, m.source)?;
        span(source, m.content)?;
        require(within(m.source, m.content), "math.content", m)?;
    }
    require(
        s.links.len() == s.link_info.len(),
        "links.count",
        s.links.len(),
    )?;
    for (target, link) in s.links.iter().zip(&s.link_info) {
        span(source, link.source)?;
        span(source, link.label)?;
        require(within(link.source, link.label), "link.label", link)?;
        if let Some(title) = link.title {
            span(source, title)?;
        }
        match target {
            LinkTarget::Reference(id) => require(id.0 < s.references.len(), "link.symbol", target)?,
            LinkTarget::Direct(r) | LinkTarget::Email(r) | LinkTarget::Www(r) => span(source, *r)?,
        }
    }
    for r in &s.references {
        for range in [r.destination, r.title].into_iter().flatten() {
            span(source, range)?;
        }
        if let Some(at) = r.first_use {
            require(source.is_char_boundary(at), "reference.first_use", at)?;
        }
    }
    for f in &s.footnote_references {
        span(source, f.source)?;
        require(f.symbol.0 < s.footnotes.len(), "footnote.symbol", f)?;
    }
    for f in &s.footnotes {
        if let Some(d) = f.definition {
            span(source, d.content)?;
            require(d.block.0 < s.blocks.len(), "footnote.block", d)?;
        }
    }
    for chunks in s
        .math_payloads
        .iter()
        .chain(&s.math_block_payloads)
        .chain(&s.footnote_payloads)
    {
        let mut end = 0;
        for (range, _) in chunks {
            span(source, *range)?;
            require(end <= range.start, "payload.source_order", range)?;
            end = range.end;
        }
    }
    let mut end = 0;
    for (range, block, _, link) in &s.runs {
        span(source, *range)?;
        require(
            range.start == end && !range.is_empty(),
            "runs.coverage",
            range,
        )?;
        end = range.end;
        require(
            block.is_none_or(|id| id.0 < s.blocks.len())
                && link.is_none_or(|id| id.0 < s.links.len()),
            "runs.ids",
            range,
        )?;
    }
    require(end == source.len(), "runs.eof", end)?;
    let mut end = 0;
    for (range, replacement, _) in &s.display {
        span(source, *range)?;
        require(
            range.start == end && !range.is_empty(),
            "display.coverage",
            range,
        )?;
        end = range.end;
        if let Some(replacement) = replacement {
            span(source, *replacement)?;
        }
    }
    require(end == source.len(), "display.eof", end)?;
    Ok(())
}

fn compare(
    whole: &Document,
    split: &Document,
    fault: Fault,
    finished: bool,
) -> Result<(), Failure> {
    let expected = snapshot(whole);
    let mut actual = snapshot(split);
    // Mutation at the observation boundary leaves production code untouched and
    // exercises the same invariant/comparison path used for real failures.
    match fault {
        Fault::None | Fault::SplitCrLf => {}
        Fault::LoseStyle => {
            if let Some(run) = actual
                .runs
                .iter_mut()
                .find(|run| run.2.contains(Style::STRONG))
            {
                run.2 = Style::default();
            }
        }
        Fault::BadRange => {
            if let Some(block) = actual.blocks.first_mut() {
                block.source.end = actual.source.len() + 1;
            }
        }
    }
    invariants(&expected, finished)?;
    invariants(&actual, finished)?;
    macro_rules! fields {
        ($($name:ident),+ $(,)?) => { $(
            require(expected.$name == actual.$name, concat!("semantic.", stringify!($name)),
                format!("prefix={} finished={finished}", whole.source().len()))?;
        )+ };
    }
    fields!(
        source,
        blocks,
        containers,
        tiles,
        references,
        links,
        link_info,
        headings,
        tables,
        maths,
        math_payloads,
        math_block_payloads,
        footnotes,
        footnote_payloads,
        footnote_references,
        runs,
        display
    );
    Ok(())
}

fn check_inner(case: &Case) -> Result<(), Failure> {
    case.validate().map_err(|detail| Failure {
        property: "case.format".into(),
        detail,
    })?;
    let mut split = Document::new();
    let mut start = 0;
    for end in case.cuts.iter().copied().chain([case.source.len()]) {
        let mut chunk = &case.source[start..end];
        // Deliberately broken adapter: loses LF only when CR and LF arrive in
        // distinct nonempty chunks. Reduction must retain that exact split.
        if case.fault == Fault::SplitCrLf
            && case.source[..start].ends_with('\r')
            && chunk.starts_with('\n')
        {
            chunk = &chunk[1..];
        }
        split.append(chunk).unwrap();
        let mut whole = Document::new();
        whole.append(&case.source[..end]).unwrap();
        require(
            split.pending().len() <= BLOCK_LOOKAHEAD + 3,
            "pending.bound",
            split.pending().len(),
        )?;
        compare(&whole, &split, case.fault, false).map_err(|mut f| {
            f.property = format!("live.{}", f.property);
            f
        })?;
        start = end;
    }
    let mut whole = Document::new();
    whole.append(&case.source).unwrap();
    whole.finish();
    split.finish();
    compare(&whole, &split, case.fault, true).map_err(|mut f| {
        f.property = format!("finished.{}", f.property);
        f
    })?;
    let before = snapshot(&split);
    split.finish();
    require(
        split.append("").is_err(),
        "finish.append_rejected",
        "append after EOF",
    )?;
    require(
        before == snapshot(&split),
        "finish.idempotent",
        "repeat finish/empty append",
    )
}

pub fn check(case: &Case) -> Result<(), Failure> {
    catch_unwind(AssertUnwindSafe(|| check_inner(case))).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "non-string panic".into());
        Err(Failure {
            property: format!("panic:{message}"),
            detail: message,
        })
    })
}

fn delete(case: &Case, start: usize, end: usize) -> Case {
    let mut result = case.clone();
    result.source.replace_range(start..end, "");
    for cut in &mut result.cuts {
        *cut = if *cut <= start {
            *cut
        } else if *cut < end {
            start
        } else {
            *cut - (end - start)
        };
    }
    result
}

/// Deterministic coarse-to-fine deletion, then chunk coalescing. Repeats to a
/// fixed point: no single Unicode scalar or explicit chunk end can be removed
/// while preserving the failure property. This is not a global minimum claim.
/// Terminates by decreasing (source bytes, cut count); no timing/random cutoff.
pub fn shrink(case: &Case, failure: &Failure) -> Case {
    let fails = |candidate: &Case| check(candidate).is_err_and(|f| f.property == failure.property);
    let mut best = case.clone();
    loop {
        let old = (best.source.len(), best.cuts.len());
        let mut width = best.source.chars().count();
        while width > 0 {
            let mut at = 0;
            loop {
                let boundaries: Vec<_> = best
                    .source
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain([best.source.len()])
                    .collect();
                if at + width >= boundaries.len() {
                    break;
                }
                let candidate = delete(&best, boundaries[at], boundaries[at + width]);
                if fails(&candidate) {
                    best = candidate;
                } else {
                    at += width;
                }
            }
            width /= 2;
        }
        let mut width = best.cuts.len();
        while width > 0 {
            let mut at = 0;
            while at + width <= best.cuts.len() {
                let mut candidate = best.clone();
                candidate.cuts.drain(at..at + width);
                if fails(&candidate) {
                    best = candidate;
                } else {
                    at += width;
                }
            }
            width /= 2;
        }
        if old == (best.source.len(), best.cuts.len()) {
            return best;
        }
    }
}
