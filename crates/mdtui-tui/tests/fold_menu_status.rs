use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    fold_ui::FoldUi,
    render::{
        frame::{FrameGeometry, FrameRuntime},
        text::{display_width, fit_prefix},
    },
};
use mdtui3::{Budget, FoldId, HeadingId, WorkLimit};

fn menu(page: usize, index: usize) -> FoldUi {
    let mut ui = FoldUi::default();
    ui.open(0, None, Some(FoldId::Heading(HeadingId(0))));
    for _ in 0..page {
        assert!(ui.key(KeyCode::Right));
    }
    for _ in 0..index {
        assert!(ui.key(KeyCode::Down));
    }
    ui
}

#[test]
fn narrow_default_identity_has_independent_exact_status_oracles() {
    for (index, level) in [(0, "h1"), (1, "h2")] {
        let ui = menu(5, index);
        assert_eq!(ui.status(20).unwrap(), format!("{level} default ↑↓ ↵ Esc"));
        assert_eq!(
            ui.status(40).unwrap(),
            format!("default toggle {level} (only new) Enter Esc")
        );
    }
}

#[test]
fn all_six_pages_keep_every_action_distinct_and_fit_real_cells() {
    // Counts and identifying words come from the user-facing command contract.
    for (page, count) in [(0, 11), (1, 3), (2, 15), (3, 12), (4, 10), (5, 21)] {
        for width in [20, 40] {
            let mut seen = std::collections::BTreeSet::new();
            for index in 0..count {
                let ui = menu(page, index);
                let status = ui.status(width).unwrap();
                assert!(display_width(&status) <= usize::from(width), "{status}");
                assert!(status.contains("Esc"), "{page}/{index}: {status}");
                assert!(seen.insert(status.clone()), "{page}/{index}: {status}");
                for tiny in 0..20 {
                    let short = ui.status(tiny).unwrap();
                    assert!(
                        display_width(&short) <= usize::from(tiny),
                        "{tiny}: {short}"
                    );
                    assert_eq!(fit_prefix(&short, usize::from(tiny)).end, short.len());
                }
            }
        }
    }
    for (index, identity) in [
        (0, "h1"),
        (1, "h2"),
        (2, "h3"),
        (3, "h4"),
        (4, "h5"),
        (5, "h6"),
        (6, "code"),
        (7, "math"),
        (8, "table"),
        (12, "done"),
        (13, "todo"),
        (14, "depth>=2"),
        (15, "para>=4K"),
        (16, "0"),
        (17, "L1"),
        (18, "L3"),
        (19, "D1"),
        (20, "D3"),
    ] {
        assert!(menu(5, index).status(20).unwrap().contains(identity));
    }
    for (index, identity) in [(10, "0"), (11, "L1"), (12, "L3"), (13, "D1"), (14, "D3")] {
        assert!(menu(2, index).status(20).unwrap().contains(identity));
    }
    for (index, identity) in [
        (0, "source next here"),
        (1, "source next any"),
        (2, "table rows here"),
        (8, "table rows fixed"),
    ] {
        assert!(menu(2, index).status(20).unwrap().starts_with(identity));
    }
}

fn draw(runtime: &mut FrameRuntime, parser: &mut vt100::Parser) -> String {
    let mut output = Vec::new();
    for _ in 0..10_000 {
        let mut budget = Budget::new(WorkLimit {
            bytes: 6,
            queries: 2,
            objects: 2,
            ..mdtui_tui::stream::FRAME_WORK
        });
        let commit = runtime.frame_to(&mut output, &mut budget).unwrap();
        assert!(budget.used().bytes <= 6);
        assert!(budget.used().queries <= 2);
        assert!(budget.used().objects <= 2);
        if commit.committed && !runtime.fold_pending() {
            parser.process(&output);
            return parser.screen().contents();
        }
    }
    panic!("menu frame did not settle");
}

#[test]
fn actual_grid_and_defaults_survive_navigation_resize_cancel_and_zero_budget() {
    let mut runtime = FrameRuntime::from_finished_source(
        "# Heading\nREADY\n",
        FrameGeometry {
            rows: 6,
            cols: 20,
            content_width: 20,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut parser = vt100::Parser::new(6.try_into().unwrap(), 20.try_into().unwrap(), 0);
    draw(&mut runtime, &mut parser);
    runtime
        .reading_key(KeyCode::Char('z'), KeyModifiers::NONE, true)
        .unwrap();
    draw(&mut runtime, &mut parser);
    for _ in 0..5 {
        assert!(runtime.fold_menu_key(KeyCode::Right));
    }
    assert!(runtime.fold_menu_key(KeyCode::Home));
    let h1 = draw(&mut runtime, &mut parser);
    assert_eq!(h1, "Heading\n\nREADY\n\n\nh1 default ↑↓ ↵ Esc");
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    draw(&mut runtime, &mut parser);
    assert_eq!(
        runtime.viewport().folds().defaults().heading_levels,
        [true, false, false, false, false, false]
    );
    assert!(runtime.fold_menu_key(KeyCode::Down));
    assert_eq!(
        draw(&mut runtime, &mut parser),
        "Heading\n\nREADY\n\n\nh2 default ↑↓ ↵ Esc"
    );
    for width in [40, 80, 20] {
        runtime.resize(6, width).unwrap();
        parser = vt100::Parser::new(6.try_into().unwrap(), width.try_into().unwrap(), 0);
        let grid = draw(&mut runtime, &mut parser);
        assert!(grid.lines().last().unwrap().contains("h2"), "{grid}");
        assert!(
            grid.lines()
                .all(|line| display_width(line) <= usize::from(width))
        );
    }
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    draw(&mut runtime, &mut parser);
    assert_eq!(
        runtime.viewport().folds().defaults().heading_levels,
        [true, true, false, false, false, false]
    );
    assert!(runtime.fold_menu_key(KeyCode::Up));
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    let mut zero = Budget::new(WorkLimit::default());
    runtime.frame_to(&mut Vec::new(), &mut zero).unwrap();
    assert_eq!(zero.used(), WorkLimit::default());
    assert!(runtime.fold_pending());
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    draw(&mut runtime, &mut parser);
    assert_eq!(
        runtime.viewport().folds().defaults().heading_levels,
        [true, true, false, false, false, false]
    );
    assert!(!runtime.fold_busy());
    runtime.open_fold_menu(false);
    draw(&mut runtime, &mut parser);
    assert!(runtime.fold_menu_key(KeyCode::Left));
    assert!(runtime.fold_menu_key(KeyCode::Enter));
    draw(&mut runtime, &mut parser);
    assert_eq!(
        runtime.viewport().folds().defaults().heading_levels,
        [false, true, false, false, false, false]
    );
    assert!(runtime.fold_menu_key(KeyCode::Esc));
    draw(&mut runtime, &mut parser);
}

#[test]
fn six_page_grids_at_minimum_height_keep_actions_and_home_end_page_navigation() {
    for width in [20, 40] {
        let mut runtime = FrameRuntime::from_finished_source(
            "# Heading\nREADY\n",
            FrameGeometry {
                rows: 3,
                cols: width,
                content_width: width,
            },
            LinkMode::Default,
        )
        .unwrap();
        let mut parser = vt100::Parser::new(3.try_into().unwrap(), width.try_into().unwrap(), 0);
        draw(&mut runtime, &mut parser);
        runtime.open_fold_menu(false);
        draw(&mut runtime, &mut parser);
        for count in [11, 3, 15, 12, 10, 21] {
            let mut seen = std::collections::BTreeSet::new();
            let mut first = String::new();
            let mut last = String::new();
            for index in 0..count {
                let grid = draw(&mut runtime, &mut parser);
                assert!(grid.lines().count() <= 3, "{grid}");
                assert!(
                    grid.lines()
                        .all(|line| display_width(line) <= usize::from(width))
                );
                let status = grid.lines().last().unwrap().trim_end().to_owned();
                assert!(status.contains("Esc"), "{grid}");
                assert!(seen.insert(status.clone()), "{grid}");
                if index == 0 {
                    first = status.clone();
                }
                last = status;
                assert!(runtime.fold_menu_key(KeyCode::Down));
            }
            assert!(runtime.fold_menu_key(KeyCode::Home));
            let home = draw(&mut runtime, &mut parser);
            assert_eq!(home.lines().last().unwrap().trim_end(), first, "{home}");
            assert!(runtime.fold_menu_key(KeyCode::End));
            let end = draw(&mut runtime, &mut parser);
            assert_eq!(end.lines().last().unwrap().trim_end(), last, "{end}");
            assert!(runtime.fold_menu_key(KeyCode::Right));
        }
        assert!(draw(&mut runtime, &mut parser).contains("toggle"));
        assert!(runtime.fold_menu_key(KeyCode::Left));
        assert!(draw(&mut runtime, &mut parser).contains("h1"));
        assert!(runtime.fold_menu_key(KeyCode::Esc));
        draw(&mut runtime, &mut parser);
        assert!(!runtime.fold_busy());
    }
}

#[test]
fn narrow_d1_executes_the_selected_source_policy_and_errors_precede_labels() {
    for width in [20, 40] {
        let mut runtime = FrameRuntime::from_finished_source(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\n",
            FrameGeometry {
                rows: 6,
                cols: width,
                content_width: width,
            },
            LinkMode::Default,
        )
        .unwrap();
        let mut parser = vt100::Parser::new(6.try_into().unwrap(), width.try_into().unwrap(), 0);
        draw(&mut runtime, &mut parser);
        runtime.open_fold_menu(false);
        draw(&mut runtime, &mut parser);
        for _ in 0..2 {
            assert!(runtime.fold_menu_key(KeyCode::Right));
        }
        // Fixed selection without a selection must show its actual error.
        for _ in 0..3 {
            assert!(runtime.fold_menu_key(KeyCode::Down));
        }
        assert!(runtime.fold_menu_key(KeyCode::Enter));
        let error = draw(&mut runtime, &mut parser);
        assert!(
            error.lines().last().unwrap().starts_with("InvalidRange"),
            "{error}"
        );
        assert!(runtime.fold_menu_key(KeyCode::End));
        assert!(runtime.fold_menu_key(KeyCode::Up));
        let selected = draw(&mut runtime, &mut parser);
        assert!(
            selected.lines().last().unwrap().contains("D1"),
            "{selected}"
        );
        assert!(runtime.fold_menu_key(KeyCode::Enter));
        draw(&mut runtime, &mut parser);
        let candidate = runtime
            .viewport()
            .folds()
            .source_fold_at(runtime.document(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.hidden.start, usize::from(width));
        assert!(runtime.viewport().folds().is_collapsed(candidate.id));
        assert!(runtime.fold_menu_key(KeyCode::Esc));
        draw(&mut runtime, &mut parser);
        assert!(runtime.viewport().folds().is_collapsed(candidate.id));
        assert!(!runtime.fold_busy());
    }
}

#[test]
fn following_does_not_prepend_help_over_the_selected_fold_action() {
    for width in [20, 40, 80] {
        let mut runtime = FrameRuntime::from_finished_source(
            "# Heading\nREADY\n",
            FrameGeometry {
                rows: 6,
                cols: width,
                content_width: width,
            },
            LinkMode::Default,
        )
        .unwrap();
        let mut parser = vt100::Parser::new(6.try_into().unwrap(), width.try_into().unwrap(), 0);
        draw(&mut runtime, &mut parser);
        runtime.seek_end(true).unwrap();
        draw(&mut runtime, &mut parser);
        assert!(runtime.viewport().following());
        runtime.open_fold_menu(false);
        draw(&mut runtime, &mut parser);
        assert!(runtime.fold_menu_key(KeyCode::Left));
        let grid = draw(&mut runtime, &mut parser);
        let status = grid.lines().last().unwrap();
        assert!(status.contains("h1") && status.contains("Esc"), "{grid}");
        assert!(!status.contains("FOLLOW"), "{grid}");
        assert!(runtime.viewport().following());
        assert!(runtime.fold_menu_key(KeyCode::Esc));
        assert!(draw(&mut runtime, &mut parser).contains("FOLLOW"));
        assert!(runtime.viewport().following());
    }
}
