//! Command-line dispatch for `cargo xtask`.

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci [--all] [--target <triple>]
                    the local gate: format, secrets, the dependency rule, one spawn path,
                    lint, tests and what they leave behind, dependency audit, file sizes, and
                    the web checks, which need Node 22.22.3+ (GLAVE_SKIP_WEB=1 skips them).
                    Off CI it lints and tests only the crates a change since origin/main
                    reaches, and runs the web checks only when web/ or the API schema changed;
                    --all, and CI always, check everything. --target builds the lint, docs and
                    tests for another target than this machine's (CI's run on musl). Off CI,
                    tests run on half the cores (GLAVE_TEST_THREADS=n sets how many)
  deps              check the dependency rule: each crate depends only on the layers its own
                    may use (the layers are in xtask/src/deps.rs)
  file-sizes        enforce the source-file line limits
  secrets           fail on a secret-shaped literal in a tracked file (tests make their fakes at
                    run time)
  spawns            fail on a process started outside glave-process (crates/process), which
                    gives each one an owner
  schema            write specs/session.schema.json from the session API's Rust types, and the
                    web client's TypeScript types from it (`--check`: fail if either is out of
                    date)
  release-workflow  write .github/workflows/release.yml with dist (installed, the version
                    dist-workspace.toml names), where only the job that publishes can write
  bench [args]      run benchmarks under the machine-wide benchmark lock
  eval [args]       run the harness on the eval tasks against real models and grade the
                    results (see `cargo xtask eval --help`); the model endpoint comes from the
                    environment or from .env (`--env-file <path>` names another file)
  help              show this message";

/// Parses the process arguments, runs the command, and maps the result to an exit code.
pub(crate) fn main() -> ExitCode {
    if let Some(code) = crate::cmd::helper() {
        return code;
    }
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
            "ci" => crate::ci::run(rest),
            "deps" => crate::deps::check(),
            "file-sizes" => crate::file_sizes::check(),
            "secrets" => crate::secrets::check(),
            "spawns" => crate::spawns::check(),
            "schema" => crate::schema::run(rest),
            "release-workflow" => crate::release_workflow::run(rest),
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
