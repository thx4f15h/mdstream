use mdtui3::{Budget, Completion, ContainerId, Document, WorkLimit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    for chunk in [
        "> - [x] first\n",
        ">   > nested\n",
        ">   continued\n",
        "> - second\n",
        "outside",
        "\n",
    ] {
        let update = doc.append(chunk)?;
        println!(
            "new={:?} closed={:?} parsed_end={}",
            update.new_containers, update.closed_containers, update.container_end
        );
    }
    let end = doc.finish();
    println!("EOF closed={:?}", end.closed_containers);
    let mut ancestors = doc.container_ancestors(ContainerId(3)).unwrap();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 2,
            objects: 2,
            ..WorkLimit::default()
        });
        loop {
            match ancestors.advance(&mut budget) {
                Ok(Some((id, container))) => println!("{id:?}: {container:?}"),
                Ok(None) => {
                    println!(
                        "{} containers, {frames} ancestry frames; {:?}",
                        doc.container_count(),
                        doc.work()
                    );
                    return Ok(());
                }
                Err(Completion::BudgetExhausted) => break,
                Err(other) => panic!("unexpected completion: {other:?}"),
            }
        }
    }
}
