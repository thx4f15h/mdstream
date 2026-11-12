mod support;

use std::process::{Command, Output, Stdio};

use mdtui_tui::static_render::ONCE_CLEAR;
use support::env::fixed_std_command;
use support::temp::TempWorkspace;

const BINARY: &str = env!("CARGO_BIN_EXE_mdtui-tui");

fn run_once(source: &str, width: u16, height: u16, extra: &[&str]) -> Output {
    let workspace = TempWorkspace::new().expect("temporary workspace");
    let file = workspace
        .write("input.md", source)
        .expect("write once fixture");
    let mut command: Command = fixed_std_command(BINARY);
    command
        .arg("--once")
        .arg("--width")
        .arg(width.to_string())
        .arg("--height")
        .arg(height.to_string())
        .args(extra)
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .expect("run once binary")
}

fn assert_golden(source: &str, width: u16, height: u16, golden: &[u8]) {
    let output = run_once(source, width, height, &[]);
    assert_eq!(output.status.code(), Some(0), "{golden:?}");
    assert!(output.stderr.is_empty(), "{golden:?}");
    assert_eq!(output.stdout, golden);
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

#[test]
fn once_raw_goldens_cover_all_required_blocks_and_overflow() {
    assert_golden(
        "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n",
        80,
        20,
        include_bytes!("golden/once_headings.ansi"),
    );
    assert_golden(
        "one  two\tthree four five\nsoft\n\nnext\n",
        10,
        20,
        include_bytes!("golden/once_paragraphs.ansi"),
    );
    assert_golden(
        "**bold** *italic* ~~strike~~ `code` &amp;\n",
        80,
        20,
        include_bytes!("golden/once_inline.ansi"),
    );
    assert_golden(
        "[web](https://example.com) <user@example.com> [empty]()\n",
        80,
        20,
        include_bytes!("golden/once_links.ansi"),
    );
    assert_golden(
        "```rust extra\nlong code line\n\nend\n```\n",
        80,
        20,
        include_bytes!("golden/once_code.ansi"),
    );
    assert_golden(
        "| A | B |\n| --- | --- |\n| x | [l](https://e) |\n",
        80,
        20,
        include_bytes!("golden/once_tables.ansi"),
    );
    assert_golden(
        "# 1234567890\n\nabc def ghi\n\n```\n1234567890\n```\n\n| A | B |\n| --- | --- |\n| 1234567890 | z |\n",
        5,
        20,
        include_bytes!("golden/once_overwide.ansi"),
    );
    assert_golden(
        "- alpha beta gamma\n\n> quoted words here\n",
        8,
        20,
        include_bytes!("golden/once_flow.ansi"),
    );
}

#[test]
fn once_images_use_clickable_alt_and_an_empty_alt_placeholder() {
    assert_golden(
        "![alt](image.png)\n\n![](empty.png)\n",
        80,
        24,
        include_bytes!("golden/once_images.ansi"),
    );
}

#[test]
fn once_empty_and_whitespace_are_zero_bytes() {
    assert_golden("", 20, 10, include_bytes!("golden/once_empty.ansi"));
    assert_golden(
        "   \n\t\n\n",
        20,
        10,
        include_bytes!("golden/once_whitespace.ansi"),
    );
}

#[test]
fn once_clear_prefix_is_emitted_once_for_nonempty_content() {
    let output = run_once("# one\n", 20, 10, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.starts_with(ONCE_CLEAR));
    assert_eq!(count(&output.stdout, b"\x1b[2J"), 1);
    assert_eq!(count(&output.stdout, b"\x1b[H"), 1);
}

#[test]
fn once_table_uses_internal_lf_and_block_final_crlf() {
    let output = run_once("| A | B |\n| --- | --- |\n| x | y |\n", 80, 20, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.ends_with(b"\r\n"));
    assert_eq!(count(&output.stdout, b"\r\n"), 1);
    assert_eq!(count(&output.stdout, b"\n"), 5);
    assert!(!output.stdout.windows(3).any(|window| window == b"\n\r\n"));
}

#[test]
fn once_overwide_heading_code_and_table_are_not_clipped() {
    let output = run_once(
        "# 1234567890\n\n```\n1234567890\n```\n\n| A |\n| --- |\n| 1234567890 |\n",
        5,
        20,
        &[],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        count(&output.stdout, b"1234567890"),
        3,
        "heading, code and table content must all survive"
    );
}

#[test]
fn once_table_links_keep_style_without_osc8() {
    let output = run_once(
        "| link |\n| --- |\n| [label](https://example.com) |\n",
        80,
        20,
        &[],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(!output.stdout.windows(3).any(|window| window == b"\x1b]8"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("label"));
}

#[test]
fn once_no_links_removes_osc8_but_keeps_link_style() {
    let source = "[web](https://example.com) <user@example.com>\n";
    let normal = run_once(source, 80, 20, &[]);
    let no_links = run_once(source, 80, 20, &["--no-links"]);
    assert_eq!(no_links.status.code(), Some(0));
    assert!(!no_links.stdout.windows(3).any(|window| window == b"\x1b]8"));
    assert!(
        String::from_utf8_lossy(&no_links.stdout)
            .contains("\x1b[4m\x1b[38;2;139;233;253mweb\x1b[0m")
    );
    assert_ne!(normal.stdout, no_links.stdout);
}

#[test]
fn once_adjacent_equal_styles_keep_independent_resets() {
    let output = run_once("**a**__b__\n", 80, 20, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output
            .stdout
            .windows(b"a\x1b[0m\x1b[1m".len())
            .any(|window| window == b"a\x1b[0m\x1b[1m")
    );
}

#[test]
fn once_open_links_does_not_change_the_one_shot_bytes() {
    let source = "[web](https://example.com)\n";
    let default = run_once(source, 80, 20, &[]);
    let open = run_once(source, 80, 20, &["--open-links"]);
    assert_eq!(open.status.code(), Some(0));
    assert_eq!(open.stdout, default.stdout);
}
