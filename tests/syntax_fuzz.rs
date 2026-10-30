#[path = "support/syntax_fuzz.rs"]
mod fuzz;

use fuzz::{Case, Fault, SEEDS, check, generate, shrink};

fn assert_case(case: &Case) {
    if let Err(failure) = check(case) {
        // nextest captures the complete portable case even without a writable
        // filesystem. The CLI also saves originals before it starts reducing.
        eprintln!("ORIGINAL CASE\n{}", case.encode());
        let reduced = shrink(case, &failure);
        panic!("{failure:?}\nREDUCED CASE\n{}", reduced.encode());
    }
}

#[test]
fn fixed_seed_grammar_corpus_checks_live_and_finished_semantics() {
    for seed in SEEDS {
        for index in 0..36 {
            assert_case(&generate(seed, index));
        }
    }
}

#[test]
fn generated_cases_have_stable_bytes_and_exact_portable_replay() {
    let mut hash = 0xcbf29ce484222325_u64;
    for seed in SEEDS {
        for index in 0..36 {
            let case = generate(seed, index);
            assert_eq!(case, generate(seed, index));
            assert_eq!(Case::decode(&case.encode()).unwrap(), case);
            for byte in case.encode().bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    // FNV-1a over all encoded cases, including schedules and persisted seeds.
    assert_eq!(hash, 8_310_356_521_655_359_848);
}

#[test]
fn case_file_preserves_crlf_unicode_empty_chunks_and_unterminated_input() {
    for source in ["", "\0中🙂\r\n\n\nlast\r", "#\nseed=0\n\nraw", "no newline"] {
        let case = Case {
            source: source.into(),
            cuts: vec![0, 0, source.len()],
            seed: u64::MAX,
            index: 12,
            fault: Fault::None,
        };
        let decoded = Case::decode(&case.encode()).unwrap();
        assert_eq!(decoded, case);
        assert_case(&decoded);
    }
    let case = Case {
        source: "中🙂".into(),
        cuts: vec![],
        seed: 0,
        index: 0,
        fault: Fault::None,
    };
    for cuts in [vec![1], vec![8], vec![3, 0]] {
        let invalid = Case {
            cuts,
            ..case.clone()
        };
        assert!(Case::decode(&invalid.encode()).is_err());
        assert_eq!(check(&invalid).unwrap_err().property, "case.format");
    }
    assert!(Case::decode(&case.encode().replacen("v1", "v2", 1)).is_err());
    assert!(Case::decode("bad").is_err());
}

fn assert_irreducible(case: &Case, property: &str) {
    for (start, c) in case.source.char_indices() {
        let end = start + c.len_utf8();
        let mut candidate = case.clone();
        candidate.source.replace_range(start..end, "");
        for cut in &mut candidate.cuts {
            *cut = if *cut <= start {
                *cut
            } else if *cut < end {
                start
            } else {
                *cut - c.len_utf8()
            };
        }
        assert!(!check(&candidate).is_err_and(|f| f.property == property));
    }
    for i in 0..case.cuts.len() {
        let mut candidate = case.clone();
        candidate.cuts.remove(i);
        assert!(!check(&candidate).is_err_and(|f| f.property == property));
    }
}

#[test]
fn injected_style_loss_is_detected_replayed_and_deterministically_reduced() {
    // Locate a mutation through the same fixed grammar corpus as normal fuzz.
    let case = (0..36)
        .map(|i| {
            let mut case = generate(SEEDS[0], i);
            case.fault = Fault::LoseStyle;
            case
        })
        .find(|c| check(c).is_err())
        .expect("generator must reach strong emphasis");
    let failure = check(&case).unwrap_err();
    assert_eq!(failure.property, "live.semantic.runs");
    assert_eq!(
        check(&Case::decode(&case.encode()).unwrap()),
        Err(failure.clone())
    );
    let a = shrink(&case, &failure);
    let b = shrink(&case, &failure);
    assert_eq!(a, b);
    assert!(a.source.len() < case.source.len());
    assert!(a.cuts.len() < case.cuts.len());
    assert_eq!(check(&a).unwrap_err().property, failure.property);
    assert_irreducible(&a, &failure.property);
    let mut clean = a.clone();
    clean.fault = Fault::None;
    assert_case(&clean);
    eprintln!(
        "style mutant: {} -> {} bytes; {} -> {} cuts; {:?}",
        case.source.len(),
        a.source.len(),
        case.cuts.len(),
        a.cuts.len(),
        a.source
    );
}

#[test]
fn injected_invalid_range_is_caught_by_invariants_and_reduced() {
    let mut case = generate(SEEDS[1], 2);
    case.fault = Fault::BadRange;
    let failure = check(&case).unwrap_err();
    assert_eq!(failure.property, "live.range.utf8");
    let reduced = shrink(&case, &failure);
    assert_eq!(reduced, shrink(&case, &failure));
    assert_eq!(Case::decode(&reduced.encode()).unwrap(), reduced);
    assert_eq!(check(&reduced).unwrap_err().property, failure.property);
    assert!(reduced.source.len() < case.source.len());
    assert_irreducible(&reduced, &failure.property);
    let mut clean = reduced.clone();
    clean.fault = Fault::None;
    assert_case(&clean);
    eprintln!(
        "range mutant: {} -> {} bytes; {:?}",
        case.source.len(),
        reduced.source.len(),
        reduced.source
    );
}

#[test]
fn chunk_sensitive_fault_keeps_required_crlf_boundary_after_reduction() {
    let source = "noise 中\r\n**strong**\nmore noise";
    let case = Case {
        source: source.into(),
        cuts: source.char_indices().map(|(i, _)| i).collect(),
        seed: 0,
        index: 0,
        fault: Fault::SplitCrLf,
    };
    let failure = check(&case).unwrap_err();
    assert_eq!(failure.property, "live.semantic.source");
    let reduced = shrink(&case, &failure);
    assert_eq!(reduced, shrink(&case, &failure));
    assert_eq!(reduced.source, "\r\n");
    assert_eq!(reduced.cuts, [1]);
    assert_eq!(
        check(&Case::decode(&reduced.encode()).unwrap())
            .unwrap_err()
            .property,
        failure.property
    );
    assert_irreducible(&reduced, &failure.property);
    let mut clean = reduced;
    clean.fault = Fault::None;
    assert_case(&clean);
}

#[test]
fn fixed_corpus_reaches_syntax_objects_and_live_classification() {
    use mdtui3::{BLOCK_LOOKAHEAD, BlockKind, Document};
    let mut counts = [0_usize; 11];
    let mut prefix_checks = 0;
    for seed in SEEDS {
        for index in 0..36 {
            let case = generate(seed, index);
            prefix_checks += case.cuts.len() + 1;
            counts[0] += usize::from(case.source.contains('\r'));
            counts[1] += usize::from(!case.source.is_ascii());
            counts[2] += usize::from(case.source.lines().any(|l| l.len() > BLOCK_LOOKAHEAD));
            let mut doc = Document::new();
            doc.append(&case.source).unwrap();
            doc.finish();
            counts[3] += doc.headings().entries().len();
            counts[4] += doc.tables().len();
            counts[5] += doc.container_count();
            counts[6] += doc.links().len();
            counts[7] += doc.maths().len();
            counts[8] += doc.footnote_references().len();
            counts[9] += doc
                .blocks()
                .iter()
                .filter(|b| matches!(b.kind, BlockKind::Code { .. }))
                .count();
            counts[10] += doc
                .blocks()
                .iter()
                .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
                .count();
        }
    }
    assert!(
        counts.iter().all(|&n| n > 0),
        "grammar failed to reach syntax families: {counts:?}"
    );
    eprintln!(
        "144 cases, {prefix_checks} live comparisons; CRLF/Unicode/long/heading/table/container/link/math/footnote/code/math-block counts: {counts:?}"
    );
}
