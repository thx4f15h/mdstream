use crate::segtables::{
    self,
    grapheme::{grapheme_category, GraphemeCat, GraphemeCat::*},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prop {
    cat: GraphemeCat,
    linker: bool,
    inc_extend: bool,
}
impl Prop {
    pub fn of(c: char) -> Self {
        Self {
            cat: grapheme_category(c).2,
            linker: segtables::is_incb_linker(c),
            inc_extend: segtables::derived_property::InCB_Extend(c),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    prev: Option<GraphemeCat>,
    ri: bool,
    ep: u8,
    inc: u8,
}
impl State {
    pub const START: Self = Self {
        prev: None,
        ri: false,
        ep: 0,
        inc: 0,
    };
    pub fn step(self, p: Prop) -> (Self, bool) {
        let boundary = match self.prev {
            None => true,
            Some(a) => match (a, p.cat) {
                (GC_CR, GC_LF) => false,
                (GC_Control | GC_CR | GC_LF, _) | (_, GC_Control | GC_CR | GC_LF) => true,
                (GC_L, GC_L | GC_V | GC_LV | GC_LVT)
                | (GC_LV | GC_V, GC_V | GC_T)
                | (GC_LVT | GC_T, GC_T) => false,
                (_, GC_Extend | GC_ZWJ | GC_SpacingMark) | (GC_Prepend, _) => false,
                (_, GC_InCB_Consonant) if self.inc == 2 => false,
                (GC_ZWJ, GC_Extended_Pictographic) if self.ep == 2 => false,
                (GC_Regional_Indicator, GC_Regional_Indicator) if self.ri => false,
                _ => true,
            },
        };
        let ri = p.cat == GC_Regional_Indicator && !self.ri;
        let ep = match p.cat {
            GC_Extended_Pictographic => 1,
            GC_Extend if self.ep == 1 => 1,
            GC_ZWJ if self.ep == 1 => 2,
            _ => 0,
        };
        let inc = if p.cat == GC_InCB_Consonant {
            1
        } else if p.linker && self.inc != 0 {
            2
        } else if p.inc_extend {
            self.inc
        } else {
            0
        };
        (
            Self {
                prev: Some(p.cat),
                ri,
                ep,
                inc,
            },
            boundary,
        )
    }
}
