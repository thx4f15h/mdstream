//! Independent hand grids/source/budget checks against the production paged
//! engine and ACTUAL native objects. No test-only object metadata installer.
use mdtui_tui::render::math::*;
use mdtui3::{Budget, Document, MathObjectMeta, MathWindowStep, Span, WorkLimit};
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..WorkLimit::default()
    })
}
fn first(document: &Document) -> MathObjectMeta {
    let mut w = document
        .math_window(Span::new(0, document.source().len()))
        .unwrap();
    for _ in 0..document.source().len() + 16 {
        match w.advance(document, &mut tiny()).unwrap() {
            MathWindowStep::Object(m) => return m,
            MathWindowStep::BudgetExhausted => {}
            MathWindowStep::Complete => panic!("real native math object"),
        }
    }
    panic!("native math window stalled under positive budget")
}
fn document(tex: &str) -> Document {
    let mut d = Document::new();
    d.append(&format!("\\({tex}\\)\n")).unwrap();
    d.finish();
    d
}
fn finish(engine: &mut MathEngine, d: &Document) -> MathProgress {
    for _ in 0..d.source().len() * 70 + 300 {
        let mut b = tiny();
        let status = engine.advance(d, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if matches!(
            status,
            MathProgress::Ready | MathProgress::PendingInput | MathProgress::NeedShape
        ) {
            return status;
        }
    }
    panic!("continued engine stalled")
}
fn draw(engine: &MathEngine, d: &Document, rect: Rect) -> (Vec<String>, Vec<MathCell>) {
    let mut paint = engine.paint(rect).unwrap();
    assert_eq!(paint.storage().pages, 0);
    assert_eq!(
        paint
            .advance(engine, d, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        PaintStep::BudgetExhausted
    );
    assert_eq!(paint.storage().pages, 0);
    let mut rows = vec![vec![String::from(" "); rect.width]; rect.height];
    let mut cells = Vec::new();
    for _ in 0..d.source().len() * 20 + rect.width * rect.height * 10 + 100 {
        let mut b = tiny();
        match paint.advance(engine, d, &mut b).unwrap() {
            PaintStep::Complete => {
                return (
                    rows.into_iter()
                        .map(|row| row.concat().trim_end().to_owned())
                        .collect(),
                    cells,
                );
            }
            PaintStep::Cell(cell) => {
                let mut bytes = Vec::new();
                let mut ink = engine.ink_cursor(cell.ink).unwrap();
                let mut out = [0; 4];
                assert_eq!(
                    ink.advance(engine, d, &mut Budget::new(WorkLimit::default()), &mut out)
                        .unwrap(),
                    InkReadStep::BudgetExhausted
                );
                loop {
                    match ink.advance(engine, d, &mut tiny(), &mut out).unwrap() {
                        InkReadStep::Bytes(n) => bytes.extend_from_slice(&out[..n]),
                        InkReadStep::Complete => break,
                        InkReadStep::BudgetExhausted => {}
                    }
                }
                let text = String::from_utf8(bytes).unwrap();
                rows[cell.y - rect.y][cell.x - rect.x] = text;
                for x in 1..usize::from(cell.width) {
                    rows[cell.y - rect.y][cell.x - rect.x + x].clear();
                }
                cells.push(cell)
            }
            _ => {}
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
    }
    panic!("paint stalled")
}
#[test]
fn finite_handwritten_grids_use_real_native_payload_and_bounded_pages() {
    let cases: [(&str, &[&str], usize); 16] = [
        (r"\frac{a}{b}", &[" a", "───", " b"], 1),
        (
            r"\frac{1}{\frac{a}{b}}",
            &["  1", "─────", "  a", " ───", "  b"],
            1,
        ),
        (r"x_1^2", &[" 2", "x", " 1"], 1),
        (r"x^{a_b}", &[" a", "  b", "x"], 2),
        (r"\sqrt{x}", &[" ─", "√x"], 1),
        (r"\left(\frac{a}{b}\right)", &["⎛ a ⎞", "⎜───⎟", "⎝ b ⎠"], 1),
        (r"\alpha+\beta\leq\infty", &["α+β≤∞"], 0),
        (r"\sin x+\log y", &["sinx+logy"], 0),
        (
            r"\begin{matrix}a&b\\c&d\end{matrix}",
            &["a  b", "", "c  d"],
            1,
        ),
        (
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            &["⎛a  b⎞", "⎜    ⎟", "⎝c  d⎠"],
            1,
        ),
        (r"\unknown{x}", &[r"\unknownx"], 0),
        (r"\frac{a}", &["[invalid]"], 0),
        (
            r"\begin{aligned}x&=\frac{1}{2}\\y&=3\end{aligned}",
            &["     1", "x  =───", "     2", "", "y  =3"],
            2,
        ),
        (r"\sqrt{\frac{a}{b}}", &[" ───", "  a", "√───", "  b"], 2),
        (
            r"\frac{\frac{a}{b}}{c}",
            &["  a", " ───", "  b", "─────", "  c"],
            3,
        ),
        (r"\sum_i^n", &[" n", "∑", " i"], 1),
    ];
    for (tex, expected, baseline) in cases {
        let d = document(tex);
        let mut engine = MathEngine::new(&d, first(&d)).unwrap();
        let initial = engine.stats();
        assert_eq!(initial.payload.pages, 0);
        assert!(initial.syntax_storage.iter().all(|s| s.pages == 0));
        assert!(initial.layout_storage.iter().all(|s| s.pages == 0));
        assert_eq!(
            engine
                .advance(&d, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            MathProgress::BudgetExhausted
        );
        assert_eq!(engine.stats().syntax.steps, 0);
        assert_eq!(finish(&mut engine, &d), MathProgress::Ready, "{tex}");
        let metrics = engine.metrics().unwrap();
        assert_eq!(metrics.baseline, baseline, "{tex}");
        let (grid, cells) = draw(
            &engine,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: metrics.width.max(1),
                height: metrics.height,
            },
        );
        assert_eq!(grid, expected, "{tex}");
        for cell in cells {
            assert!(
                cell.source.start >= 2 && cell.source.end <= tex.len() + 2,
                "{tex} {cell:?}"
            );
        }
        let stats = engine.stats();
        assert_eq!(stats.syntax.bytes, tex.len());
        assert_eq!(stats.payload_bytes, tex.len());
        for s in stats
            .syntax_storage
            .into_iter()
            .chain(stats.layout_storage)
            .chain([stats.payload])
        {
            assert_eq!(s.old_records_relocated, 0);
            assert!(s.max_page_bytes < 20000);
        }
    }
}
#[test]
fn growing_balanced_and_partial_commands_keep_prefix_without_reparse() {
    let mut d = Document::new();
    d.append("\\[\nx").unwrap();
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    assert_eq!(
        draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 8,
                height: 1
            }
        )
        .0,
        ["x"]
    );
    let before = e.stats();
    for (suffix, expected) in [
        ("+y", "x+y"),
        ("+\\fra", "x+y+"),
        ("c{1}", "x+y+"),
        ("{2}", "x+y+"),
    ] {
        d.append(suffix).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
        let metrics = e.metrics().unwrap();
        let grid = draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: metrics.width.max(1),
                height: metrics.height,
            },
        )
        .0;
        if metrics.height == 1 {
            assert_eq!(grid, [expected]);
        }
    }
    let after = e.stats();
    assert!(after.syntax.bytes > before.syntax.bytes);
    assert_eq!(after.syntax.bytes, after.payload_bytes);
    assert_eq!(after.source_bytes, after.payload_bytes);
    d.append("\n\\]\n").unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    let metrics = e.metrics().unwrap();
    assert_eq!(metrics.height, 3);
    assert_eq!(
        draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: metrics.width,
                height: 3
            }
        )
        .0,
        ["     1", "x+y+───", "     2"]
    );
}
#[test]
fn long_wide_groups_deep_stacks_and_retirement_never_relocate_old_records() {
    for tex in [
        "x".repeat(5000),
        format!("{}x{}", "\\sqrt{".repeat(2000), "}".repeat(2000)),
        format!(
            "\\begin{{matrix}}{}x&x\\end{{matrix}}",
            "x&x\\\\".repeat(1999)
        ),
        format!("\\begin{{matrix}}{}x\\end{{matrix}}", "x&".repeat(4999)),
    ] {
        let d = document(&tex);
        let mut e = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::Ready);
        let stats = e.stats();
        for s in stats
            .syntax_storage
            .into_iter()
            .chain(stats.layout_storage)
            .chain([stats.payload])
        {
            assert_eq!(s.old_records_relocated, 0)
        }
        let m = e.metrics().unwrap();
        let (_, cells) = draw(
            &e,
            &d,
            Rect {
                x: m.width.saturating_sub(8),
                y: m.height.saturating_sub(2),
                width: m.width.clamp(1, 8),
                height: m.height.min(2),
            },
        );
        assert!(cells.len() <= 16);
        let mut retire = e.into_retirement();
        let before = retire.pub_steps;
        assert!(!retire.advance(&mut Budget::new(WorkLimit::default())));
        assert_eq!(retire.pub_steps, before);
        for _ in 0..tex.len() * 50 + 100 {
            if retire.advance(&mut tiny()) {
                break;
            }
        }
        let final_stats = retire.stats();
        assert_eq!(final_stats.payload.pages, 0);
        assert!(
            final_stats
                .syntax_storage
                .iter()
                .chain(final_stats.layout_storage.iter())
                .all(|s| s.pages == 0)
        );
        assert_eq!(final_stats.index_entries, 0);
    }
}
#[test]
fn unicode_uses_shared_post_tex_shape_and_stale_tasks_reject() {
    let mut d = Document::new();
    d.append("\\[\n\\alpha{}\u{301}").unwrap();
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    assert_eq!(e.metrics().unwrap().width, 1);
    assert_eq!(
        draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 8,
                height: 1
            }
        )
        .0,
        ["ά"]
    );
    assert!(matches!(
        e.advance_syntax(
            &d,
            &mut Budget::new(WorkLimit {
                queries: 1000,
                objects: 1000,
                bytes: 1000,
                ..WorkLimit::default()
            })
        )
        .unwrap(),
        MathProgress::PendingInput
    ));
    let mut d = Document::new();
    d.append("\\[\nx").unwrap();
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    let mut paint = e
        .paint(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        })
        .unwrap();
    d.append("+y").unwrap();
    assert_eq!(
        paint.advance(&e, &d, &mut tiny()),
        Err(mdtui3::LayoutError::StaleContinuation)
    );
}

#[test]
fn both_fence_sides_have_independent_top_middle_bottom_grids() {
    // Explicit Unicode expectations, including each side independently and
    // invisible fences. Do not derive these strings from renderer tables.
    for (tex, expected) in [
        (r"\left(x\right)", vec!["(x)"]),
        (r"\left[x\right]", vec!["[x]"]),
        (r"\left|x\right|", vec!["│x│"]),
        (r"\left. x\right.", vec!["x"]),
        (r"\left(\frac{a}{b}\right)", vec!["⎛ a ⎞", "⎜───⎟", "⎝ b ⎠"]),
        (r"\left[\frac{a}{b}\right]", vec!["┌ a ┐", "│───│", "└ b ┘"]),
        (r"\left|\frac{a}{b}\right|", vec!["│ a │", "│───│", "│ b │"]),
        (r"\left.\frac{a}{b}\right.", vec![" a", "───", " b"]),
        (r"\left.\frac{a}{b}\right)", vec![" a ⎞", "───⎟", " b ⎠"]),
        (r"\left[\frac{a}{b}\right.", vec!["┌ a", "│───", "└ b"]),
        (r"\left(\sqrt{x}\right]", vec!["⎛ ─┐", "⎝√x┘"]),
    ] {
        let d = document(tex);
        let mut e = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::Ready, "{tex}");
        let m = e.metrics().unwrap();
        assert_eq!(
            draw(
                &e,
                &d,
                Rect {
                    x: 0,
                    y: 0,
                    width: m.width,
                    height: m.height
                }
            )
            .0,
            expected,
            "{tex}"
        );
    }
}

fn locate(e: &MathEngine, d: &Document, goal: Span) -> MathPosition {
    let mut task = e.locate_source(goal).unwrap();
    let before = task.steps();
    assert_eq!(
        task.advance(e, d, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        LocateStep::BudgetExhausted
    );
    assert_eq!(task.steps(), before);
    for _ in 0..d.source().len() * 10 + 100 {
        match task.advance(e, d, &mut tiny()).unwrap() {
            LocateStep::Position(p) => return p,
            LocateStep::Progress | LocateStep::BudgetExhausted => {}
        }
    }
    panic!("locator stalled")
}

#[test]
fn source_locator_uses_root_spans_across_nonmonotone_fraction_rows() {
    let d = document(r"z+\frac{ab}{cd}+x_1^2");
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    // Independent expected source offsets and coordinates: source order visits
    // the bar command BEFORE its visually preceding numerator.
    for (raw, row, column) in [
        (r"\frac", 1, 2),
        ("a", 0, 3),
        ("b", 0, 4),
        ("c", 2, 3),
        ("d", 2, 4),
        ("1", 2, 8),
        ("2", 0, 8),
    ] {
        let at = d.source().rfind(raw).unwrap();
        let p = locate(&e, &d, Span::new(at, at + raw.len()));
        assert_eq!((p.row, p.column), (row, column), "{raw}");
        assert_eq!(p.source, Span::new(at, at + raw.len()));
        assert!(p.cell_source.start <= at && p.cell_source.end >= at + raw.len());
    }
    let other = document(r"z+\frac{ab}{cd}+x_1^2");
    assert!(matches!(
        MathEngine::new(&other, first(&d)),
        Err(mdtui3::LayoutError::WrongDocument)
    ));
    let mut task = e.locate_source(Span::new(2, 3)).unwrap();
    assert_eq!(
        task.advance(&e, &other, &mut tiny()),
        Err(mdtui3::LayoutError::WrongDocument)
    );
    let invalid = document(r"x+\frac{a}");
    let mut e = MathEngine::new(&invalid, first(&invalid)).unwrap();
    assert_eq!(finish(&mut e, &invalid), MathProgress::Ready);
    let position = locate(&e, &invalid, Span::new(2, 3));
    assert_eq!(position.row, 0);
    assert_eq!(position.cell_source, first(&invalid).content);
}

#[test]
fn empty_invalid_unknown_and_narrow_windows_keep_complete_atom_sources() {
    for (tex, expected) in [
        ("", vec![""]),
        ("{}", vec![""]),
        (r"\begin{matrix}\end{matrix}", vec![""]),
        (r"\begin{matrix}&\\&\end{matrix}", vec!["", "", ""]),
        (r"\left.\right.", vec![""]),
        (r"\sqrt[2]{x}", vec!["[invalid]"]),
        (r"x^1^2", vec!["[invalid]"]),
        (r"\begin{matrix}a&b\\c\end{matrix}", vec!["[invalid]"]),
        (r"\begin{matrix}a\end{bmatrix}", vec!["[invalid]"]),
        (r"x}", vec!["[invalid]"]),
        (r"{x", vec!["[invalid]"]),
        (r"\unknown{x}", vec![r"\unknownx"]),
        (r"\epsilon\varepsilon\phi\varphi", vec!["ϵεϕφ"]),
    ] {
        let d = document(tex);
        let mut e = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::Ready, "{tex}");
        let m = e.metrics().unwrap();
        let (rows, cells) = draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: m.width.max(1),
                height: m.height,
            },
        );
        assert_eq!(rows, expected, "{tex}");
        if tex == r"\unknown{x}" {
            let command = Span::new(2, 10);
            assert!(cells.iter().take(8).all(|c| c.source == command));
            let (rows, cells) = draw(
                &e,
                &d,
                Rect {
                    x: 4,
                    y: 0,
                    width: 1,
                    height: 1,
                },
            );
            assert_eq!(rows, ["n"]);
            assert_eq!(cells[0].source, command);
        }
        let (_, cells) = draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: m.height,
            },
        );
        assert!(cells.is_empty());
    }
}

#[test]
fn partial_layout_and_paint_cancel_under_the_shared_budget() {
    let tex = format!("{}x{}", "\\sqrt{".repeat(4000), "}".repeat(4000));
    let d = document(&tex);
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    for _ in 0..1000 {
        e.advance(&d, &mut tiny()).unwrap();
    }
    let before = e.stats();
    assert!(before.syntax.max_frames > 1);
    let mut retirement = e.into_retirement();
    let steps = retirement.pub_steps;
    assert!(!retirement.advance(&mut Budget::new(WorkLimit::default())));
    assert_eq!(retirement.pub_steps, steps);
    while !retirement.advance(&mut tiny()) {}
    assert_eq!(retirement.stats().index_payload_bytes, 0);
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    let m = e.metrics().unwrap();
    let mut p = e
        .paint(Rect {
            x: m.width - 1,
            y: m.height - 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    for _ in 0..500 {
        p.advance(&e, &d, &mut tiny()).unwrap();
    }
    assert!(p.storage().pages > 0);
    let before = p.storage();
    assert!(!p.retire_step(&mut Budget::new(WorkLimit::default())));
    assert_eq!(before, p.storage());
    while !p.retire_step(&mut tiny()) {}
    assert_eq!(p.storage().pages, 0);
}

#[test]
fn online_one_byte_appends_preserve_the_parsed_prefix_and_paid_tail_window() {
    let mut d = Document::new();
    d.append("\\[\nx").unwrap();
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    let mut update = mdtui3::Update::default();
    let mut calls = 0;
    for n in 2..=16384 {
        d.append_into("x", &mut update).unwrap();
        let before = e.parsed_bytes();
        loop {
            calls += 1;
            assert!(calls < n * 20);
            match e.advance(&d, &mut tiny()).unwrap() {
                MathProgress::BudgetExhausted => {}
                MathProgress::PendingInput => break,
                other => panic!("unsealed real payload: {other:?}"),
            }
        }
        assert_eq!(e.parsed_bytes(), before + 1);
        assert_eq!(e.stats().syntax.bytes, n);
        assert_eq!(e.stats().payload_bytes, n);
    }
    let before = e.stats();
    assert_eq!(before.syntax.nodes, 16384);
    let mut p = e
        .paint(Rect {
            x: 16383,
            y: 0,
            width: 1,
            height: 1,
        })
        .unwrap();
    let mut cells = 0;
    loop {
        match p.advance(&e, &d, &mut tiny()).unwrap() {
            PaintStep::Cell(c) => {
                cells += 1;
                assert_eq!(c.source, Span::new(d.source().len() - 1, d.source().len()));
            }
            PaintStep::Complete => break,
            _ => {}
        }
    }
    assert_eq!(cells, 1);
    assert!(p.work().steps < 20);
    d.append("\n\\]\n").unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    assert_eq!(e.stats().syntax.bytes, before.syntax.bytes + 1);
    assert_eq!(e.metrics().unwrap().width, 16384);
}

#[test]
fn zero_byte_frames_still_charge_real_metadata_and_ink_continues_utf8_bytes() {
    let d = document(r"\alpha");
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    let no_bytes = || {
        Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        })
    };
    let mut descriptor = false;
    for _ in 0..20 {
        let mut b = no_bytes();
        assert_eq!(
            e.advance(&d, &mut b).unwrap(),
            MathProgress::BudgetExhausted
        );
        assert_eq!(b.used().queries, 1);
    }
    assert_eq!(e.parsed_bytes(), 0);
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    let (_, cells) = draw(
        &e,
        &d,
        Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    );
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].source, Span::new(2, 8));
    let mut ink = e.ink_cursor(cells[0].ink).unwrap();
    let mut out = [0; 4];
    let mut b = no_bytes();
    assert_eq!(
        ink.advance(&e, &d, &mut b, &mut out).unwrap(),
        InkReadStep::BudgetExhausted
    );
    assert_eq!(b.used().queries, 1);
    let one_byte = || {
        Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 1,
            ..WorkLimit::default()
        })
    };
    let mut bytes = Vec::new();
    for _ in 0..2 {
        assert_eq!(
            ink.advance(&e, &d, &mut one_byte(), &mut out).unwrap(),
            InkReadStep::Bytes(1)
        );
        bytes.push(out[0]);
    }
    assert_eq!(String::from_utf8(bytes).unwrap(), "α");
    let mut b = no_bytes();
    assert_eq!(
        ink.advance(&e, &d, &mut b, &mut out).unwrap(),
        InkReadStep::Complete
    );
    assert_eq!(b.used().queries, 1);
    let mut paint = e
        .paint(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        })
        .unwrap();
    for _ in 0..20 {
        let mut b = no_bytes();
        let status = paint.advance(&e, &d, &mut b).unwrap();
        descriptor |= matches!(status, PaintStep::Cell(_));
        assert_eq!(b.used().queries, 1);
    }
    // Painting a fixed indexed glyph descriptor emits no text bytes; its ink
    // reader above charges the actual output, including bytewise continuation.
    assert!(descriptor);
}

#[test]
fn raw_controls_follow_shared_filter_without_emitting_terminal_bytes() {
    for (tex, expected) in [
        ("x\u{1b}[31my", "x[31my"),
        ("x\u{7f}y", "xy"),
        ("\\\u{1b}", "\\"),
    ] {
        let d = document(tex);
        let mut e = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::Ready);
        assert_eq!(
            draw(
                &e,
                &d,
                Rect {
                    x: 0,
                    y: 0,
                    width: 12,
                    height: 1
                }
            )
            .0,
            [expected]
        );
    }
}

#[test]
fn post_tex_clusters_cross_transparent_groups_but_not_matrix_cells() {
    for (tex, expected, width) in [
        ("\\alpha{}\u{301}", "ά", 1),
        ("中+👩{}‍💻", "中+👩‍💻", 5),
        ("🇦{}🇧", "🇦🇧", 2),
        ("a{}\u{301}b", "áb", 2),
    ] {
        let d = document(tex);
        let mut e = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut e, &d), MathProgress::Ready, "{tex}");
        assert_eq!(e.metrics().unwrap().width, width, "{tex}");
        let (grid, cells) = draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 12,
                height: 1,
            },
        );
        assert_eq!(grid, [expected], "{tex}");
        if cells.len() == 1 {
            assert_eq!(
                cells[0].source,
                Span::new(2, 2 + tex.len()),
                "whole root source atom"
            );
        }
    }
    let d = document("\\begin{matrix}\\alpha&\u{301}\\end{matrix}");
    let mut e = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::Ready);
    let (_, cells) = draw(
        &e,
        &d,
        Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 1,
        },
    );
    assert_eq!(
        cells.iter().map(|c| (c.x, c.width)).collect::<Vec<_>>(),
        [(0, 1), (3, 0)]
    );
}

#[test]
fn ink_handles_reject_foreign_objects_even_when_logical_offsets_match() {
    let d = document(r"\alpha");
    let mut a = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut a, &d), MathProgress::Ready);
    let (_, cells) = draw(
        &a,
        &d,
        Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    );
    let foreign = document(r"\alpha");
    let mut e = MathEngine::new(&foreign, first(&foreign)).unwrap();
    assert_eq!(finish(&mut e, &foreign), MathProgress::Ready);
    assert!(matches!(
        e.ink_cursor(cells[0].ink),
        Err(mdtui3::LayoutError::WrongDocument)
    ));
    let mut d = Document::new();
    d.append(r"\(x\) \(y\)").unwrap();
    d.finish();
    let mut w = d.math_window(Span::new(0, d.source().len())).unwrap();
    let MathWindowStep::Object(one) = w.advance(&d, &mut tiny()).unwrap() else {
        panic!()
    };
    let MathWindowStep::Object(two) = w.advance(&d, &mut tiny()).unwrap() else {
        panic!()
    };
    let mut a = MathEngine::new(&d, one).unwrap();
    let mut b = MathEngine::new(&d, two).unwrap();
    assert_eq!(finish(&mut a, &d), MathProgress::Ready);
    assert_eq!(finish(&mut b, &d), MathProgress::Ready);
    let (_, cells) = draw(
        &a,
        &d,
        Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    );
    assert!(matches!(
        b.ink_cursor(cells[0].ink),
        Err(mdtui3::LayoutError::StaleContinuation)
    ));
}

#[test]
fn same_object_different_preparation_rejects_foreign_engine_before_budget_or_cache() {
    // Exact root public-API reproduction, including the partial-progress path.
    let mut d = Document::new();
    d.append(&format!(
        "\\({}\\)",
        "abcdefghijklmnopqrstuvwxyz".repeat(20)
    ))
    .unwrap();
    d.finish();
    let meta = first(&d);
    let mut a = MathEngine::new(&d, meta).unwrap();
    for _ in 0..10000 {
        let state = a.advance(&d, &mut tiny()).unwrap();
        if a.metrics().is_some() && state != MathProgress::Ready {
            break;
        }
    }
    assert_eq!(a.parsed_bytes(), 1);
    assert_eq!(a.metrics().unwrap().width, 0);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut local = a.paint(rect).unwrap();
    let mut foreign = a.paint(rect).unwrap();
    let mut locator = a.locate_source(Span::new(12, 13)).unwrap();
    let mut b = MathEngine::new(&d, meta).unwrap();
    assert_eq!(
        b.advance_syntax(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        MathProgress::Ready
    );
    assert_eq!(
        b.advance(&d, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        MathProgress::Ready
    );
    assert_eq!(b.metrics().unwrap().width, 520);
    for mut budget in [Budget::new(WorkLimit::default()), tiny()] {
        let storage = foreign.storage();
        let steps = foreign.work().steps;
        assert_eq!(
            foreign.advance(&b, &d, &mut budget),
            Err(mdtui3::LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(foreign.storage(), storage);
        assert_eq!(foreign.work().steps, steps);
        let steps = locator.steps();
        assert_eq!(
            locator.advance(&b, &d, &mut budget),
            Err(mdtui3::LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
        assert_eq!(locator.steps(), steps);
    }
    let mut complete = false;
    for _ in 0..10000 {
        match local.advance(&a, &d, &mut tiny()).unwrap() {
            PaintStep::Complete => {
                complete = true;
                break;
            }
            PaintStep::Cell(_) => panic!("partial root snapshot is empty"),
            _ => {}
        }
    }
    assert!(complete);
    let (_, cells) = draw(&b, &d, rect);
    assert_eq!(cells.len(), 8);
    assert_eq!(cells[0].source, Span::new(2, 3));
    assert!(matches!(
        a.ink_cursor(cells[0].ink),
        Err(mdtui3::LayoutError::StaleContinuation)
    ));
    let mut ink = b.ink_cursor(cells[0].ink).unwrap();
    let mut bytes = [99; 4];
    for mut budget in [Budget::new(WorkLimit::default()), tiny()] {
        assert_eq!(
            ink.advance(&a, &d, &mut budget, &mut bytes),
            Err(mdtui3::LayoutError::StaleContinuation)
        );
        assert_eq!(bytes, [99; 4]);
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(
        ink.advance(&b, &d, &mut tiny(), &mut bytes).unwrap(),
        InkReadStep::Bytes(1)
    );
    assert_eq!(bytes[0], b'a');
    assert!(matches!(
        locator.advance(&a, &d, &mut tiny()).unwrap(),
        LocateStep::Progress | LocateStep::Position(_)
    ));
    for engine in [a, b] {
        let mut retire = engine.into_retirement();
        assert!(!retire.advance(&mut Budget::new(WorkLimit::default())));
        while !retire.advance(&mut tiny()) {}
        assert_eq!(retire.stats().index_entries, 0);
        assert_eq!(retire.stats().payload.pages, 0);
    }
}

#[test]
fn frozen_source_cut_keeps_a_pending_prefix_and_foreign_cached_owner_rejects_first() {
    let d = document(r"x_1+\alpha");
    let meta = first(&d);
    let mut e = MathEngine::new_at_source_cut(&d, meta, meta.content.start + 1).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    assert_eq!(
        draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 8,
                height: 1
            }
        )
        .0,
        ["x"]
    );
    let shared = SharedMath::new(&d);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 3,
    };
    let mut a = shared.query(&d, meta, rect).unwrap();
    while !a.advance(&d, &mut tiny()).unwrap() {}
    assert!(a.output_storage().old_records_relocated == 0);
    let mut b = shared
        .query_at_source_cut(&d, meta, rect, Some(meta.content.start + 1))
        .unwrap();
    while !b.advance(&d, &mut tiny()).unwrap() {}
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(a.advance(&d, &mut zero), Ok(true));
    assert_eq!(zero.used(), WorkLimit::default());
    let mut c = shared
        .query_at_source_cut(&d, meta, rect, Some(meta.content.start + 2))
        .unwrap();
    while !c.advance(&d, &mut tiny()).unwrap() {}
    for mut micro in [Budget::new(WorkLimit::default()), tiny()] {
        assert_eq!(
            b.advance(&d, &mut micro),
            Err(mdtui3::LayoutError::StaleContinuation)
        );
        assert_eq!(micro.used(), WorkLimit::default());
    }
    assert_eq!(shared.retained_owners(), 2);
    drop(a);
    drop(b);
    drop(c);
    for _ in 0..2000 {
        if shared.retire(&mut tiny()) {
            break;
        }
    }
    assert_eq!(shared.retirement_jobs(), 0);
}
#[test]
fn literal_post_tex_clusters_use_the_current_shared_tui_oracle() {
    use mdtui_tui::render::text::{GraphemeKind, graphemes};
    for text in [
        "中á",
        "👩‍💻",
        "🇦🇧🇨",
        "क्\u{93c}क",
        "\u{17d8}",
        "\u{301}",
        "a\0\u{301}",
    ] {
        let d = document(text);
        let mut engine = MathEngine::new(&d, first(&d)).unwrap();
        assert_eq!(finish(&mut engine, &d), MathProgress::Ready);
        let expected: Vec<_> = graphemes(text)
            .filter(|g| g.kind != GraphemeKind::Control)
            .map(|g| (g.width, Span::new(g.start + 2, g.end + 2)))
            .collect();
        let (_, cells) = draw(
            &engine,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 1,
            },
        );
        assert_eq!(
            cells
                .iter()
                .map(|c| (c.width, c.source))
                .collect::<Vec<_>>(),
            expected,
            "{text:?}"
        );
        assert_eq!(
            engine.metrics().unwrap().width,
            expected.iter().map(|(w, _)| usize::from(*w)).sum::<usize>()
        );
    }
}
#[test]
fn a_live_cut_at_the_current_payload_end_never_adopts_a_future_suffix() {
    let mut d = Document::new();
    d.append("\\[\nx").unwrap();
    let meta = first(&d);
    let mut e = MathEngine::new_at_source_cut(&d, meta, meta.content.end).unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    let before = e.stats().source_bytes;
    d.append("_1\n\\]\n").unwrap();
    assert_eq!(finish(&mut e, &d), MathProgress::PendingInput);
    assert_eq!(e.stats().source_bytes, before);
    assert_eq!(
        draw(
            &e,
            &d,
            Rect {
                x: 0,
                y: 0,
                width: 8,
                height: 1
            }
        )
        .0,
        ["x"]
    );
}

#[test]
fn current_and_frozen_queries_progress_when_the_same_budget_is_interleaved() {
    let d = document(&format!("{}+\\alpha", "x".repeat(80)));
    let meta = first(&d);
    let shared = SharedMath::new(&d);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut current = shared.query(&d, meta, rect).unwrap();
    let mut frozen = shared
        .query_at_source_cut(&d, meta, rect, Some(meta.content.start + 40))
        .unwrap();
    let mut a = false;
    let mut b = false;
    for _ in 0..10000 {
        if !a {
            a = current.advance(&d, &mut tiny()).unwrap();
        }
        if !b {
            b = frozen.advance(&d, &mut tiny()).unwrap();
        }
        if a && b {
            break;
        }
    }
    assert!(a && b);
    assert_eq!(current.result().unwrap().metrics.width, 82);
    assert_eq!(frozen.result().unwrap().metrics.width, 40);
    assert_eq!(shared.retained_owners(), 2);
}

#[test]
fn long_orphan_cluster_is_visible_and_window_skips_suppressed_nodes() {
    let marks = "\u{301}".repeat(10_000);
    let d = document(&marks);
    let mut engine = MathEngine::new(&d, first(&d)).unwrap();
    assert_eq!(finish(&mut engine, &d), MathProgress::Ready);
    assert_eq!(engine.metrics().unwrap().width, 0);
    assert!(engine.has_ink());
    let mut paint = engine
        .paint(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        })
        .unwrap();
    let mut cells = 0;
    let mut calls = 0;
    loop {
        calls += 1;
        match paint.advance(&engine, &d, &mut tiny()).unwrap() {
            PaintStep::Cell(cell) => {
                assert_eq!(cell.width, 0);
                assert_eq!(cell.source, Span::new(2, 2 + marks.len()));
                cells += 1;
            }
            PaintStep::Complete => break,
            _ => {}
        }
        assert!(
            calls < 40,
            "window query must not scan suppressed scalar nodes"
        );
    }
    assert_eq!(cells, 1);
    let empty = document("{}");
    let mut e = MathEngine::new(&empty, first(&empty)).unwrap();
    assert_eq!(finish(&mut e, &empty), MathProgress::Ready);
    assert!(!e.has_ink());
}

#[test]
fn withdrawal_invalidates_readers_from_a_retired_frozen_context_before_budget() {
    let mut d = Document::new();
    d.append("\\(old\n").unwrap();
    // The same early-published candidate as the native late-header oracle.
    d.append(&format!("close\\) {} *s* | \\(cell\\)\n", "a".repeat(2000)))
        .unwrap();
    let meta = first(&d);
    let shared = SharedMath::new(&d);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 1,
    };
    let mut a = shared
        .query_at_source_cut(&d, meta, rect, Some(meta.content.start + 1))
        .unwrap();
    while !a.advance(&d, &mut tiny()).unwrap() {}
    let payload = a
        .result()
        .unwrap()
        .cells
        .iter()
        .next()
        .unwrap()
        .text
        .clone();
    let mut b = shared
        .query_at_source_cut(&d, meta, rect, Some(meta.content.start + 2))
        .unwrap();
    while !b.advance(&d, &mut tiny()).unwrap() {}
    let update = d.append("--- | ---\n").unwrap();
    assert!(d.math_object(meta.id).is_none());
    shared.apply(&d, &update).unwrap();
    let mut reader = payload.reader();
    for mut budget in [Budget::new(WorkLimit::default()), tiny()] {
        assert!(matches!(
            reader.next(&mut budget),
            Err(mdtui3::LayoutError::StaleContinuation)
        ));
        assert_eq!(budget.used(), WorkLimit::default());
    }
}

#[test]
fn glyph_focus_rejects_foreign_owners_before_budget_and_keeps_whole_command_source() {
    let d = document(r"\sin");
    let meta = first(&d);
    let a = SharedMath::new(&d);
    let b = SharedMath::new(&d);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 3,
        height: 1,
    };
    let mut qa = a.query(&d, meta, rect).unwrap();
    let mut qb = b.query(&d, meta, rect).unwrap();
    while !qa.advance(&d, &mut tiny()).unwrap() {}
    while !qb.advance(&d, &mut tiny()).unwrap() {}
    let view = qa.result().unwrap();
    let focus = view.focus_at(2, 0).unwrap();
    assert_eq!(focus.source(), Span::new(2, 6));
    assert_eq!((focus.column(), focus.row()), (2, 0));
    for mut budget in [Budget::new(WorkLimit::default()), tiny()] {
        assert_eq!(
            b.set_focus(&d, focus, &mut budget),
            Err(mdtui3::LayoutError::StaleContinuation)
        );
        assert_eq!(budget.used(), WorkLimit::default());
    }
    assert_eq!(
        a.set_focus(&d, focus, &mut Budget::new(WorkLimit::default())),
        Ok(false)
    );
    assert_eq!(a.set_focus(&d, focus, &mut tiny()), Ok(true));
    let mut q = a
        .query(
            &d,
            meta,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        )
        .unwrap()
        .with_source(Some(meta.content.start));
    while !q.advance(&d, &mut tiny()).unwrap() {}
    let v = q.result().unwrap();
    assert_eq!(v.rect.x, 2);
    assert_eq!(
        v.cells
            .iter()
            .map(|c| c.text.collect_text())
            .collect::<String>(),
        "n"
    );
    assert!(v.cells.iter().all(|c| c.source == Span::new(2, 6)));
}

#[test]
fn row_major_projection_bounds_output_pages_and_keeps_a_wide_glyph_across_tiles() {
    let literal = format!("{}中\\alpha", "x".repeat(255));
    let d = document(&literal);
    let shared = SharedMath::new(&d);
    let mut cursor = MathTextCursor::new(&d, shared, first(&d)).unwrap();
    assert_eq!(cursor.output_storage().pages, 0);
    assert!(matches!(
        cursor
            .advance(&d, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        MathTextStep::BudgetExhausted
    ));
    assert_eq!(cursor.output_storage().pages, 0);
    let mut text = String::new();
    for _ in 0..100000 {
        let mut budget = tiny();
        let step = cursor.advance(&d, &mut budget).unwrap();
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
        assert!(cursor.output_storage().records <= 257);
        match step {
            MathTextStep::Cell { cell, .. } => text.push_str(&cell.text.collect_text()),
            MathTextStep::Space(_) => text.push(' '),
            MathTextStep::Newline(_) => text.push('\n'),
            MathTextStep::Complete => {
                assert_eq!(text, format!("{}中α\n", "x".repeat(255)));
                return;
            }
            MathTextStep::BudgetExhausted => {}
        }
    }
    panic!("bounded projection stalled");
}
