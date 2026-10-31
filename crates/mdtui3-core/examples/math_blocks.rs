//! Stream an independent math block, read its mapped payload and fold it.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    let mut view = FoldView::new(&doc, 1)?;
    view.set_defaults(FoldDefaults {
        math: true,
        ..Default::default()
    });
    let update = doc.append("> - $$\r\n")?;
    view.apply(&doc, &update)?;
    let id = BlockId(0);
    assert!(view.is_collapsed(FoldId::Math(id)));
    let mut update = doc.append(">   中 + \\alpha\r\n>   🙂 &amp;\r\n")?;
    update.merge(doc.append(">   $$\n")?)?;
    view.apply(&doc, &update)?;
    let BlockKind::Math {
        content,
        closed: true,
    } = doc.block(id).unwrap().kind
    else {
        panic!("expected closed math block");
    };
    assert_eq!(doc.blocks().len(), 1);
    assert!(doc.maths().is_empty());
    let mut cursor = doc.math_block_payload(id).unwrap();
    let mut text = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    'frames: loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 8,
            ..Default::default()
        });
        loop {
            match cursor.advance(&mut budget) {
                Ok(Some(chunk)) => {
                    println!("{:?} -> {:?}", chunk.source, chunk.text);
                    text.push_str(chunk.text);
                }
                Ok(None) => {
                    bytes += budget.used().bytes;
                    break 'frames;
                }
                Err(Completion::BudgetExhausted) => break,
                Err(other) => panic!("unexpected completion: {other:?}"),
            }
        }
        bytes += budget.used().bytes;
    }
    assert_eq!(text, "中 + \\alpha\r\n🙂 &amp;\r\n");
    assert_eq!(bytes, content.len());
    let target = doc.source().find('🙂').unwrap();
    let mut navigation = FoldNavigation::new(
        &doc,
        &view,
        &Navigator::new(),
        NavAction::Source(target),
        FoldNavigationPolicy::Reveal,
        10,
    )?;
    let mut navigation_frames = 0;
    loop {
        navigation_frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let progress = navigation.advance(&doc, &mut view, &mut budget)?;
        if progress.completion == Completion::Covered {
            assert_eq!(progress.target.unwrap().source, Some(target));
            break;
        }
    }
    assert!(!view.is_collapsed(FoldId::Math(id)));
    let update = doc.finish();
    view.apply(&doc, &update)?;
    println!(
        "{frames} payload frames / {bytes} source bytes; {navigation_frames} navigation frames"
    );
    Ok(())
}
