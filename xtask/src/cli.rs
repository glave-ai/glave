//! Command-line dispatch for `cargo xtask`.

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci            the full local gate: format, lint, tests, dependency audit, file sizes
  file-sizes    enforce the source-file line limits
  bench [args]  run benchmarks under the machine-wide benchmark lock
  eval [args]   run the harness on the eval tasks against real models and grade the results
                (see `cargo xtask eval --help`; needs the model endpoint in the environment)
  help          show this message";

/// Parses the process arguments, runs the command, and maps the result to an exit code.
pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.split_first() {
        Some((command, rest)) => match command.as_str() {
            "ci" => crate::ci::run(),
            "file-sizes" => crate::file_sizes::check(),
            "bench" => crate::bench::run(rest),
            "eval" => crate::eval::run(rest),
            "help" | "--help" | "-h" => {
                println!("{USAGE}");
                Ok(())
            }
            other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
        },
        None => Err(format!("missing command\n\n{USAGE}")),
    }
}
