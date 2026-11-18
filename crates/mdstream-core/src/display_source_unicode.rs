//! Fixed, composable Unicode machines. Classification never traverses Unicode
//! at runtime. GCB17 and non-CJK width18 deliberately have different versions.
// Single canonical generated table shared with the TUI.
#[path = "shape_tables.rs"]
mod tables;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Properties {
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
pub struct GcbState(pub u8);
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
pub struct WidthSummary {
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

#[derive(Debug, Clone)]
pub struct GcbSummary {
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

/// Diagnostic enumeration used by the independent property-class oracles.
pub fn property_ranges() -> &'static [(u32, u32, u8, u16)] {
    tables::PROPS
}
pub fn width_class_count() -> usize {
    tables::WIDTH.len()
}
