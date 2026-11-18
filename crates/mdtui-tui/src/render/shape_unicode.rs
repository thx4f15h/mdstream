//! Fixed, composable Unicode machines. Classification never traverses Unicode
//! at runtime. GCB17 and non-CJK width18 deliberately have different versions.
#[path = "shape_tables.rs"]
mod tables;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Properties {
    pub gcb: u8,
    pub width: u16,
}
impl Properties {
    pub fn of(ch: char) -> Self {
        let code = ch as u32;
        let at = tables::PROPS.partition_point(|&(_, end, _, _)| end < code);
        let (_, _, gcb, width) = tables::PROPS[at];
        Self { gcb, width }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GcbState(pub u8);
impl GcbState {
    pub fn push(&mut self, properties: Properties) -> bool {
        let value = tables::GCB[usize::from(self.0)][usize::from(properties.gcb)];
        self.0 = value & 127;
        value & 128 != 0
    }
}

/// Reverse width fold as a forward append transducer: W(xc)=W(x)∘W(c).
/// Every entry is maintained on input, while range queries read one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WidthSummary {
    pub out: [u8; 132],
    pub delta: [i64; 132],
}
impl Default for WidthSummary {
    fn default() -> Self {
        Self {
            out: std::array::from_fn(|i| i as u8),
            delta: [0; 132],
        }
    }
}
impl WidthSummary {
    pub fn push(&mut self, properties: Properties) {
        let transitions = &tables::WIDTH[usize::from(properties.width)];
        let old = self.clone();
        for (i, &transition) in transitions.iter().enumerate() {
            let state = usize::from(transition & 255);
            self.out[i] = old.out[state];
            self.delta[i] = old.delta[state] + i64::from(transition >> 8) - 8;
        }
    }
    pub fn width(&self) -> usize {
        self.delta[0].max(0) as usize
    }
}

/// Sequential per-EGC measurement, retaining fixed metadata instead of an
/// unfinished String. Used by table measurement of already-paid display runs.
#[derive(Debug, Default)]
pub(crate) struct WidthStream {
    state: GcbState,
    pending: WidthSummary,
    completed: usize,
    pending_scalars: usize,
    first: Option<char>,
    controls: bool,
}
impl WidthStream {
    pub fn push(&mut self, text: &str) {
        for ch in text.chars() {
            let properties = Properties::of(ch);
            if self.state.push(properties) && self.pending_scalars != 0 {
                self.completed += self.pending_width();
                self.pending = WidthSummary::default();
                self.pending_scalars = 0;
            }
            if self.pending_scalars == 0 {
                self.first = Some(ch);
                self.controls = true;
            }
            self.controls &= ch.is_control();
            self.pending_scalars += 1;
            self.pending.push(properties);
        }
    }
    pub(crate) fn pending_width(&self) -> usize {
        if self.pending_scalars == 0 {
            return 0;
        }
        if self.pending_scalars == 1 && self.first == Some('\t') {
            return 1;
        }
        if self.controls {
            return 0;
        }
        if self.pending_scalars == 1 {
            match self.first.unwrap() {
                '•' | '│' | '─' | '═' | '┌' | '┬' | '┐' | '└' | '┴' | '┘' | '╞' | '╪' | '╡'
                | ' ' | '\u{a0}' | 'é' => return 1,
                '⬜' | '✅' => return 2,
                _ => {}
            }
        }
        self.pending.width().min(2)
    }
    pub(crate) fn pending_kind(&self) -> super::text::GraphemeKind {
        use super::text::GraphemeKind;
        if self.pending.width() > 2 {
            return GraphemeKind::Oversize;
        }
        match self.pending_width() {
            0 => GraphemeKind::ZeroWidth,
            1 => GraphemeKind::Narrow,
            _ => GraphemeKind::Wide,
        }
    }
    pub fn width(&self) -> usize {
        self.completed + self.pending_width()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;
    #[test]
    fn long_width_transducer_matches_oracle() {
        for s in [
            "👩‍💻".to_owned(),
            format!("👩{}‍💻", "\u{301}".repeat(10_000)),
            "a\u{301}".to_owned(),
        ] {
            let mut width = WidthStream::default();
            width.push(&s);
            assert_eq!(
                width.width(),
                UnicodeWidthStr::width(s.as_str()).min(2),
                "chars {}",
                s.chars().count()
            );
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GcbSummary {
    pub out: [u8; 24],
    pub breaks: [usize; 24],
}
impl Default for GcbSummary {
    fn default() -> Self {
        Self {
            out: std::array::from_fn(|i| i as u8),
            breaks: [0; 24],
        }
    }
}
impl GcbSummary {
    pub fn push(&mut self, properties: Properties) {
        for i in 0..24 {
            let next = tables::GCB[usize::from(self.out[i])][usize::from(properties.gcb)];
            self.out[i] = next & 127;
            self.breaks[i] += usize::from(next & 128 != 0);
        }
    }
    pub fn join(left: &Self, right: &Self) -> Self {
        Self {
            out: std::array::from_fn(|i| right.out[usize::from(left.out[i])]),
            breaks: std::array::from_fn(|i| {
                left.breaks[i] + right.breaks[usize::from(left.out[i])]
            }),
        }
    }
}
impl WidthSummary {
    pub fn join(left: &Self, right: &Self) -> Self {
        Self {
            out: std::array::from_fn(|i| left.out[usize::from(right.out[i])]),
            delta: std::array::from_fn(|i| right.delta[i] + left.delta[usize::from(right.out[i])]),
        }
    }
    pub fn entry(properties: Properties, input: u8) -> (u8, i64) {
        let value = tables::WIDTH[usize::from(properties.width)][usize::from(input)];
        ((value & 255) as u8, i64::from(value >> 8) - 8)
    }
}

#[cfg(test)]
mod independent_oracles {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    #[test]
    fn official_gcb17_all_766_cases() {
        let mut cases = 0;
        for line in include_str!("../../../../data/unicode/17.0.0/GraphemeBreakTest.txt").lines() {
            let line = line.split('#').next().unwrap();
            if line.trim().is_empty() {
                continue;
            }
            let mut text = String::new();
            let mut expected = Vec::new();
            for token in line.split_whitespace() {
                match token {
                    "÷" => expected.push(text.len()),
                    "×" => {}
                    hex => {
                        text.push(char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap())
                    }
                }
            }
            let mut state = GcbState::default();
            let mut actual = Vec::new();
            for (offset, ch) in text.char_indices() {
                if state.push(Properties::of(ch)) {
                    actual.push(offset)
                }
            }
            actual.push(text.len());
            assert_eq!(actual, expected, "official case {cases}");
            cases += 1;
        }
        assert_eq!(cases, 766);
    }
    #[test]
    fn raw_width_and_per_cluster_policy_have_independent_oracles() {
        let texts = [
            "a\u{301} b\t中\0",
            "👩‍💻x❤️‍🔥",
            "\r\n",
            "א\u{200d}ל",
            "لا",
            "क्\u{93c}क",
            "\u{17d8}x",
            "✅⬜•é",
            "🇦🇧🇨🇩🇪",
        ];
        for text in texts {
            let mut raw = WidthSummary::default();
            for ch in text.chars() {
                raw.push(Properties::of(ch));
            }
            assert_eq!(raw.width(), UnicodeWidthStr::width(text), "raw {text:?}");
            let mut stream = WidthStream::default();
            stream.push(text);
            assert_eq!(
                stream.width(),
                crate::render::text::display_width(text),
                "per GCB {text:?}"
            );
        }
        let mut seen = vec![false; tables::WIDTH.len()];
        let mut pool = Vec::new();
        for &(start, _, _, width) in tables::PROPS {
            if !seen[usize::from(width)] {
                seen[usize::from(width)] = true;
                pool.push(char::from_u32(start).unwrap());
            }
        }
        pool.extend("a\u{301}👩‍💻🇦🇧क्\u{93c}❤️\r\nلاא\u{200d}ל".chars());
        let mut random = 0x0210_18ab_cdef_u64;
        for case in 0..2000 {
            let mut text = String::new();
            let mut raw = WidthSummary::default();
            let mut gcb = GcbState::default();
            let mut actual = Vec::new();
            for _ in 0..64 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let ch = pool[random as usize % pool.len()];
                let properties = Properties::of(ch);
                if gcb.push(properties) {
                    actual.push(text.len())
                }
                text.push(ch);
                raw.push(properties);
                assert_eq!(
                    raw.width(),
                    UnicodeWidthStr::width(text.as_str()),
                    "raw random {case}"
                );
            }
            actual.push(text.len());
            let expected: Vec<_> = text
                .grapheme_indices(true)
                .map(|(at, _)| at)
                .chain(std::iter::once(text.len()))
                .collect();
            assert_eq!(actual, expected, "GCB random {case}");
            let mut stream = WidthStream::default();
            stream.push(&text);
            assert_eq!(
                stream.width(),
                crate::render::text::display_width(&text),
                "per GCB random {case}"
            );
        }
    }
}
