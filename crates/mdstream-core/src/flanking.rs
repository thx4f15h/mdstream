use crate::Work;

#[path = "unicode_categories.rs"]
mod data;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Other,
    White,
    Punct,
}

/// CommonMark 0.31.2: Zs + TAB/LF/FF/CR, and P/S. This deliberately differs
/// from Rust White_Space (NEL, VT, Zl and Zp are ordinary here).
pub(super) fn class(ch: Option<char>, work: &mut Work) -> Class {
    work.flanking_steps += 1;
    let Some(ch) = ch else {
        return Class::White;
    };
    if ch.is_ascii() {
        return if ch.is_ascii_whitespace() {
            Class::White
        } else if ch.is_ascii_punctuation() {
            Class::Punct
        } else {
            Class::Other
        };
    }
    match data::RANGES.binary_search_by(|&(start, end, _)| {
        work.flanking_steps += 1;
        if end < ch {
            std::cmp::Ordering::Less
        } else if start > ch {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => data::RANGES[index].2,
        Err(_) => Class::Other,
    }
}

#[cfg(test)]
#[path = "../tests/support/unicode_categories.rs"]
mod model;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unicode_scalar_matches_official_categories_with_bounded_work() {
        let expected = model::classes();
        assert!(data::RANGES.len() < 1024);
        assert!(data::RANGES.windows(2).all(|p| p[0].1 < p[1].0));
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            let mut work = Work::default();
            assert_eq!(
                class(Some(ch), &mut work) as u8,
                expected[ch as usize],
                "{ch:?}"
            );
            assert!(work.flanking_steps <= 12, "{ch:?}: {work:?}");
        }
        assert_eq!(class(None, &mut Work::default()), Class::White);
    }
}
