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
    pub link_focus_background: Rgb,
    pub sidebar_selection_background: Rgb,
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
    link_focus_background: Rgb::new(73, 72, 62),
    sidebar_selection_background: Rgb::new(73, 72, 62),
};

impl Theme {
    #[must_use]
    pub fn resolve(&self, role: StyleRole) -> ResolvedStyle {
        let mut style = self.resolve_base(role);
        if role.contains(StyleRole::SEARCH) {
            style.background = Some(Rgb::new(100, 75, 0));
        }
        if role.contains(StyleRole::SELECTION) {
            style.background = Some(Rgb::new(35, 65, 110));
        }
        if role.contains(StyleRole::CURSOR) {
            style.background = Some(Rgb::new(100, 50, 110));
            style.bold = true;
        }
        style
    }

    fn resolve_base(&self, role: StyleRole) -> ResolvedStyle {
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

        let link_focus = role.contains(StyleRole::LINK_FOCUS);
        let sidebar_selection = role.contains(StyleRole::SIDEBAR_SELECTION);
        let bold = role.contains(StyleRole::STRONG)
            || role.contains(StyleRole::SIDEBAR_CURRENT)
            || link_focus;
        let italic = role.contains(StyleRole::EMPHASIS);
        let strikethrough = role.contains(StyleRole::STRIKE);
        let underline = if role.contains(StyleRole::LINK) {
            Underline::Single
        } else {
            Underline::None
        };

        let (foreground, background) = if role.contains(StyleRole::CODE) {
            (Some(self.code_foreground), Some(self.code_background))
        } else if role.contains(StyleRole::TASK_CHECKED) {
            return ResolvedStyle {
                foreground: Some(self.code_foreground),
                ..ResolvedStyle::default()
            };
        } else if role.contains(StyleRole::LINK) || link_focus {
            (Some(self.link), None)
        } else if role.contains(StyleRole::MATH) {
            (None, None)
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
        } else if role.contains(StyleRole::SIDEBAR_CURRENT) {
            (Some(self.strong), None)
        } else if sidebar_selection
            || role.contains(StyleRole::SIDEBAR)
            || role.contains(StyleRole::META)
        {
            (Some(self.meta), None)
        } else {
            (None, None)
        };

        let background = if sidebar_selection {
            Some(self.sidebar_selection_background)
        } else if link_focus {
            Some(self.link_focus_background)
        } else {
            background
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
    use crate::render::input::DisplayInput;
    use mdtui3::{Document, Span};

    fn encode(role: StyleRole, text: &str) -> String {
        MONOKAI.encode_segment(role, text)
    }

    fn markdown_role(source: &str, needle: &str) -> StyleRole {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        let span = Span::new(0, source.len());
        let input = DisplayInput::from_document(&document, span).unwrap();
        let mut roles = input
            .tokens()
            .iter()
            .filter(|token| token.text == needle)
            .map(|token| token.style);
        let role = roles
            .next()
            .unwrap_or_else(|| panic!("missing markdown token {needle:?}"));
        assert!(
            roles.next().is_none(),
            "duplicate markdown token {needle:?}"
        );
        role
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
        assert_eq!(MONOKAI.link_focus_background, Rgb::new(73, 72, 62));
        assert_eq!(MONOKAI.sidebar_selection_background, Rgb::new(73, 72, 62));
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
        assert_eq!(
            encode(StyleRole::LINK.with(StyleRole::LINK_FOCUS), "link"),
            "\x1b[1m\x1b[4m\x1b[38;2;139;233;253m\x1b[48;2;73;72;62mlink\x1b[0m"
        );
    }

    #[test]
    fn task_checked_emits_only_green_foreground_without_code_background() {
        let resolved = MONOKAI.resolve(StyleRole::TASK_CHECKED);
        assert_eq!(resolved.foreground, Some(MONOKAI.code_foreground));
        assert_eq!(resolved.background, None);
        assert!(!resolved.bold);
        assert!(!resolved.italic);
        assert!(!resolved.strikethrough);
        assert_eq!(
            encode(StyleRole::TASK_CHECKED, "✅"),
            "\x1b[38;2;166;226;46m✅\x1b[0m"
        );
        assert!(!encode(StyleRole::TASK_CHECKED, "✅").contains("\x1b[48;2;"));

        let combined = MONOKAI.resolve(
            StyleRole::TASK_CHECKED
                .with(StyleRole::STRONG)
                .with(StyleRole::STRIKE),
        );
        assert_eq!(combined.foreground, Some(MONOKAI.code_foreground));
        assert_eq!(combined.background, None);
        assert!(!combined.bold);
        assert!(!combined.strikethrough);
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
    fn nested_emphasis_roles_and_bytes_are_identical_at_fixed_priority() {
        let outer_strong = markdown_role("**a ~~b~~**", "b");
        let outer_strike = markdown_role("~~a **b**~~", "b");
        assert!(outer_strong.contains(StyleRole::STRONG));
        assert!(outer_strong.contains(StyleRole::STRIKE));
        assert_eq!(outer_strong, outer_strike);

        let bytes = encode(outer_strong, "b");
        assert_eq!(bytes, "\x1b[1m\x1b[9m\x1b[38;2;255;121;198mb\x1b[0m");
        assert_eq!(bytes, encode(outer_strike, "b"));
    }

    #[test]
    fn math_uses_default_foreground_and_keeps_outer_attributes() {
        let strong_math = markdown_role("**$x$**", "x");
        assert!(strong_math.contains(StyleRole::STRONG));
        assert!(strong_math.contains(StyleRole::MATH));
        let resolved = MONOKAI.resolve(strong_math);
        assert!(resolved.bold);
        assert_eq!(resolved.foreground, None);
        assert_eq!(resolved.background, None);
        assert_eq!(encode(strong_math, "x"), "\x1b[1mx\x1b[0m");

        assert_eq!(encode(StyleRole::MATH, "x"), "x");
        assert_eq!(
            encode(StyleRole::MATH.with(StyleRole::EMPHASIS), "x"),
            "\x1b[3mx\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::MATH.with(StyleRole::STRIKE), "x"),
            "\x1b[9mx\x1b[0m"
        );
    }

    #[test]
    fn strong_or_emphasis_with_strike_uses_emphasis_foreground() {
        assert_eq!(
            encode(StyleRole::STRONG.with(StyleRole::STRIKE), "b"),
            "\x1b[1m\x1b[9m\x1b[38;2;255;121;198mb\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::EMPHASIS.with(StyleRole::STRIKE), "b"),
            "\x1b[3m\x1b[9m\x1b[38;2;255;121;198mb\x1b[0m"
        );
    }

    #[test]
    fn code_foreground_wins_over_math_and_keeps_outer_flags() {
        let role = StyleRole::CODE
            .with(StyleRole::MATH)
            .with(StyleRole::STRONG)
            .with(StyleRole::EMPHASIS)
            .with(StyleRole::STRIKE);
        assert_eq!(
            encode(role, "code"),
            "\x1b[1m\x1b[3m\x1b[9m\x1b[38;2;166;226;46m\x1b[48;2;40;40;40mcode\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::CODE.with(StyleRole::STRIKE), "code"),
            "\x1b[9m\x1b[38;2;166;226;46m\x1b[48;2;40;40;40mcode\x1b[0m"
        );
    }

    #[test]
    fn link_foreground_is_not_overridden_by_inline_roles() {
        let role = StyleRole::LINK
            .with(StyleRole::STRONG)
            .with(StyleRole::STRIKE)
            .with(StyleRole::MATH);
        assert_eq!(
            encode(role, "link"),
            "\x1b[1m\x1b[4m\x1b[9m\x1b[38;2;139;233;253mlink\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::LINK.with(StyleRole::MATH), "link"),
            "\x1b[4m\x1b[38;2;139;233;253mlink\x1b[0m"
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
        assert_eq!(
            encode(StyleRole::SIDEBAR, "h"),
            "\x1b[38;2;117;113;94mh\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::SIDEBAR_CURRENT, "h"),
            "\x1b[1m\x1b[38;2;255;121;198mh\x1b[0m"
        );
        assert_eq!(
            encode(StyleRole::SIDEBAR_SELECTION, "h"),
            "\x1b[38;2;117;113;94m\x1b[48;2;73;72;62mh\x1b[0m"
        );
        assert_eq!(
            encode(
                StyleRole::SIDEBAR_CURRENT.with(StyleRole::SIDEBAR_SELECTION),
                "h"
            ),
            "\x1b[1m\x1b[38;2;255;121;198m\x1b[48;2;73;72;62mh\x1b[0m"
        );
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
            StyleRole::TASK_CHECKED,
            StyleRole::LINK,
            StyleRole::LINK.with(StyleRole::LINK_FOCUS),
            StyleRole::MATH,
            StyleRole::CODE
                .with(StyleRole::MATH)
                .with(StyleRole::STRIKE),
            StyleRole::MATH
                .with(StyleRole::STRONG)
                .with(StyleRole::STRIKE),
            StyleRole::LIST_MARKER,
            StyleRole::RULE,
            StyleRole::TABLE_BORDER,
            StyleRole::META,
            StyleRole::SIDEBAR,
            StyleRole::SIDEBAR_CURRENT,
            StyleRole::SIDEBAR_SELECTION,
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
