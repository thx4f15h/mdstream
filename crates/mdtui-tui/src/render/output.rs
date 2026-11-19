use std::collections::HashMap;
use std::io::{self, Write};

use mdtui3::Document;

use crate::cli::LinkMode;
use crate::render::diff::{DiffRun, ScreenDiff};
use crate::render::link::osc8_destination;
use crate::screen::{Cell, Screen, StyleRole};
use crate::theme::MONOKAI;

/// Per-frame terminal metadata limits, independent of document size. Text and
/// application link identities survive when an OSC8 decoration cannot fit.
pub const MAX_FRAME_DESTINATION_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_FRAME_OSC8_TARGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOptions {
    pub link_mode: LinkMode,
    pub allow_osc8: bool,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            link_mode: LinkMode::Default,
            allow_osc8: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputStats {
    pub runs: usize,
    pub segments: usize,
    pub cells: usize,
    pub continuations: usize,
    pub osc8_links: usize,
    pub bytes: usize,
    pub destination_source_bytes: usize,
    pub oversized_destinations: usize,
    pub deferred_link_segments: usize,
    pub osc8_target_bytes: usize,
}

/// Write the changed content-area cells using absolute one-based cursor moves.
///
/// The writer never clears the screen or homes the cursor. Each diff run is
/// repositioned independently; cells are grouped into self-contained style and
/// link segments. Continuation cells only describe the second half of a wide
/// glyph and are never written as text.
pub fn write_diff<W: Write + ?Sized>(
    writer: &mut W,
    document: &Document,
    screen: &Screen,
    diff: &ScreenDiff,
    options: OutputOptions,
) -> io::Result<OutputStats> {
    validate_diff(screen, diff)?;
    let mut stats = OutputStats::default();
    let mut destinations = HashMap::new();
    for run in &diff.runs {
        let continuations = run
            .cells
            .iter()
            .filter(|cell| cell.is_continuation())
            .count();
        stats.continuations = stats.continuations.saturating_add(continuations);
        if continuations == run.cells.len() {
            continue;
        }

        let cursor = cursor_sequence(run);
        writer.write_all(cursor.as_bytes())?;
        stats.bytes = stats.bytes.saturating_add(cursor.len());
        stats.runs += 1;

        let mut segment: Option<TextSegment> = None;
        for cell in &run.cells {
            if cell.is_continuation() {
                continue;
            }
            stats.cells += 1;
            match segment.as_mut() {
                Some(current)
                    if current.style == cell.style() && current.link_id == cell.link_id() =>
                {
                    append_cell_text(&mut current.text, cell);
                }
                Some(_) => {
                    if let Some(finished) = segment.take() {
                        write_segment(
                            writer,
                            document,
                            &finished,
                            options,
                            &mut stats,
                            &mut destinations,
                        )?;
                    }
                    segment = Some(TextSegment::new(cell));
                }
                None => segment = Some(TextSegment::new(cell)),
            }
        }
        if let Some(finished) = segment {
            write_segment(
                writer,
                document,
                &finished,
                options,
                &mut stats,
                &mut destinations,
            )?;
        }
    }
    Ok(stats)
}

/// Convenience wrapper returning the complete frame bytes and write statistics.
pub fn diff_to_vec(
    document: &Document,
    screen: &Screen,
    diff: &ScreenDiff,
    options: OutputOptions,
) -> io::Result<(Vec<u8>, OutputStats)> {
    let mut bytes = Vec::new();
    let stats = write_diff(&mut bytes, document, screen, diff, options)?;
    Ok((bytes, stats))
}

#[derive(Debug)]
struct TextSegment {
    style: StyleRole,
    link_id: Option<mdtui3::LinkId>,
    text: String,
}

impl TextSegment {
    fn new(cell: &Cell) -> Self {
        let mut text = String::new();
        append_cell_text(&mut text, cell);
        Self {
            style: cell.style(),
            link_id: cell.link_id(),
            text,
        }
    }
}

fn write_segment<W: Write + ?Sized>(
    writer: &mut W,
    document: &Document,
    segment: &TextSegment,
    options: OutputOptions,
    stats: &mut OutputStats,
    destinations: &mut HashMap<mdtui3::LinkId, Option<String>>,
) -> io::Result<()> {
    let (encoded, opened_link) =
        encode_segment(document, None, segment, options, stats, destinations);
    writer.write_all(encoded.as_bytes())?;
    stats.segments += 1;
    stats.bytes = stats.bytes.saturating_add(encoded.len());
    stats.osc8_links += usize::from(opened_link);
    Ok(())
}

fn encode_segment(
    document: &Document,
    body: Option<mdtui3::BodyView<'_>>,
    segment: &TextSegment,
    options: OutputOptions,
    stats: &mut OutputStats,
    destinations: &mut HashMap<mdtui3::LinkId, Option<String>>,
) -> (String, bool) {
    let Some(link_id) = segment.link_id else {
        return (MONOKAI.encode_segment(segment.style, &segment.text), false);
    };

    if options.allow_osc8 && options.link_mode != LinkMode::NoOsc8 {
        let destination = destinations.entry(link_id).or_insert_with(|| {
            let source_len = if let Some(body) = body {
                body.raw_link_destination(body.qualify(link_id))
                    .ok()
                    .flatten()
                    .map_or(0, |r| r.text.len())
            } else {
                document.link_destination(link_id).map_or(0, str::len)
            };
            if source_len <= crate::render::link::MAX_OSC8_DESTINATION_BYTES
                && source_len
                    > MAX_FRAME_DESTINATION_SOURCE_BYTES
                        .saturating_sub(stats.destination_source_bytes)
            {
                stats.deferred_link_segments += 1;
                return None;
            }
            let (target, inspected) = if let Some(body) = body {
                let (target, stats) =
                    crate::render::link::osc8_body_destination(body, body.qualify(link_id));
                (target.map(std::borrow::Cow::Owned), stats)
            } else {
                osc8_destination(document, link_id)
            };
            stats.destination_source_bytes += inspected.source_bytes;
            stats.oversized_destinations += inspected.too_long;
            target.map(|target| target.into_owned())
        });
        if let Some(destination) = destination
            && destination.len()
                <= MAX_FRAME_OSC8_TARGET_BYTES.saturating_sub(stats.osc8_target_bytes)
        {
            stats.osc8_target_bytes += destination.len();
            let styled = MONOKAI.encode_segment(segment.style.with(StyleRole::LINK), &segment.text);
            return (
                format!("\x1b]8;;{destination}\x1b\\{styled}\x1b]8;;\x1b\\"),
                true,
            );
        } else if destination.is_some() {
            stats.deferred_link_segments += 1;
        }
    }

    // Streaming fragments and unresolved references may not have a destination
    // yet. Keep the visible link styling but never emit a partial OSC 8 target.
    (
        MONOKAI.encode_segment(segment.style.with(StyleRole::LINK), &segment.text),
        false,
    )
}

pub(crate) fn segment_boundaries(
    document: &Document,
    style: StyleRole,
    link_id: Option<mdtui3::LinkId>,
    options: OutputOptions,
    stats: &mut OutputStats,
    destinations: &mut HashMap<mdtui3::LinkId, Option<String>>,
) -> (String, String, bool) {
    segment_boundaries_body(document, None, style, link_id, options, stats, destinations)
}
pub(crate) fn segment_boundaries_body(
    document: &Document,
    body: Option<mdtui3::BodyView<'_>>,
    style: StyleRole,
    link_id: Option<mdtui3::LinkId>,
    options: OutputOptions,
    stats: &mut OutputStats,
    destinations: &mut HashMap<mdtui3::LinkId, Option<String>>,
) -> (String, String, bool) {
    let segment = TextSegment {
        style,
        link_id,
        text: "\0".to_owned(),
    };
    let (encoded, linked) = encode_segment(document, body, &segment, options, stats, destinations);
    let (prefix, suffix) = encoded
        .split_once('\0')
        .expect("encoding preserves sentinel");
    (prefix.to_owned(), suffix.to_owned(), linked)
}

fn append_cell_text(output: &mut String, cell: &Cell) {
    for character in cell.text().chars() {
        output.push(if character == '\t' { ' ' } else { character });
    }
}

fn cursor_sequence(run: &DiffRun) -> String {
    format!(
        "\x1b[{};{}H",
        u32::from(run.row) + 1,
        u32::from(run.start_col) + 1
    )
}

pub(crate) fn validate_diff(screen: &Screen, diff: &ScreenDiff) -> io::Result<()> {
    let content = screen.content_area();
    let left = u32::from(content.x);
    let right = u32::from(content.right());
    let terminal_right = u32::from(screen.cols());
    for run in &diff.runs {
        if run.cells.is_empty() {
            continue;
        }
        if run.row >= screen.rows() {
            return Err(invalid_data(format!(
                "diff row {} is outside {} screen rows",
                run.row,
                screen.rows()
            )));
        }
        let start = u32::from(run.start_col);
        if start < left || start >= right {
            return Err(invalid_data(format!(
                "diff row {} starts at column {}, outside content columns {left}..{right}",
                run.row, run.start_col
            )));
        }

        let mut column = start;
        for cell in &run.cells {
            validate_cell_text(cell)?;
            if cell.is_continuation() {
                continue;
            }
            let width = u32::from(cell.width());
            if !(0..=2).contains(&width) {
                return Err(invalid_data(format!(
                    "cell at row {} column {column} has invalid width {width}",
                    run.row
                )));
            }
            let text_width = cell.text().width();
            if text_width != usize::from(cell.width()) {
                return Err(invalid_data(format!(
                    "cell at row {} column {column} has declared width {width} but text width {text_width}",
                    run.row
                )));
            }
            let end = column.checked_add(width).ok_or_else(|| {
                invalid_data(format!(
                    "cell end overflow at row {} column {column}",
                    run.row
                ))
            })?;
            if end > right || end > terminal_right {
                return Err(invalid_data(format!(
                    "cell at row {} column {} ends at {end}, beyond content right {right} or terminal width {terminal_right}",
                    run.row, column
                )));
            }
            column = end;
        }
    }
    Ok(())
}

fn validate_cell_text(cell: &Cell) -> io::Result<()> {
    if cell.text().is_safe() {
        Ok(())
    } else {
        Err(invalid_data(
            "cell text contains an unsupported control character".to_owned(),
        ))
    }
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::num::NonZeroU16;

    use mdtui3::LinkId;
    use vt100::Color;

    use super::*;
    use crate::render::diff::{DiffRun, ScreenDiff, diff_screen};
    use crate::screen::{ContentArea, StyleRole};

    fn cell(text: &str, style: StyleRole, width: u8, link_id: Option<LinkId>) -> Cell {
        Cell::new(text, style, width, link_id, None).unwrap()
    }

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn cursor(row: u16, column: u16) -> String {
        format!("\x1b[{};{}H", u32::from(row) + 1, u32::from(column) + 1)
    }

    fn link_style(text: &str) -> String {
        MONOKAI.encode_segment(StyleRole::LINK, text)
    }

    fn manual_diff(runs: Vec<DiffRun>) -> ScreenDiff {
        ScreenDiff {
            runs,
            ..ScreenDiff::default()
        }
    }

    fn draw(screen: &mut Screen, row: u16, column: u16, cells: &[Cell]) {
        let mut column = column;
        for cell in cells {
            if cell.is_continuation() {
                continue;
            }
            screen.write_cell(row, column, cell.clone()).unwrap();
            column += u16::from(cell.width());
        }
    }

    #[test]
    fn empty_diff_writes_nothing() {
        let document = Document::new();
        let screen = Screen::new(1, 6, ContentArea::new(0, 6)).unwrap();
        let (bytes, stats) = diff_to_vec(
            &document,
            &screen,
            &ScreenDiff::default(),
            OutputOptions::default(),
        )
        .unwrap();

        assert!(bytes.is_empty());
        assert_eq!(stats, OutputStats::default());
    }

    #[test]
    fn absolute_cursor_and_adjacent_same_style_cells_merge() {
        let document = Document::new();
        let mut screen = Screen::new(2, 10, ContentArea::new(2, 6)).unwrap();
        draw(
            &mut screen,
            1,
            3,
            &[
                cell("a", StyleRole::STRONG, 1, None),
                cell("b", StyleRole::STRONG, 1, None),
            ],
        );
        let diff = diff_screen(&screen);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let expected = format!(
            "{}{}",
            cursor(1, 3),
            MONOKAI.encode_segment(StyleRole::STRONG, "ab")
        );

        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(
            stats,
            OutputStats {
                runs: 1,
                segments: 1,
                cells: 2,
                continuations: 0,
                osc8_links: 0,
                bytes: expected.len(),
                ..OutputStats::default()
            }
        );
    }

    #[test]
    fn separated_runs_reposition_and_style_boundaries_split_segments() {
        let document = Document::new();
        let mut screen = Screen::new(1, 10, ContentArea::new(2, 8)).unwrap();
        draw(
            &mut screen,
            0,
            2,
            &[
                cell("a", StyleRole::STRONG, 1, None),
                cell("b", StyleRole::STRONG, 1, None),
                cell("c", StyleRole::CODE, 1, None),
            ],
        );
        draw(&mut screen, 0, 6, &[cell("d", StyleRole::STRONG, 1, None)]);
        let diff = diff_screen(&screen);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let expected = format!(
            "{}{}{}{}{}",
            cursor(0, 2),
            MONOKAI.encode_segment(StyleRole::STRONG, "ab"),
            MONOKAI.encode_segment(StyleRole::CODE, "c"),
            cursor(0, 6),
            MONOKAI.encode_segment(StyleRole::STRONG, "d")
        );

        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(stats.runs, 2);
        assert_eq!(stats.segments, 3);
        assert_eq!(stats.cells, 4);
    }

    #[test]
    fn adjacent_links_merge_and_osc8_can_be_disabled_without_losing_style() {
        let document = parse("[one](https://one.example) [two](https://two.example)\n");
        let screen = Screen::new(1, 20, ContentArea::new(0, 20)).unwrap();
        let diff = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 0,
            cells: vec![
                cell("o", StyleRole::DEFAULT, 1, Some(LinkId(0))),
                cell("n", StyleRole::DEFAULT, 1, Some(LinkId(0))),
                cell("e", StyleRole::DEFAULT, 1, Some(LinkId(0))),
                cell(" ", StyleRole::DEFAULT, 1, None),
                cell("t", StyleRole::DEFAULT, 1, Some(LinkId(1))),
                cell("w", StyleRole::DEFAULT, 1, Some(LinkId(1))),
                cell("o", StyleRole::DEFAULT, 1, Some(LinkId(1))),
            ],
        }]);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let expected = format!(
            "{}{}",
            cursor(0, 0),
            concat!(
                "\x1b]8;;https://one.example\x1b\\",
                "\x1b[4m\x1b[38;2;139;233;253mone\x1b[0m",
                "\x1b]8;;\x1b\\",
                " ",
                "\x1b]8;;https://two.example\x1b\\",
                "\x1b[4m\x1b[38;2;139;233;253mtwo\x1b[0m",
                "\x1b]8;;\x1b\\",
            )
        );
        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(stats.cells, 7);
        assert_eq!(stats.segments, 3);
        assert_eq!(stats.osc8_links, 2);

        let styled_only = format!(
            "{}{} {}",
            cursor(0, 0),
            link_style("one"),
            link_style("two")
        );
        for options in [
            OutputOptions {
                link_mode: LinkMode::NoOsc8,
                allow_osc8: true,
            },
            OutputOptions {
                link_mode: LinkMode::Default,
                allow_osc8: false,
            },
        ] {
            let (bytes, stats) = diff_to_vec(&document, &screen, &diff, options).unwrap();
            assert_eq!(bytes, styled_only.as_bytes());
            assert!(!bytes.windows(5).any(|window| window == b"\x1b]8;;"));
            assert_eq!(stats.osc8_links, 0);
        }
    }

    #[test]
    fn multi_row_runs_close_sgr_and_osc8_before_the_next_line() {
        let document = parse("[site](https://example.com)\n");
        let mut screen = Screen::new(2, 12, ContentArea::new(0, 12)).unwrap();
        draw(
            &mut screen,
            0,
            0,
            &[
                cell("A", StyleRole::STRONG, 1, None),
                cell("L", StyleRole::DEFAULT, 1, Some(LinkId(0))),
                cell("Z", StyleRole::DEFAULT, 1, None),
            ],
        );
        draw(
            &mut screen,
            1,
            0,
            &[
                cell("C", StyleRole::CODE, 1, None),
                cell("Y", StyleRole::DEFAULT, 1, None),
            ],
        );
        let diff = diff_screen(&screen);
        let (bytes, _) = diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let expected = format!(
            "{}{}{}{}{}{}{}{}{}",
            cursor(0, 0),
            MONOKAI.encode_segment(StyleRole::STRONG, "A"),
            "\x1b]8;;https://example.com\x1b\\",
            link_style("L"),
            "\x1b]8;;\x1b\\",
            "Z",
            cursor(1, 0),
            MONOKAI.encode_segment(StyleRole::CODE, "C"),
            "Y",
        );
        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(count_occurrences(&bytes, b"\x1b[0m"), 3);
        assert_eq!(count_occurrences(&bytes, b"\x1b]8;;\x1b\\"), 1);
        assert_whitelisted(&bytes);

        let mut parser =
            vt100::Parser::new(NonZeroU16::new(2).unwrap(), NonZeroU16::new(12).unwrap(), 0);
        parser.process(&bytes);
        let terminal = parser.screen();
        let link = terminal.cell(0, 1).unwrap();
        assert_eq!(link.contents(), "L");
        assert_eq!(link.fgcolor(), Color::Rgb(139, 233, 253));
        assert!(link.underline());
        for (row, column, text) in [(0, 2, "Z"), (1, 0, "C"), (1, 1, "Y")] {
            let cell = terminal.cell(row, column).unwrap();
            assert_eq!(cell.contents(), text);
            assert_eq!(
                cell.fgcolor(),
                if text == "C" {
                    Color::Rgb(166, 226, 46)
                } else {
                    Color::Default
                }
            );
            assert!(!cell.underline());
        }
    }

    #[test]
    fn link_destination_is_raw_and_titles_are_not_emitted() {
        let document = parse("[site](https://example.com/?a=&amp;b \"Ignored title\")\n");
        let screen = Screen::new(1, 12, ContentArea::new(0, 12)).unwrap();
        let diff = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 0,
            cells: "site"
                .chars()
                .map(|character| {
                    cell(
                        &character.to_string(),
                        StyleRole::DEFAULT,
                        1,
                        Some(LinkId(0)),
                    )
                })
                .collect(),
        }]);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let text = String::from_utf8(bytes).unwrap();

        assert!(text.contains("\x1b]8;;https://example.com/?a=&amp;b\x1b\\"));
        assert!(!text.contains("Ignored title"));
        assert_eq!(stats.osc8_links, 1);
    }

    #[test]
    fn unresolved_and_empty_links_keep_style_but_never_open_osc8() {
        let unknown_document = Document::new();
        let empty_document = parse("[empty]()\n");
        let screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();

        for (document, text) in [(&unknown_document, "u"), (&empty_document, "e")] {
            let diff = manual_diff(vec![DiffRun {
                row: 0,
                start_col: 0,
                cells: vec![cell(text, StyleRole::DEFAULT, 1, Some(LinkId(0)))],
            }]);
            let (bytes, stats) =
                diff_to_vec(document, &screen, &diff, OutputOptions::default()).unwrap();
            assert_eq!(
                bytes,
                format!("{}{}", cursor(0, 0), link_style(text)).as_bytes()
            );
            assert_eq!(stats.osc8_links, 0);
        }
    }

    #[test]
    fn wide_bases_write_once_and_continuations_are_skipped() {
        let document = Document::new();
        let mut screen = Screen::new(1, 12, ContentArea::new(2, 8)).unwrap();
        draw(
            &mut screen,
            0,
            2,
            &[
                cell("你", StyleRole::DEFAULT, 2, None),
                cell("x", StyleRole::DEFAULT, 1, None),
            ],
        );
        draw(
            &mut screen,
            0,
            8,
            &[cell("界", StyleRole::DEFAULT, 2, None)],
        );
        let diff = diff_screen(&screen);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();

        assert_eq!(
            bytes,
            format!("{}你x{}界", cursor(0, 2), cursor(0, 8)).as_bytes()
        );
        assert_eq!(stats.runs, 2);
        assert_eq!(stats.cells, 3);
        assert_eq!(stats.continuations, 2);
        assert_whitelisted(&bytes);
    }

    #[test]
    fn vt100_replay_matches_visible_cells_and_styles() {
        let document = parse("[L](https://example.com)\n");
        let mut screen = Screen::new(3, 14, ContentArea::new(2, 10)).unwrap();
        draw(
            &mut screen,
            0,
            2,
            &[
                cell("A", StyleRole::STRONG, 1, None),
                cell("B", StyleRole::CODE, 1, None),
                cell("D", StyleRole::DEFAULT, 1, None),
                cell("你", StyleRole::DEFAULT, 2, None),
                cell("L", StyleRole::LINK, 1, Some(LinkId(0))),
                cell("Z", StyleRole::DEFAULT, 1, None),
            ],
        );
        draw(&mut screen, 1, 4, &[cell("─", StyleRole::RULE, 1, None)]);
        draw(&mut screen, 2, 8, &[cell("m", StyleRole::META, 1, None)]);
        let diff = diff_screen(&screen);
        let (bytes, _) = diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let mut parser =
            vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(14).unwrap(), 0);
        parser.process(&bytes);
        let terminal = parser.screen();

        let strong = terminal.cell(0, 2).unwrap();
        assert_eq!(strong.contents(), "A");
        assert_eq!(strong.fgcolor(), Color::Rgb(255, 121, 198));
        assert!(strong.bold());

        let code = terminal.cell(0, 3).unwrap();
        assert_eq!(code.contents(), "B");
        assert_eq!(code.fgcolor(), Color::Rgb(166, 226, 46));
        assert_eq!(code.bgcolor(), Color::Rgb(40, 40, 40));
        assert!(!code.bold());

        let default = terminal.cell(0, 4).unwrap();
        assert_eq!(default.contents(), "D");
        assert_eq!(default.fgcolor(), Color::Default);
        assert_eq!(default.bgcolor(), Color::Default);
        assert!(!default.bold());
        assert!(!default.underline());

        let wide = terminal.cell(0, 5).unwrap();
        assert_eq!(wide.contents(), "你");
        assert!(wide.is_wide());
        assert!(terminal.cell(0, 6).unwrap().is_wide_continuation());

        let link = terminal.cell(0, 7).unwrap();
        assert_eq!(link.contents(), "L");
        assert_eq!(link.fgcolor(), Color::Rgb(139, 233, 253));
        assert!(link.underline());

        let after_link = terminal.cell(0, 8).unwrap();
        assert_eq!(after_link.contents(), "Z");
        assert_eq!(after_link.fgcolor(), Color::Default);
        assert_eq!(after_link.bgcolor(), Color::Default);
        assert!(!after_link.underline());

        let rule = terminal.cell(1, 4).unwrap();
        assert_eq!(rule.contents(), "─");
        assert_eq!(rule.fgcolor(), Color::Rgb(117, 113, 94));

        let meta = terminal.cell(2, 8).unwrap();
        assert_eq!(meta.contents(), "m");
        assert_eq!(meta.fgcolor(), Color::Rgb(117, 113, 94));
    }

    #[test]
    fn output_whitelist_excludes_clear_home_and_line_clear_sequences() {
        let document = parse("[L](https://example.com/?a=&amp;b)\n");
        let screen = Screen::new(1, 12, ContentArea::new(1, 8)).unwrap();
        let diff = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 1,
            cells: vec![
                cell("A", StyleRole::STRONG, 1, None),
                cell("L", StyleRole::LINK, 1, Some(LinkId(0))),
            ],
        }]);
        let (bytes, _) = diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();

        assert_whitelisted(&bytes);
        assert!(!bytes.windows(4).any(|window| window == b"\x1b[2J"));
        assert!(!bytes.windows(3).any(|window| window == b"\x1b[H"));
        assert!(!bytes.windows(3).any(|window| window == b"\x1b[K"));
    }

    #[test]
    fn out_of_bounds_runs_fail_before_any_bytes_are_written() {
        let document = Document::new();
        let screen = Screen::new(1, 6, ContentArea::new(1, 3)).unwrap();
        let cases = [
            manual_diff(vec![DiffRun {
                row: 1,
                start_col: 1,
                cells: vec![cell("x", StyleRole::DEFAULT, 1, None)],
            }]),
            manual_diff(vec![DiffRun {
                row: 0,
                start_col: 0,
                cells: vec![cell("x", StyleRole::DEFAULT, 1, None)],
            }]),
            manual_diff(vec![DiffRun {
                row: 0,
                start_col: 3,
                cells: vec![cell("你", StyleRole::DEFAULT, 2, None)],
            }]),
            manual_diff(vec![
                DiffRun {
                    row: 0,
                    start_col: 1,
                    cells: vec![cell("x", StyleRole::DEFAULT, 1, None)],
                },
                DiffRun {
                    row: 0,
                    start_col: 3,
                    cells: vec![cell("你", StyleRole::DEFAULT, 2, None)],
                },
            ]),
        ];

        for diff in cases {
            let mut bytes = Vec::new();
            let error = write_diff(
                &mut bytes,
                &document,
                &screen,
                &diff,
                OutputOptions::default(),
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn tabs_are_one_cell_spaces_and_other_controls_are_rejected() {
        let document = Document::new();
        let screen = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        let tab = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 0,
            cells: vec![cell("\t", StyleRole::DEFAULT, 1, None)],
        }]);
        let (bytes, stats) =
            diff_to_vec(&document, &screen, &tab, OutputOptions::default()).unwrap();
        assert_eq!(bytes, format!("{} ", cursor(0, 0)).as_bytes());
        assert_eq!(stats.cells, 1);
        assert!(!bytes.contains(&b'\t'));

        let control = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 0,
            cells: vec![cell("\x1b", StyleRole::DEFAULT, 1, None)],
        }]);
        let mut bytes = Vec::new();
        let error = write_diff(
            &mut bytes,
            &document,
            &screen,
            &control,
            OutputOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(bytes.is_empty());

        let mismatched_width = manual_diff(vec![DiffRun {
            row: 0,
            start_col: 0,
            cells: vec![cell("xx", StyleRole::DEFAULT, 1, None)],
        }]);
        let mut bytes = Vec::new();
        let error = write_diff(
            &mut bytes,
            &document,
            &screen,
            &mismatched_width,
            OutputOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(bytes.is_empty());
    }

    #[test]
    fn failing_writer_returns_error_and_successful_output_is_deterministic() {
        let document = Document::new();
        let mut screen = Screen::new(1, 8, ContentArea::new(0, 8)).unwrap();
        draw(
            &mut screen,
            0,
            1,
            &[
                cell("a", StyleRole::STRONG, 1, None),
                cell("b", StyleRole::STRONG, 1, None),
            ],
        );
        let diff = diff_screen(&screen);
        let front = screen.front_row(0).unwrap().to_vec();
        let back = screen.back_row(0).unwrap().to_vec();
        let (first, first_stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();
        let (second, second_stats) =
            diff_to_vec(&document, &screen, &diff, OutputOptions::default()).unwrap();

        assert_eq!(first, second);
        assert_eq!(first_stats, second_stats);
        assert_eq!(screen.front_row(0).unwrap(), front.as_slice());
        assert_eq!(screen.back_row(0).unwrap(), back.as_slice());

        let mut failing = FailingWriter { remaining: 2 };
        let error = write_diff(
            &mut failing,
            &document,
            &screen,
            &diff,
            OutputOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    struct FailingWriter {
        remaining: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected write failure"));
            }
            let written = self.remaining.min(bytes.len());
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn assert_whitelisted(bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                0x1b => {
                    if bytes.get(index + 1) == Some(&b'[') {
                        let body_start = index + 2;
                        let mut end = body_start;
                        while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                            assert!(
                                bytes[end].is_ascii_digit() || matches!(bytes[end], b';' | b':'),
                                "unexpected CSI parameter byte 0x{:02x}",
                                bytes[end]
                            );
                            end += 1;
                        }
                        assert!(end < bytes.len(), "unterminated CSI");
                        let body = std::str::from_utf8(&bytes[body_start..end]).unwrap();
                        match bytes[end] {
                            b'H' => {
                                let mut parts = body.split(';');
                                assert!(parts.next().is_some_and(|part| !part.is_empty()));
                                assert!(parts.next().is_some_and(|part| !part.is_empty()));
                                assert!(parts.next().is_none(), "cursor has extra parameters");
                            }
                            b'm' => assert!(!body.is_empty(), "empty SGR"),
                            other => panic!("unexpected CSI final byte {:?}", other as char),
                        }
                        index = end + 1;
                    } else if bytes.get(index + 1) == Some(&b']') {
                        let prefix = b"\x1b]8;;";
                        assert!(bytes[index..].starts_with(prefix));
                        let destination_start = index + prefix.len();
                        let destination_end = bytes[destination_start..]
                            .windows(2)
                            .position(|window| window == b"\x1b\\")
                            .map(|offset| destination_start + offset)
                            .expect("unterminated OSC 8");
                        assert!(
                            bytes[destination_start..destination_end]
                                .iter()
                                .all(|byte| *byte >= 0x20 && *byte != 0x7f)
                        );
                        index = destination_end + 2;
                    } else {
                        panic!("unexpected ESC sequence");
                    }
                }
                byte if byte < 0x20 || byte == 0x7f => {
                    panic!("unexpected control byte 0x{byte:02x}")
                }
                _ => index += 1,
            }
        }
    }

    fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }
}
