use super::Effects;
use crate::{Span, Work, entity};

/// A single recovery capability over actual raw consumption, never an envelope
/// or a clone of grammar state. Success and failure both drop the allocation.
#[derive(Default)]
pub(super) struct AtomTrace {
    raw: Vec<Span>,
}

impl AtomTrace {
    pub fn push(&mut self, pos: usize, work: &mut Work) {
        work.link_fallback_steps += 1;
        if let Some(last) = self.raw.last_mut() {
            debug_assert!(last.end <= pos);
            if last.end == pos {
                last.end += 1;
                return;
            }
        }
        self.raw.push(Span::new(pos, pos + 1));
    }

    pub fn recover(self, effects: &mut Effects<'_>) {
        if self.scan(effects, false) {
            self.scan(effects, true);
        }
    }

    fn scan(&self, effects: &mut Effects<'_>, publish: bool) -> bool {
        enum Atom {
            Plain,
            Escape(usize),
            Entity { start: usize, len: usize },
        }
        let mut atom = Atom::Plain;
        let mut body = [0; entity::MAX_ENTITY_BODY];
        for span in &self.raw {
            effects.work.link_fallback_steps += 1;
            for pos in span.range() {
                effects.work.link_fallback_bytes += 1;
                let byte = effects.source.as_bytes()[pos];
                match atom {
                    Atom::Escape(start) => {
                        if matches!(byte, b'\r' | b'\n') {
                            return false;
                        }
                        atom = Atom::Plain;
                        if byte.is_ascii_punctuation() {
                            if publish {
                                effects.hide(Span::new(start, start + 1));
                            }
                            continue;
                        }
                    }
                    Atom::Entity { start, len } => {
                        if byte == b';' {
                            if publish {
                                effects.work.entity_bytes += len as u64;
                                // The buffer contains only admitted ASCII bytes.
                                let text = std::str::from_utf8(&body[..len]).unwrap();
                                if let Some(text) = entity::decode(text) {
                                    effects.entity(Span::new(start, pos + 1), text);
                                }
                            }
                            atom = Atom::Plain;
                            continue;
                        }
                        if len < body.len()
                            && (byte.is_ascii_alphanumeric() || (byte == b'#' && len == 0))
                        {
                            body[len] = byte;
                            atom = Atom::Entity {
                                start,
                                len: len + 1,
                            };
                            continue;
                        }
                        // Only the rejecting byte is reconsidered, in place.
                    }
                    Atom::Plain => {}
                }
                atom = match byte {
                    b'<' | b'[' | b']' | b'!' | b'*' | b'_' | b'~' | b'`' | b'$' => {
                        return false;
                    }
                    b'\\' => Atom::Escape(pos),
                    b'&' => Atom::Entity { start: pos, len: 0 },
                    _ => Atom::Plain,
                };
            }
        }
        matches!(atom, Atom::Plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use crate::inline::{Eligibility, Tail};

    #[test]
    fn trace_rle_capacity_lifetime_and_observer_have_no_history() {
        let mut d = Document::new();
        d.append(&format!("{} [a](", "p".repeat(1025))).unwrap();
        let start = d.source().len();
        let Some(Tail::Destination {
            atoms: Some(atoms), ..
        }) = &d.inline.tail
        else {
            panic!("live destination");
        };
        assert_eq!(atoms.raw.capacity(), 0);
        for _ in 0..1000 {
            d.append("x").unwrap();
        }
        let Some(Tail::Destination {
            atoms: Some(atoms), ..
        }) = &d.inline.tail
        else {
            panic!("live destination");
        };
        assert_eq!(atoms.raw, [Span::new(start, start + 1000)]);
        assert!(atoms.raw.capacity() <= 4);
        d.append(") [b](&amp;").unwrap();
        let Some(Tail::Destination {
            atoms: Some(atoms), ..
        }) = &d.inline.tail
        else {
            panic!("new trace");
        };
        assert_eq!(atoms.raw, [Span::new(start + 1006, start + 1011)]);
        d.finish();
        assert!(d.inline.tail.is_none());
        let work = d.work();
        d.finish();
        assert_eq!(d.work(), work);
        let source = "[a](&amp; x)";
        let mut observer = Eligibility::default();
        let mut work = Work::default();
        // The ordinary eligibility backend can never hold a publishing trace.
        observer.feed(source, Span::new(0, 9), 0, &mut work);
        let Some(Tail::Destination { atoms: None, .. }) = &observer.inline.tail else {
            panic!("observer has no trace");
        };
        observer.finish(source, &mut work);
        assert_eq!(work.link_fallback_bytes, 0);
        assert_eq!(work.link_fallback_steps, 0);
        let mut d = Document::new();
        for _ in 0..1000 {
            d.append("[a](&amp; x) [b](u) [c](u '*&amp;\n\n").unwrap();
            assert!(d.inline.tail.is_none());
            assert!(d.direct_title_gaps.iter().all(Vec::is_empty));
        }
        d.finish();
    }

    #[test]
    fn physical_prefix_spans_and_virtual_tab_are_never_raw() {
        let mut d = Document::new();
        d.append("> [a](u 'x\n>\t&amp;\n").unwrap();
        let Some(Tail::Destination {
            atoms: Some(atoms), ..
        }) = &d.inline.tail
        else {
            panic!("live title");
        };
        assert_eq!(atoms.raw, [Span::new(6, 11), Span::new(13, 19)]);
        let data: String = atoms.raw.iter().map(|s| &d.source()[s.range()]).collect();
        assert_eq!(data, "u 'x\n&amp;\n");
        assert!(atoms.raw.capacity() <= 2 * crate::MAX_LINK_SUFFIX_BYTES);
        d.finish();
        assert_eq!(d.work().link_fallback_bytes, 2 * data.len() as u64);
    }

    #[test]
    fn recovering_atoms_changes_effects_without_changing_any_live_grammar() {
        for source in [
            "[a](u '&amp;'x) [b](v)",
            "[a](&amp;",
            "[a](\\*",
            "[a](u '&amp;`x`",
        ] {
            let mut d = Document::new();
            d.source = source.into();
            let mut inline = super::super::Inline::default();
            let mut observer = super::super::Inline::default();
            let mut work = Work::default();
            let mut update = crate::Update::default();
            for pos in 0..source.len() {
                inline.feed(Span::new(pos, pos + 1), &mut d.effects(&mut update));
                observer.feed(
                    Span::new(pos, pos + 1),
                    &mut Effects::observing(source, &mut work),
                );
                super::super::tests::same(&inline, &observer, "atomic recovery byte");
            }
            inline.finish(&mut d.effects(&mut update));
            observer.finish(&mut Effects::observing(source, &mut work));
            super::super::tests::same(&inline, &observer, "atomic recovery finish");
            assert_eq!(work.link_fallback_steps, 0);
        }
    }
}
