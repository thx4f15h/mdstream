use std::fmt::Write;

use crate::screen::StyleRole;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ITALIC: &str = "\x1b[3m";
const UNDERLINE_SINGLE: &str = "\x1b[4m";
const UNDERLINE_DOUBLE: &str = "\x1b[4:2m";
const UNDERLINE_CURLY: &str = "\x1b[4:3m";
const UNDERLINE_DOTTED: &str = "\x1b[4:4m";
const UNDERLINE_DASHED: &str = "\x1b[4:5m";
const STRIKE: &str = "\x1b[9m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl Underline {
    const fn sequence(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Single => Some(UNDERLINE_SINGLE),
            Self::Double => Some(UNDERLINE_DOUBLE),
            Self::Curly => Some(UNDERLINE_CURLY),
            Self::Dotted => Some(UNDERLINE_DOTTED),
            Self::Dashed => Some(UNDERLINE_DASHED),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolvedStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strikethrough: bool,
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
}

impl ResolvedStyle {
    #[must_use]
    pub fn is_plain(&self) -> bool {
        !self.bold
            && !self.italic
            && self.underline == Underline::None
            && !self.strikethrough
            && self.foreground.is_none()
            && self.background.is_none()
    }

    #[must_use]
    pub fn start_sequence(&self) -> String {
        let mut output = String::new();
        if self.bold {
            output.push_str(BOLD);
        }
        if self.italic {
            output.push_str(ITALIC);
        }
        if let Some(underline) = self.underline.sequence() {
            output.push_str(underline);
        }
        if self.strikethrough {
            output.push_str(STRIKE);
        }
        if let Some(foreground) = self.foreground {
            write!(
                output,
                "\x1b[38;2;{};{};{}m",
                foreground.red, foreground.green, foreground.blue
            )
            .expect("writing to a String cannot fail");
        }
        if let Some(background) = self.background {
            write!(
                output,
                "\x1b[48;2;{};{};{}m",
                background.red, background.green, background.blue
            )
            .expect("writing to a String cannot fail");
        }
        output
    }

    #[must_use]
    pub fn encode_segment(&self, text: &str) -> String {
        if self.is_plain() {
            return text.to_owned();
        }
        format!("{}{text}{RESET}", self.start_sequence())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub heading: Rgb,
    pub strong: Rgb,
    pub emphasis: Rgb,
    pub code_foreground: Rgb,
    pub code_background: Rgb,
    pub link: Rgb,
    pub quote: Rgb,
    pub list_marker: Rgb,
    pub rule: Rgb,
    pub table_border: Rgb,
    pub strike: Rgb,
    pub meta: Rgb,
}

pub const MONOKAI: Theme = Theme {
    heading: Rgb::new(249, 38, 114),
    strong: Rgb::new(255, 121, 198),
    emphasis: Rgb::new(255, 121, 198),
    code_foreground: Rgb::new(166, 226, 46),
    code_background: Rgb::new(40, 40, 40),
    link: Rgb::new(139, 233, 253),
    quote: Rgb::new(117, 113, 94),
    list_marker: Rgb::new(255, 121, 198),
    rule: Rgb::new(117, 113, 94),
    table_border: Rgb::new(117, 113, 94),
    strike: Rgb::new(117, 113, 94),
    meta: Rgb::new(117, 113, 94),
};

impl Theme {
    #[must_use]
    pub fn resolve(&self, role: StyleRole) -> ResolvedStyle {
        let heading_level = role.heading_level();
        if role.contains(StyleRole::HEADING) {
            let level = heading_level.unwrap_or(1);
            return ResolvedStyle {
                bold: true,
                underline: match level {
                    1 => Underline::Single,
                    2 => Underline::Double,
                    3 => Underline::Curly,
                    4 => Underline::Dotted,
                    5 => Underline::Dashed,
                    _ => Underline::None,
                },
                foreground: Some(self.heading),
                ..ResolvedStyle::default()
            };
        }

        let bold = role.contains(StyleRole::STRONG);
        let italic = role.contains(StyleRole::EMPHASIS);
        let strikethrough = role.contains(StyleRole::STRIKE);
        let underline = if role.contains(StyleRole::LINK) {
            Underline::Single
        } else {
            Underline::None
        };

        let (foreground, background) = if role.contains(StyleRole::CODE) {
            (Some(self.code_foreground), Some(self.code_background))
        } else if role.contains(StyleRole::LINK) {
            (Some(self.link), None)
        } else if role.contains(StyleRole::STRONG) || role.contains(StyleRole::EMPHASIS) {
            (Some(self.strong), None)
        } else if strikethrough {
            (Some(self.strike), None)
        } else if role.contains(StyleRole::LIST_MARKER) {
            (Some(self.list_marker), None)
        } else if role.contains(StyleRole::RULE) {
            (Some(self.rule), None)
        } else if role.contains(StyleRole::TABLE_BORDER) {
            (Some(self.table_border), None)
        } else if role.contains(StyleRole::META) {
            (Some(self.meta), None)
        } else {
            (None, None)
        };

        ResolvedStyle {
            bold,
            italic,
            underline,
            strikethrough,
            foreground,
            background,
        }
    }

    #[must_use]
    pub fn encode_segment(&self, role: StyleRole, text: &str) -> String {
        self.resolve(role).encode_segment(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(role: StyleRole, text: &str) -> String {
        MONOKAI.encode_segment(role, text)
    }

    #[test]
    fn theme_constants_match_reference() {
        assert_eq!(MONOKAI.heading, Rgb::new(249, 38, 114));
        assert_eq!(MONOKAI.strong, Rgb::new(255, 121, 198));
        assert_eq!(MONOKAI.emphasis, Rgb::new(255, 121, 198));
        assert_eq!(MONOKAI.code_foreground, Rgb::new(166, 226, 46));
        assert_eq!(MONOKAI.code_background, Rgb::new(40, 40, 40));
        assert_eq!(MONOKAI.link, Rgb::new(139, 233, 253));
        assert_eq!(MONOKAI.quote, Rgb::new(117, 113, 94));
        assert_eq!(MONOKAI.list_marker, Rgb::new(255, 121, 198));
        assert_eq!(MONOKAI.strike, Rgb::new(117, 113, 94));
        assert_eq!(MONOKAI.meta, Rgb::new(117, 113, 94));
    }

    #[test]
    fn headings_h1_to_h6_emit_exact_sequences() {
        let cases = [
            (1, "\x1b[1m\x1b[4m\x1b[38;2;249;38;114mTitle\x1b[0m"),
            (2, "\x1b[1m\x1b[4:2m\x1b[38;2;249;38;114mTitle\x1b[0m"),
            (3, "\x1b[1m\x1b[4:3m\x1b[38;2;249;38;114mTitle\x1b[0m"),
            (4, "\x1b[1m\x1b[4:4m\x1b[38;2;249;38;114mTitle\x1b[0m"),
            (5, "\x1b[1m\x1b[4:5m\x1b[38;2;249;38;114mTitle\x1b[0m"),
            (6, "\x1b[1m\x1b[38;2;249;38;114mTitle\x1b[0m"),
        ];
        for (level, expected) in cases {
            assert_eq!(encode(StyleRole::heading(level), "Title"), expected);
        }
    }

    #[test]
    fn inline_roles_emit_exact_sequences() {
        assert_eq!(
            encode(StyleRole::STRONG, "bold"),
            "\x1b[1m\x1b[38;2;255;121;198mbold\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::EMPHASIS, "italic"),
            "\x1b[3m\x1b[38;2;255;121;198mitalic\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::STRIKE, "strike"),
            "\x1b[9m\x1b[38;2;117;113;94mstrike\x1b[0m"
        );
    }

    #[test]
    fn code_and_link_emit_exact_sequences() {
        assert_eq!(
            encode(StyleRole::CODE, "code"),
            "\x1b[38;2;166;226;46m\x1b[48;2;40;40;40mcode\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::LINK, "link"),
            "\x1b[4m\x1b[38;2;139;233;253mlink\x1b[0m"
        );
    }

    #[test]
    fn code_keeps_outer_flags_in_sgr_order() {
        let role = StyleRole::CODE
            .with(StyleRole::STRONG)
            .with(StyleRole::EMPHASIS)
            .with(StyleRole::STRIKE);
        assert_eq!(
            encode(role, "code"),
            "\x1b[1m\x1b[3m\x1b[9m\x1b[38;2;166;226;46m\x1b[48;2;40;40;40mcode\x1b[0m"
        );
    }

    #[test]
    fn structural_roles_emit_expected_colors_and_quote_stays_plain() {
        assert_eq!(
            encode(StyleRole::LIST_MARKER, "•"),
            "\x1b[38;2;255;121;198m•\x1b[0m"
        );
        for role in [StyleRole::RULE, StyleRole::TABLE_BORDER, StyleRole::META] {
            assert_eq!(encode(role, "─"), "\x1b[38;2;117;113;94m─\x1b[0m");
        }
        assert_eq!(encode(StyleRole::QUOTE, "quote"), "quote");
    }

    #[test]
    fn default_text_is_raw() {
        assert_eq!(encode(StyleRole::DEFAULT, "plain text"), "plain text");
    }

    #[test]
    fn adjacent_segments_reset_independently() {
        let output = format!(
            "{}{}{}",
            encode(StyleRole::STRONG, "a"),
            encode(StyleRole::CODE, "b"),
            encode(StyleRole::DEFAULT, "c")
        );
        assert_eq!(
            output,
            "\x1b[1m\x1b[38;2;255;121;198ma\x1b[0m\x1b[38;2;166;226;46m\x1b[48;2;40;40;40mb\x1b[0mc"
        );
        assert_eq!(output.matches(RESET).count(), 2);
    }

    #[test]
    fn forbidden_sgr_sequences_are_absent() {
        let roles = [
            StyleRole::heading(1),
            StyleRole::heading(2),
            StyleRole::heading(3),
            StyleRole::heading(4),
            StyleRole::heading(5),
            StyleRole::heading(6),
            StyleRole::STRONG,
            StyleRole::EMPHASIS,
            StyleRole::STRIKE,
            StyleRole::CODE,
            StyleRole::LINK,
            StyleRole::LIST_MARKER,
            StyleRole::RULE,
            StyleRole::TABLE_BORDER,
            StyleRole::META,
        ];
        for role in roles {
            let output = encode(role, "x");
            for forbidden in [
                "\x1b[39m",
                "\x1b[49m",
                "\x1b[2m",
                "\x1b[7m",
                "\x1b[38;5;",
                "\x1b[48;5;",
                "\x1b]8",
            ] {
                assert!(
                    !output.contains(forbidden),
                    "role {role:?} emitted forbidden sequence {forbidden:?}: {output:?}"
                );
            }
        }
    }
}
