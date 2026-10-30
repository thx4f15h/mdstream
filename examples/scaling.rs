//! Elapsed times are observations, never test thresholds.
#[path = "../tests/support/performance.rs"]
mod performance;

use performance::{CASES, measure};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: scaling [base_bytes=65536] [samples=31]".into());
    }
    let base = args.first().map_or(Ok(65_536), |s| s.parse::<usize>())?;
    let samples = args.get(1).map_or(Ok(31), |s| s.parse::<usize>())?;
    if !(1024..=4_194_304).contains(&base) || !(3..=10_000).contains(&samples) {
        return Err("base_bytes must be 1024..=4194304; samples must be 3..=10000".into());
    }
    let mut header = false;
    for size in [base, base * 2, base * 4] {
        for case in CASES {
            let columns = measure(case, size, samples).columns();
            if !header {
                println!(
                    "{}",
                    columns
                        .iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                header = true;
            }
            println!(
                "{}",
                columns
                    .into_iter()
                    .map(|(_, v)| v)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    Ok(())
}
