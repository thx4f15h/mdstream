//! Finite metadata from existing typed objects. No image protocol is inferred.
use super::objects::SharedLinks;
use mdtui3::{Budget, Document, DocumentId, LayoutError, Span, WorkLimit};

pub(crate) struct MediaDetails {
    document: DocumentId,
    revision: u64,
    source: Span,
    objects: mdtui3::LinkWindow,
    visited: usize,
    image: Option<mdtui3::LinkId>,
    activity: Option<mdtui3::LinkActivity>,
    alt: Option<mdtui3::DisplayExcerpt>,
    target: Option<mdtui3::LinkDestinationTask>,
    phase: u8,
    alt_text: String,
    target_text: String,
    source_work: usize,
    charged_bytes: usize,
    alt_empty: bool,
    complete: bool,
}
impl MediaDetails {
    pub fn new(doc: &Document, source: Span) -> Result<Self, LayoutError> {
        if doc.text(source).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        let mut end = source.start.saturating_add(256).min(source.end);
        while !doc.source().is_char_boundary(end) {
            end -= 1;
        }
        Ok(Self {
            document: doc.id(),
            revision: doc.revision(),
            source,
            objects: doc.link_window(Span::new(source.start, end)),
            visited: 0,
            image: None,
            activity: None,
            alt: None,
            target: None,
            phase: 0,
            alt_text: String::new(),
            target_text: String::new(),
            source_work: 0,
            charged_bytes: 0,
            alt_empty: false,
            complete: false,
        })
    }
    fn append(text: &mut String, input: &str) {
        for c in input.chars() {
            let c = if c.is_control() { ' ' } else { c };
            if text.len() + c.len_utf8() > 32 {
                break;
            }
            text.push(c);
        }
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<String>, LayoutError> {
        // The canonical destination decoder guarantees one step with six
        // bytes. A smaller lifetime remainder ends this bounded detail rather
        // than repeatedly requesting an atom that can never fit that remainder.
        if 256 - self.charged_bytes < 6 {
            self.complete = true;
        }
        let mut limit = budget.remaining();
        limit.bytes = limit.bytes.min(256 - self.charged_bytes);
        let mut work = Budget::new(limit);
        let result = self.advance_inner(doc, links, &mut work);
        self.charged_bytes += work.used().bytes;
        assert!(budget.charge(work.used()));
        let result = result?;
        if result.is_none() && self.charged_bytes == 256 {
            self.complete = true;
        }
        Ok(result)
    }
    fn advance_inner(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<String>, LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if doc.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if !links.prepare(doc, budget)? {
            return Ok(None);
        }
        if !self.complete && self.source_work >= 256 {
            self.complete = true;
        }
        while !self.complete {
            match self.phase {
                0 => {
                    if self.visited >= 8 {
                        self.complete = true;
                        continue;
                    }
                    match self.objects.advance(doc, budget)? {
                        mdtui3::LinkStep::BudgetExhausted => return Ok(None),
                        mdtui3::LinkStep::Complete => {
                            self.complete = true;
                        }
                        mdtui3::LinkStep::Object(id, info) => {
                            self.visited += 1;
                            if info.kind == mdtui3::LinkKind::Image {
                                self.image = Some(id);
                                self.activity = doc.link_activity(id);
                                self.phase = 1;
                            }
                        }
                    }
                }
                1 => {
                    let Some(disposition) = links.with_resolver(|r| {
                        self.activity.as_mut().unwrap().advance_with(doc, r, budget)
                    })?
                    else {
                        return Ok(None);
                    };
                    if disposition != mdtui3::LinkDisposition::Active {
                        self.image = None;
                        self.activity = None;
                        self.phase = 0;
                        continue;
                    }
                    let id = self.image.unwrap();
                    let info = doc.link_info(id).ok_or(LayoutError::InvalidRange)?;
                    if info.source.end <= self.source.start || info.source.start >= self.source.end
                    {
                        return Err(LayoutError::InvalidRange);
                    }
                    self.alt = Some(mdtui3::DisplayExcerpt::new(
                        doc,
                        info.label,
                        mdtui3::ExcerptLimits {
                            source_bytes: 96,
                            display_bytes: 32,
                            lines: 1,
                        },
                    )?);
                    self.target = doc.link_destination_task(id);
                    self.phase = 2;
                }
                2 => {
                    let before = budget.used().bytes;
                    let step = links.with_resolver(|r| {
                        let alt = self.alt.as_mut().unwrap();
                        alt.swap_link_resolver(r)?;
                        let result = alt.next(doc, budget);
                        alt.swap_link_resolver(r)?;
                        result
                    })?;
                    self.source_work += budget.used().bytes - before;
                    match step {
                        mdtui3::ExcerptStep::BudgetExhausted => return Ok(None),
                        mdtui3::ExcerptStep::Run(run) => Self::append(&mut self.alt_text, run.text),
                        mdtui3::ExcerptStep::Complete(end) => {
                            self.alt_empty =
                                end == mdtui3::ExcerptEnd::SourceEnd && self.alt_text.is_empty();
                            self.alt = None;
                            self.phase = 3;
                        }
                    }
                }
                _ => {
                    let Some(target) = &mut self.target else {
                        self.complete = true;
                        continue;
                    };
                    let mut limit = budget.remaining();
                    limit.bytes = limit.bytes.min(256 - self.source_work);
                    let mut work = Budget::new(limit);
                    let result = target.advance(doc, &mut work)?;
                    self.source_work += work.used().bytes;
                    assert!(budget.charge(work.used()));
                    match result {
                        Ok(Some(chunk)) => {
                            Self::append(&mut self.target_text, chunk.text);
                            if self.target_text.len() >= 32 {
                                self.complete = true;
                            }
                        }
                        Ok(None) => self.complete = true,
                        Err(_) => {
                            if self.source_work >= 256 {
                                self.complete = true;
                            } else {
                                return Ok(None);
                            }
                        }
                    }
                }
            }
            if self.source_work >= 256 {
                self.complete = true;
            }
        }
        if let Some(id) = self.image.filter(|_| self.phase >= 2) {
            Ok(Some(format!(
                "image #{} alt={}\ntarget={} [bounded; size unavailable]",
                id.0,
                if self.alt_text.is_empty() {
                    if self.alt_empty {
                        "[empty]"
                    } else {
                        "[unavailable]"
                    }
                } else {
                    &self.alt_text
                },
                if self.target_text.is_empty() {
                    "[unavailable]"
                } else {
                    &self.target_text
                }
            )))
        } else {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            Ok(Some(
                "media details unavailable in first 256 source bytes".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_metadata_phases_share_one_lifetime_byte_cap() {
        let source = format!(
            "![{}](https://example.test/{}) SECRET_PROSE\n",
            "x".repeat(512),
            "y".repeat(512)
        );
        let mut d = Document::new();
        d.append(&source).unwrap();
        d.finish();
        assert_eq!(d.links().len(), 1, "fixture requires a real typed image");
        let links = SharedLinks::new(&d);
        let mut task = MediaDetails::new(&d, Span::new(0, source.len())).unwrap();
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(task.advance(&d, &links, &mut zero).unwrap(), None);
        assert_eq!(zero.used(), WorkLimit::default());
        let mut charged = 0;
        let mut result = None;
        for _ in 0..100_000 {
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            result = task.advance(&d, &links, &mut b).unwrap();
            charged += b.used().bytes;
            assert!(charged <= 256);
            if result.is_some() {
                break;
            }
            assert_ne!(b.used(), WorkLimit::default());
        }
        let result = result.unwrap();
        eprintln!("metadata result={result:?} charged={charged}");
        assert!(result.contains("image #0 alt="));
        assert!(!result.contains("SECRET_PROSE"));
        assert!(result.len() <= 128);
    }
    #[test]
    fn an_unrecognized_giant_literal_does_not_invent_an_image_type() {
        let source = format!(
            "![{}](https://example.test/{}) SECRET_PROSE\n",
            "x".repeat(4096),
            "y".repeat(4096)
        );
        let mut d = Document::new();
        d.append(&source).unwrap();
        d.finish();
        assert!(d.links().is_empty());
        let links = SharedLinks::new(&d);
        let mut task = MediaDetails::new(&d, Span::new(0, source.len())).unwrap();
        let result = task
            .advance(&d, &links, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap();
        assert_eq!(
            result,
            "media details unavailable in first 256 source bytes"
        );
    }

    #[test]
    fn an_entity_at_the_lifetime_frontier_finishes_with_explicit_partial_details() {
        let source = format!(
            "![{}]({})\n",
            "x".repeat(128),
            "&CounterClockwiseContourIntegral;".repeat(16)
        );
        let mut d = Document::new();
        d.append(&source).unwrap();
        d.finish();
        assert_eq!(d.links().len(), 1);
        let links = SharedLinks::new(&d);
        let mut task = MediaDetails::new(&d, Span::new(0, source.len())).unwrap();
        let mut bytes = 0;
        let mut result = None;
        for _ in 0..100_000 {
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            result = task.advance(&d, &links, &mut b).unwrap();
            bytes += b.used().bytes;
            assert!(bytes <= 256);
            if result.is_some() {
                break;
            }
            assert_ne!(b.used(), WorkLimit::default());
        }
        assert!(result.unwrap().contains("[bounded; size unavailable]"));
    }
}
