//! Original hand-authored witnesses, checked against the locked external
//! cmark-gfm 0.29.0.gfm.13 --extension strikethrough (MIT/BSD, upstream COPYING).
//! HTML is independent reference evidence, not a Cargo/runtime tool dependency.
//! Formal expectations follow the pre-existing README's exact ASCII rule:
//! non-ASCII-whitespace before a strike run allows closing, close wins, and
//! intervening unmatched openers are discarded. This includes punctuation.
use mdstream::{InlineStyleKind, Span, StylePairId};

pub struct Witness {
    pub source: &'static str,
    pub cmark_html: &'static str,
    pub formal_text: &'static str,
    pub formal_pairs: &'static [StylePairId],
}
const fn strike(open: usize, close: usize) -> StylePairId {
    StylePairId {
        open: Span::new(open, open + 1),
        close: Span::new(close, close + 1),
        kind: InlineStyleKind::Strike,
    }
}
pub const CASES: &[Witness] = &[
    Witness {
        source: "p ~a *~x~* z~ q\n",
        cmark_html: "<p>p <del>a <em><del>x</del></em> z</del> q</p>\n",
        formal_text: "p a *x* z q\n",
        formal_pairs: &[strike(2, 6), strike(8, 12)],
    },
    Witness {
        source: "p ~a _[~x~][r]_ z~ q\n",
        cmark_html: "<p>p <del>a <em>[<del>x</del>][r]</em> z</del> q</p>\n",
        formal_text: "p a _[x][r]_ z q\n",
        formal_pairs: &[strike(2, 7), strike(9, 17)],
    },
    Witness {
        source: "p ~a __[~x~][r]__ z~ q\n",
        cmark_html: "<p>p <del>a <strong>[<del>x</del>][r]</strong> z</del> q</p>\n",
        formal_text: "p a __[x][r]__ z q\n",
        formal_pairs: &[strike(2, 8), strike(10, 19)],
    },
    Witness {
        source: "p ~a __~x__ z~ q\n",
        cmark_html: "<p>p <del>a <strong>~x</strong> z</del> q</p>\n",
        formal_text: "p a __x__ z~ q\n",
        formal_pairs: &[strike(2, 7)],
    },
];
