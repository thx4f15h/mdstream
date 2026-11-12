use mdtui_tui::render::display::{DisplayScanner, ScanStatus};
use mdtui_tui::render::input::TokenKind;
use mdtui3::{Budget, Document, LayoutError, Span, Style, TILE_BYTES, TileId, WorkLimit};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn scan_all(document: &Document, span: Span) -> DisplayScanner {
    let mut scanner = DisplayScanner::new(document, span).expect("display range must be valid");
    let mut budget = unlimited();
    assert_eq!(
        scanner.advance(document, &mut budget),
        Ok(ScanStatus::Complete)
    );
    scanner
}

fn static_visible(document: &Document, span: Span) -> String {
    document
        .display_runs_in(span)
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect()
}

fn scanned_text(scanner: &DisplayScanner) -> String {
    scanner
        .segments()
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

#[test]
fn entity_replacement_keeps_display_text_source_and_full_replacement() {
    let source = "a &amp; b\n";
    let document = parse(source);
    let scanner = scan_all(&document, Span::new(0, source.len()));

    assert_eq!(scanned_text(&scanner), "a & b\n");
    assert_eq!(
        scanned_text(&scanner),
        static_visible(&document, Span::new(0, source.len()))
    );
    let entity_start = source.find("&amp;").unwrap();
    let entity = scanner
        .segments()
        .iter()
        .find(|segment| segment.text == "&")
        .expect("entity display segment");
    assert_eq!(entity.kind, TokenKind::Text);
    assert_eq!(
        entity.replacement,
        Some(Span::new(entity_start, entity_start + "&amp;".len()))
    );
    assert_eq!(entity.tile, document.tile_at(entity_start).unwrap());
    assert_eq!(scanner.stats().display_bytes, scanned_text(&scanner).len());
    assert!(scanner.stats().segments > 0);
}

#[test]
fn code_whitespace_is_normalized_once_and_backticks_disappear() {
    let body = format!("a  \tb {}z", "c  \t".repeat(100));
    let source = format!("`{body}`\n");
    let document = parse(&source);
    let scanner = scan_all(&document, Span::new(0, source.len()));
    let text = scanned_text(&scanner);

    assert!(text.contains("a  \tb c"), "unexpected code text: {text:?}");
    assert!(!text.contains('`'));
    assert_eq!(text, static_visible(&document, Span::new(0, source.len())));
    assert!(source.len() > TILE_BYTES);
    assert!(scanner.segments().iter().any(|segment| {
        segment.text.contains('\t') && segment.style.contains(mdtui_tui::screen::StyleRole::CODE)
    }));
}

#[test]
fn hidden_container_prefixes_never_become_segments() {
    let source = "> - *hello* [link](https://example.com)\n";
    let document = parse(source);
    let scanner = scan_all(&document, Span::new(0, source.len()));
    let text = scanned_text(&scanner);

    assert!(text.starts_with("hello"));
    assert!(!text.contains("> "));
    assert!(!text.contains("- "));
    assert_eq!(text, static_visible(&document, Span::new(0, source.len())));
}

#[test]
fn ordinary_runs_split_at_tile_boundaries_in_source_order() {
    let source = format!("{}\n", "x".repeat(TILE_BYTES * 2 + 17));
    let document = parse(&source);
    let scanner = scan_all(&document, Span::new(0, source.len()));

    assert_eq!(scanned_text(&scanner), source);
    assert!(
        scanner
            .segments()
            .iter()
            .all(|segment| segment.replacement.is_none())
    );
    let mut previous_tile = 0;
    let mut previous_end = 0;
    for segment in scanner.segments() {
        assert!(segment.tile.0 >= previous_tile);
        assert_eq!(segment.source.start, previous_end);
        let tile = document.tile(segment.tile).unwrap();
        assert!(tile.start <= segment.source.start);
        assert!(segment.source.end <= tile.end);
        previous_tile = segment.tile.0;
        previous_end = segment.source.end;
    }
    assert_eq!(previous_end, source.len());
    assert_eq!(
        scanner
            .segments_for_tile(TileId(0))
            .map(|segment| segment.text.as_str())
            .collect::<String>(),
        "x".repeat(TILE_BYTES)
    );
    assert_eq!(
        scanner.complete_tiles(),
        &(0..document.tile_count()).map(TileId).collect::<Vec<_>>()
    );
}

#[test]
fn only_tiles_fully_covered_by_the_request_are_complete() {
    let source = "x".repeat(TILE_BYTES * 2);
    let document = parse(&source);
    assert_eq!(document.tile_count(), 2);
    let scanner = scan_all(&document, Span::new(10, source.len()));

    assert_eq!(scanner.complete_tiles(), &[TileId(1)]);
    assert_eq!(scanner.stats().complete_tiles, 1);
    assert_eq!(scanner.scanned_through(), source.len());
}

#[test]
fn entity_crossing_a_tile_boundary_is_owned_by_its_start_tile() {
    let prefix = "x".repeat(TILE_BYTES - 2);
    let source = format!("{prefix}&amp;y\n");
    let document = parse(&source);
    let scanner = scan_all(&document, Span::new(0, source.len()));
    let entity_start = TILE_BYTES - 2;
    assert_eq!(document.tile_at(entity_start), Some(TileId(0)));
    assert_eq!(document.tile_at(entity_start + 3), Some(TileId(1)));

    let entities: Vec<_> = scanner
        .segments()
        .iter()
        .filter(|segment| segment.text == "&")
        .collect();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].tile, TileId(0));
    assert_eq!(
        entities[0].replacement,
        Some(Span::new(entity_start, entity_start + 5))
    );
    assert_eq!(
        scanned_text(&scanner),
        static_visible(&document, Span::new(0, source.len()))
    );
}

#[test]
fn tiny_budget_resumes_without_duplicates_and_loses_no_prefix() {
    let source = "a &amp; b **c** `a  b`\n\n".repeat(20);
    let document = parse(&source);
    let span = Span::new(0, source.len());
    let expected = static_visible(&document, span);
    let mut scanner = DisplayScanner::new(&document, span).unwrap();
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..WorkLimit::default()
    };
    let mut frames = 0;
    loop {
        frames += 1;
        assert!(frames < 2_000, "scanner failed to make progress");
        let before_segments = scanner.segments().to_vec();
        let before_scanned = scanner.scanned_through();
        let before_tiles = scanner.complete_tiles().to_vec();
        let mut budget = Budget::new(limit);
        let status = scanner.advance(&document, &mut budget).unwrap();
        assert!(scanner.segments().starts_with(&before_segments));
        assert!(scanner.scanned_through() >= before_scanned);
        assert!(scanner.complete_tiles().starts_with(&before_tiles));
        assert!(budget.used().queries <= 1);
        assert!(budget.used().objects <= 1);
        assert!(budget.used().bytes <= 6);
        if status == ScanStatus::Complete {
            break;
        }
        assert_eq!(status, ScanStatus::BudgetExhausted);
        // Metadata-only seek/selection frames are valid progress; the finite
        // frame bound above verifies that they eventually reach source bytes.
        assert!(budget.used().queries > 0);
    }
    assert!(frames > 1);
    assert_eq!(scanned_text(&scanner), expected);
    assert!(scanner.is_complete());
    assert_eq!(scanner.scanned_through(), source.len());
    assert_eq!(
        scanner.complete_tiles(),
        &(0..document.tile_count()).map(TileId).collect::<Vec<_>>()
    );
}

#[test]
fn append_and_finish_make_existing_scanners_stale() {
    let mut document = Document::new();
    document.append("alpha\n").unwrap();
    let span = Span::new(0, document.source().len());
    let mut scanner = DisplayScanner::new(&document, span).unwrap();
    document.append("").unwrap();
    assert!(scanner.matches(&document));

    document.append("beta\n").unwrap();
    assert!(!scanner.matches(&document));
    let mut budget = unlimited();
    assert_eq!(
        scanner.advance(&document, &mut budget),
        Err(LayoutError::StaleContinuation)
    );

    let mut finishing = Document::new();
    finishing.append("text\n").unwrap();
    let finish_span = Span::new(0, finishing.source().len());
    let mut finish_scanner = DisplayScanner::new(&finishing, finish_span).unwrap();
    finishing.finish();
    let mut budget = unlimited();
    assert_eq!(
        finish_scanner.advance(&finishing, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn tiny_budget_cannot_scan_a_large_document_in_one_frame() {
    let source = "x".repeat(64 * 1024);
    let document = parse(&source);
    let span = Span::new(0, source.len());
    let mut scanner = DisplayScanner::new(&document, span).unwrap();
    let mut budget = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..WorkLimit::default()
    });

    assert_eq!(
        scanner.advance(&document, &mut budget),
        Ok(ScanStatus::BudgetExhausted)
    );
    assert_eq!(
        budget.used(),
        WorkLimit {
            queries: 1,
            ..WorkLimit::default()
        }
    );
    assert!(!scanner.is_complete());
    assert!(scanner.scanned_through() <= 4);
    assert!(scanner.scanned_through() < source.len());
    assert!(scanner.segments().len() <= 2);
    assert_eq!(scanner.stats().budget_exhausted, 1);
}
