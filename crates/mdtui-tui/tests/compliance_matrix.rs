use std::path::Path;

const MATRIX: &str = include_str!("../../../docs/tui-render-compliance.md");
const ONCE_TESTS: &str = include_str!("once_ansi.rs");
const STATIC_TESTS: &str = include_str!("static_render.rs");

const TOP_LEVEL_SECTIONS: &[&str] = &[
    "## §0", "## §1", "## §2", "## §3", "## §4", "## §5", "## §6", "## §9",
];

const SUBSECTIONS: &[&str] = &[
    "### §3.1",
    "### §3.2",
    "### §3.3",
    "### §3.4",
    "### §3.5",
    "### §3.6",
    "### §3.7",
    "### §3.8",
    "### §3.9",
    "### §3.10",
    "### §3.11",
    "### §3.12",
    "### §3.13",
];

const FIXTURES: &[&str] = &[
    "fixtures/spec/headings.md",
    "fixtures/spec/paragraphs.md",
    "fixtures/spec/inline.md",
    "fixtures/spec/links.md",
    "fixtures/spec/images.md",
    "fixtures/spec/quotes.md",
    "fixtures/spec/lists.md",
    "fixtures/spec/code.md",
    "fixtures/spec/overwide.md",
    "fixtures/spec/hr.md",
    "fixtures/spec/tables.md",
    "fixtures/spec/spacing.md",
    "fixtures/spec/footnotes.md",
    "fixtures/spec/html.md",
];

const GOLDENS: &[&str] = &[
    "golden/once_headings.ansi",
    "golden/once_paragraphs.ansi",
    "golden/once_inline.ansi",
    "golden/once_links.ansi",
    "golden/once_images.ansi",
    "golden/once_code.ansi",
    "golden/once_tables.ansi",
    "golden/once_overwide.ansi",
    "golden/once_flow.ansi",
    "golden/once_empty.ansi",
    "golden/once_whitespace.ansi",
];

const DIFFERENCE_TERMS: &[&str] = &[
    "HTML 块",
    "ratatui §8",
    "TUI 行模型",
    "嵌套强调外层前景",
    "MATH",
    "超宽不折行块",
    "表格内链接",
    "--no-links",
    "空输入/纯空白",
];

#[test]
fn compliance_matrix_has_sections_fixtures_goldens_and_named_evidence() {
    for heading in TOP_LEVEL_SECTIONS {
        assert!(
            MATRIX.contains(heading),
            "missing matrix heading {heading:?}"
        );
    }
    for heading in SUBSECTIONS {
        assert!(
            MATRIX.contains(heading),
            "missing matrix heading {heading:?}"
        );
    }

    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for fixture in FIXTURES {
        assert!(
            tests_dir.join(fixture).is_file(),
            "missing fixture {fixture}"
        );
    }
    for golden in GOLDENS {
        assert!(tests_dir.join(golden).is_file(), "missing golden {golden}");
    }
    assert!(
        tests_dir.join("golden/.gitattributes").is_file(),
        "golden attributes file is missing"
    );

    for test_name in [
        "once_raw_goldens_cover_all_required_blocks_and_overflow",
        "once_images_use_clickable_alt_and_an_empty_alt_placeholder",
        "once_empty_and_whitespace_are_zero_bytes",
        "once_clear_prefix_is_emitted_once_for_nonempty_content",
        "once_table_uses_internal_lf_and_block_final_crlf",
        "once_overwide_heading_code_and_table_are_not_clipped",
        "once_table_links_keep_style_without_osc8",
        "once_no_links_removes_osc8_but_keeps_link_style",
        "once_open_links_does_not_change_the_one_shot_bytes",
        "once_adjacent_equal_styles_keep_independent_resets",
    ] {
        assert!(
            ONCE_TESTS.contains(test_name),
            "missing once test {test_name}"
        );
    }
    for test_name in [
        "all_fixtures_have_stable_cell_snapshots_at_all_target_widths",
        "spacing_fixture_stats_match_p25_oracle",
        "row_overflow_is_reported_by_the_static_api",
    ] {
        assert!(
            STATIC_TESTS.contains(test_name),
            "missing static test {test_name}"
        );
    }

    for term in DIFFERENCE_TERMS {
        assert!(MATRIX.contains(term), "missing difference term {term:?}");
    }
}
