use std::{collections::BTreeMap, fmt};

use crate::{
    Budget, Completion, DisplayCursor, DisplayStep, Document, DocumentId, HeadingId, LayoutError,
    Style, TILE_BYTES, WorkLimit,
};

/// Borrowed base plus a collision suffix. Formatting/copying the complete slug
/// costs O(slug bytes) in the caller; index queries never copy a long title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorSlug<'a> {
    pub base: &'a str,
    pub suffix: Option<usize>,
}

impl fmt::Display for AnchorSlug<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.base)?;
        if let Some(suffix) = self.suffix {
            write!(f, "-{suffix}")?;
        }
        Ok(())
    }
}

/// Exact source target for NavAction::Source / FoldNavigation::Reveal.
/// Heading targets and slugs are provisional until Document::finish, and all
/// results must be discarded on revision changes. None denotes document start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorTarget {
    pub heading: Option<HeadingId>,
    pub source: usize,
    pub provisional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadingAnchor<'a> {
    pub slug: AnchorSlug<'a>,
    pub target: AnchorTarget,
}

struct Entry {
    base: String,
    suffix: Option<usize>,
}

#[derive(Default)]
struct Node {
    // At most 256 edges: no long string comparisons or adversarial hashing.
    children: BTreeMap<u8, usize>,
    heading: Option<HeadingId>,
    next_suffix: usize,
}

struct Building {
    cursor: DisplayCursor,
    reading: bool,
    base: String,
    node: usize,
    base_node: usize,
    suffix: Option<usize>,
    pending: String,
    pending_pos: usize,
    // Unicode lowercase has at most three scalars; decimal usize plus '-'
    // also fits. All trie work on these bytes is separately charged.
    emitted: [u8; 32],
    emitted_len: usize,
    emitted_pos: usize,
}

/// Explicit, budgeted slug index for one revision. Construction and parsing
/// never scan historical heading text. Call advance to read only indexed titles.
///
/// Dialect: non-HIDDEN display text; Unicode alphanumerics lowercase, '_'/'-'
/// retained, each whitespace scalar becomes '-', everything else removed. No
/// normalization, trimming or whitespace collapsing. Empty bases use "section".
/// Source order claims the first unused base, base-1, base-2, ... globally.
///
/// Storage O(S + H), for title source bytes S and heading count H. Suffixes
/// are stored as integers and shared trie paths, never copies of long bases.
/// Per revision work is O((S + 1) log(n + 1) + H log(H + 1)), including duplicate
/// probes. Allocations are amortized, not a hard wall-clock bound.
pub struct HeadingAnchors {
    document: DocumentId,
    revision: u64,
    heading_count: usize,
    entries: Vec<Entry>,
    nodes: Vec<Node>,
    building: Option<Building>,
    resolver: crate::LinkResolver,
    retiring: bool,
}

impl HeadingAnchors {
    pub fn new(document: &Document) -> Self {
        Self {
            document: document.id(),
            revision: document.revision(),
            heading_count: document.headings().entries().len(),
            entries: Vec::new(),
            nodes: vec![Node::default()],
            building: None,
            resolver: document.link_resolver(),
            retiring: false,
        }
    }

    /// Selection memo only; the separately paid slug trie retains its lifecycle.
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        let empty = self.resolver.empty_like();
        std::mem::replace(&mut self.resolver, empty)
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        resolver.check_binding(self.document, self.revision)?;
        std::mem::swap(&mut self.resolver, resolver);
        Ok(())
    }
    pub fn retire_link_resolver(&mut self, budget: &mut Budget) -> Completion {
        self.resolver.retire(budget)
    }
    /// Pay each retained trie edge/node and title record before destruction.
    /// This is also used by the one active hosted heading index on switch.
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        if self.resolver.retire(budget) != Completion::Covered {
            return Completion::BudgetExhausted;
        }
        self.retiring = true;
        if let Some(building) = &mut self.building {
            if building.cursor.retire(budget) != Completion::Covered {
                return Completion::BudgetExhausted;
            }
            if !step(budget) {
                return Completion::BudgetExhausted;
            }
            self.building = None;
        }
        while !self.entries.is_empty() {
            if !step(budget) {
                return Completion::BudgetExhausted;
            }
            self.entries.pop();
        }
        while let Some(node) = self.nodes.last_mut() {
            if !step(budget) {
                return Completion::BudgetExhausted;
            }
            if node.children.pop_first().is_none() {
                self.nodes.pop();
            }
        }
        Completion::Covered
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn indexed_headings(&self) -> usize {
        self.entries.len()
    }

    pub fn is_complete(&self) -> bool {
        self.entries.len() == self.heading_count
    }

    fn validate(&self, document: &Document) -> Result<(), LayoutError> {
        if document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if self.retiring || document.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        Ok(())
    }

    /// O(1), borrowing a completed slug. None means not indexed yet; an unknown
    /// ID is InvalidRange. Even finished indexes reject stale/wrong documents.
    pub fn get(
        &self,
        document: &Document,
        id: HeadingId,
    ) -> Result<Option<HeadingAnchor<'_>>, LayoutError> {
        self.validate(document)?;
        if id.0 >= self.heading_count {
            return Err(LayoutError::InvalidRange);
        }
        Ok(self.entries.get(id.0).map(|entry| HeadingAnchor {
            slug: AnchorSlug {
                base: &entry.base,
                suffix: entry.suffix,
            },
            target: target(document, id),
        }))
    }

    /// Reads reuse DisplayCursor charges. Each normalized scalar costs its
    /// input UTF-8 bytes + one object; each emitted trie byte costs one byte +
    /// one object. Heading setup and candidate checks cost one query + object.
    /// Six bytes, one query and one object per frame suffice for progress.
    /// Append/first finish invalidate this task and all borrowed/copied results;
    /// rebuilding is explicit and repeated rebuild work belongs to the caller.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Completion, LayoutError> {
        self.validate(document)?;
        self.resolver.check(document)?;
        loop {
            if self.is_complete() {
                return Ok(Completion::EndOfDocument);
            }
            let Some(current) = &mut self.building else {
                if !step(budget) {
                    return Ok(Completion::BudgetExhausted);
                }
                let content = document.headings().entries()[self.entries.len()].content;
                self.building = Some(Building {
                    cursor: document.display_cursor(content).unwrap(),
                    reading: true,
                    base: String::new(),
                    node: 0,
                    base_node: 0,
                    suffix: None,
                    pending: String::new(),
                    pending_pos: 0,
                    emitted: [0; 32],
                    emitted_len: 0,
                    emitted_pos: 0,
                });
                continue;
            };
            if current.emitted_pos < current.emitted_len {
                if !budget.charge(WorkLimit {
                    bytes: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                let byte = current.emitted[current.emitted_pos];
                let next = self.nodes[current.node].children.get(&byte).copied();
                current.node = next.unwrap_or_else(|| {
                    let next = self.nodes.len();
                    self.nodes.push(Node::default());
                    self.nodes[current.node].children.insert(byte, next);
                    next
                });
                current.emitted_pos += 1;
                continue;
            }
            if current.reading {
                if current.pending_pos == current.pending.len() {
                    match current
                        .cursor
                        .next_with_resolver(document, &mut self.resolver, budget)?
                    {
                        DisplayStep::BudgetExhausted => return Ok(Completion::BudgetExhausted),
                        DisplayStep::Complete => {
                            current.reading = false;
                            if current.base.is_empty() {
                                current.base.push_str("section");
                                current.emitted[..7].copy_from_slice(b"section");
                                current.emitted_len = 7;
                                current.emitted_pos = 0;
                            }
                            continue;
                        }
                        DisplayStep::Run(run) => {
                            if run.style.contains(Style::HIDDEN) || run.text.is_empty() {
                                continue;
                            }
                            debug_assert!(run.text.len() <= TILE_BYTES);
                            current.pending.clear();
                            current.pending.push_str(run.text);
                            current.pending_pos = 0;
                        }
                    }
                }
                let ch = current.pending[current.pending_pos..]
                    .chars()
                    .next()
                    .unwrap();
                if !budget.charge(WorkLimit {
                    bytes: ch.len_utf8(),
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(Completion::BudgetExhausted);
                }
                current.pending_pos += ch.len_utf8();
                current.emitted_len = 0;
                current.emitted_pos = 0;
                if ch.is_alphanumeric() {
                    for lower in ch.to_lowercase() {
                        current.emit_char(lower);
                    }
                } else if ch == '-' || ch == '_' {
                    current.emit_char(ch);
                } else if ch.is_whitespace() {
                    current.emit_char('-');
                }
                continue;
            }
            if !step(budget) {
                return Ok(Completion::BudgetExhausted);
            }
            if self.nodes[current.node].heading.is_none() {
                self.nodes[current.node].heading = Some(HeadingId(self.entries.len()));
                let completed = self.building.take().unwrap();
                self.entries.push(Entry {
                    base: completed.base,
                    suffix: completed.suffix,
                });
                continue;
            }
            if current.suffix.is_none() {
                current.base_node = current.node;
            }
            // This counter never restarts. Each failed candidate already names
            // an earlier heading, and only one base can generate that suffix.
            let base = &mut self.nodes[current.base_node];
            let suffix = base.next_suffix.max(1);
            base.next_suffix = suffix.checked_add(1).expect("heading count fits usize");
            current.suffix = Some(suffix);
            current.node = current.base_node;
            current.emitted_pos = 0;
            current.emitted_len = decimal_suffix(suffix, &mut current.emitted);
        }
    }

    /// Borrow an index and an already extracted URI fragment. Construction is
    /// O(1), requires '#', and does not copy/decode the fragment. Full URLs and
    /// Markdown destination entity/backslash decoding belong to the caller.
    pub fn fragment<'a>(&'a self, fragment: &'a str) -> Result<FragmentLookup<'a>, LayoutError> {
        if !fragment.starts_with('#') {
            return Err(LayoutError::InvalidRange);
        }
        Ok(FragmentLookup {
            index: self,
            fragment: fragment.as_bytes(),
            state: FragmentState {
                position: 1,
                node: Some(0),
                utf8: [0; 4],
                utf8_len: 0,
                utf8_need: 0,
                result: None,
                invalid: false,
            },
        })
    }
}

impl Building {
    fn emit_char(&mut self, ch: char) {
        self.base.push(ch);
        let len = ch.encode_utf8(&mut self.emitted[self.emitted_len..]).len();
        self.emitted_len += len;
    }
}

fn step(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn target(document: &Document, id: HeadingId) -> AnchorTarget {
    AnchorTarget {
        heading: Some(id),
        source: document.headings().get(id).unwrap().source.start,
        provisional: !document.is_finished(),
    }
}

fn decimal_suffix(mut number: usize, out: &mut [u8; 32]) -> usize {
    let mut reversed = [0; 32];
    let mut len = 0;
    loop {
        reversed[len] = b'0' + (number % 10) as u8;
        len += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    out[0] = b'-';
    for i in 0..len {
        out[i + 1] = reversed[len - i - 1];
    }
    len + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentStep {
    BudgetExhausted,
    /// Drop the lookup, advance the index, then create a new lookup.
    IndexIncomplete,
    Resolved(AnchorTarget),
    /// Absent in this revision, but more input may supply the target.
    Unresolved,
    /// Absent after Document::finish.
    NotFound,
}

/// O(1) task storage. Decodes percent escapes once and validates UTF-8 while
/// walking the trie. '+' stays literal; matching is case-sensitive with no slug
/// normalization. Malformed escapes/UTF-8 return InvalidRange, even after a
/// missing trie edge. '#' resolves to source zero, not an empty heading.
#[derive(Clone, Copy)]
struct FragmentState {
    position: usize,
    node: Option<usize>,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
    result: Option<FragmentStep>,
    invalid: bool,
}
pub struct FragmentLookup<'a> {
    index: &'a HeadingAnchors,
    fragment: &'a [u8],
    state: FragmentState,
}
/// An owned continuation of the same fragment decoder. The index remains in
/// its single renderer owner; this task retains no borrowed index or title.
pub struct FragmentContinuation {
    document: DocumentId,
    revision: u64,
    raw: std::sync::Arc<String>,
    state: FragmentState,
}
impl HeadingAnchors {
    pub fn fragment_owned(
        &self,
        raw: std::sync::Arc<String>,
    ) -> Result<FragmentContinuation, LayoutError> {
        let state = self.fragment(&raw)?.state;
        Ok(FragmentContinuation {
            document: self.document,
            revision: self.revision,
            raw,
            state,
        })
    }
}
impl FragmentContinuation {
    pub fn advance(
        &mut self,
        index: &HeadingAnchors,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<FragmentStep, LayoutError> {
        index.validate(document)?;
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        let mut lookup = FragmentLookup {
            index,
            fragment: self.raw.as_bytes(),
            state: self.state,
        };
        let result = lookup.advance(document, budget);
        self.state = lookup.state;
        result
    }
}

impl FragmentLookup<'_> {
    /// Each raw byte/percent triplet costs 1/3 bytes and one object; final
    /// resolution costs one query and one object. Invalid input is charged;
    /// wrong/stale documents fail before all charges or state changes.
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<FragmentStep, LayoutError> {
        self.index.validate(document)?;
        if self.state.invalid {
            return Err(LayoutError::InvalidRange);
        }
        if let Some(result) = self.state.result {
            return Ok(result);
        }
        if !self.index.is_complete() {
            return Ok(FragmentStep::IndexIncomplete);
        }
        while self.state.position < self.fragment.len() {
            let encoded = self.fragment[self.state.position] == b'%';
            let len = if encoded { 3 } else { 1 }.min(self.fragment.len() - self.state.position);
            if !budget.charge(WorkLimit {
                bytes: len,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(FragmentStep::BudgetExhausted);
            }
            let byte = if encoded {
                let pair = &self.fragment[self.state.position + 1..self.state.position + len];
                if let [hi, lo] = pair
                    && let (Some(hi), Some(lo)) = (hex(*hi), hex(*lo))
                {
                    (hi << 4) | lo
                } else {
                    self.state.invalid = true;
                    return Err(LayoutError::InvalidRange);
                }
            } else {
                self.fragment[self.state.position]
            };
            self.state.position += len;
            if !self.valid_byte(byte) {
                self.state.invalid = true;
                return Err(LayoutError::InvalidRange);
            }
            self.state.node = self
                .state
                .node
                .and_then(|node| self.index.nodes[node].children.get(&byte).copied());
        }
        if self.state.utf8_len != 0 {
            self.state.invalid = true;
            return Err(LayoutError::InvalidRange);
        }
        if !step(budget) {
            return Ok(FragmentStep::BudgetExhausted);
        }
        let result = if self.fragment.len() == 1 {
            FragmentStep::Resolved(AnchorTarget {
                heading: None,
                source: 0,
                provisional: false,
            })
        } else if let Some(id) = self
            .state
            .node
            .and_then(|node| self.index.nodes[node].heading)
        {
            FragmentStep::Resolved(target(document, id))
        } else if document.is_finished() {
            FragmentStep::NotFound
        } else {
            FragmentStep::Unresolved
        };
        self.state.result = Some(result);
        Ok(result)
    }

    fn valid_byte(&mut self, byte: u8) -> bool {
        if self.state.utf8_len == 0 {
            self.state.utf8_need = match byte {
                0..=0x7f => return true,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return false,
            };
        }
        self.state.utf8[self.state.utf8_len] = byte;
        self.state.utf8_len += 1;
        if self.state.utf8_len == self.state.utf8_need {
            let valid = std::str::from_utf8(&self.state.utf8[..self.state.utf8_len]).is_ok();
            self.state.utf8_len = 0;
            valid
        } else {
            true
        }
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_suffix_probes_and_storage_do_not_restart_or_copy_bases() {
        let count = 20_000;
        let mut doc = Document::new();
        // Occupy numeric names first, forcing the repeated base to skip them.
        for i in 1..=count {
            doc.append(&format!("# x-{i}\n")).unwrap();
        }
        doc.append(&"# x\n".repeat(count)).unwrap();
        doc.finish();
        let mut index = HeadingAnchors::new(&doc);
        let mut budget = Budget::new(WorkLimit::unlimited());
        assert_eq!(
            index.advance(&doc, &mut budget),
            Ok(Completion::EndOfDocument)
        );
        // Each duplicate checks its base, then advances a monotonic counter.
        assert!(budget.used().queries < count * 20);
        assert!(budget.used().bytes < doc.source().len() * 15);
        // Decimal prefixes are shared. There is no per-heading suffix String.
        assert!(index.nodes.len() < count * 3);
        assert_eq!(index.entries.last().unwrap().suffix, Some(count * 2 - 1));
        assert_eq!(index.entries.last().unwrap().base, "x");
    }
}
