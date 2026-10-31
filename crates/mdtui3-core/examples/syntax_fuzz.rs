//! Fixed-seed generation, exact case replay and deterministic failure reduction.
#[path = "../tests/support/syntax_fuzz.rs"]
mod fuzz;

use fuzz::{Case, Fault, SEEDS, check, generate, shrink};
use std::{env, fs, path::PathBuf, process::ExitCode};

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut seed = SEEDS[0];
    let mut count = 48;
    let mut output = PathBuf::from("syntax-fuzz.case");
    let mut replay = None;
    let mut fault = Fault::None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!(
                "syntax_fuzz [--seed U64] [--cases N] [--inject none|lose-style|bad-range|split-crlf]\n            [--output FILE] [--replay FILE]\nFailure: writes FILE and FILE.min; exit 1. Invalid arguments/case/IO: exit 2.\nReplay uses the exact input, chunk ends and fault stored in FILE."
            );
            return Ok(true);
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--seed" => seed = value.parse()?,
            "--cases" => count = value.parse()?,
            "--output" => output = PathBuf::from(value),
            "--replay" => replay = Some(PathBuf::from(value)),
            "--inject" => fault = Fault::parse(&value)?,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    if count == 0 {
        return Err("--cases must be positive".into());
    }
    let cases: Box<dyn Iterator<Item = Case>> = if let Some(path) = replay {
        if fault != Fault::None {
            return Err("--replay uses its stored fault; omit --inject".into());
        }
        Box::new(std::iter::once(Case::decode(&fs::read_to_string(path)?)?))
    } else {
        Box::new((0..count).map(move |index| {
            let mut case = generate(seed, index);
            case.fault = fault;
            case
        }))
    };
    let mut passed = 0;
    for case in cases {
        if let Err(failure) = check(&case) {
            // Save the original before potentially expensive reduction.
            fs::write(&output, case.encode())?;
            eprintln!(
                "FAIL seed={} case={} fault={} property={} detail={}\noriginal={}",
                case.seed,
                case.index,
                case.fault.name(),
                failure.property,
                failure.detail,
                output.display()
            );
            let reduced = shrink(&case, &failure);
            let mut path = output.as_os_str().to_owned();
            path.push(".min");
            let path = PathBuf::from(path);
            fs::write(&path, reduced.encode())?;
            eprintln!(
                "reduced={} bytes {} -> {}, cuts {} -> {}\ninput={:?}",
                path.display(),
                case.source.len(),
                reduced.source.len(),
                case.cuts.len(),
                reduced.cuts.len(),
                reduced.source
            );
            return Ok(false);
        }
        passed += 1;
    }
    println!("{}: {passed} cases passed", fuzz::VERSION);
    Ok(true)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
