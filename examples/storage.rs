//! Capacity and cache lifecycle measurements, see docs/storage.md.
#[path = "../tests/support/storage.rs"]
#[allow(dead_code)]
mod storage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: storage [base_bytes=4096] [queries=64]".into());
    }
    let base = args.first().map_or(Ok(4096), |s| s.parse::<usize>())?;
    let queries = args.get(1).map_or(Ok(64), |s| s.parse::<usize>())?;
    if !(1024..=262_144).contains(&base) || !(1..=10_000).contains(&queries) {
        return Err("base_bytes must be 1024..=262144; queries must be 1..=10000".into());
    }
    println!("schema,case,scale,phase,metric,value");
    for size in [base, base * 4, base * 16] {
        for sample in storage::measure(size, queries) {
            for (key, value) in sample.metrics {
                println!(
                    "1,{},{},{},{},{}",
                    sample.case, sample.scale, sample.phase, key, value
                );
            }
            println!(
                "1,{},{},{},elapsed_ns,{}",
                sample.case,
                sample.scale,
                sample.phase,
                sample.elapsed.as_nanos()
            );
        }
    }
    Ok(())
}
