//! `cargo xtask live`: glave against a real model on small throwaway projects, graded from
//! the thread's events and the files left behind. Not part of `ci`: it needs a model and
//! costs money, so run it after changes the model will feel.
//!
//! The endpoint comes from the environment like any glave run (`GLAVE_BASE_URL`,
//! `GLAVE_API_KEY`); the model from `--model` or `GLAVE_MODEL`.

mod check;
mod run;
mod tasks;
mod trace;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::lock::{self, Mode};
use run::{Report, Setup};
use tasks::{SANDBOX_TASKS, TASKS};

const USAGE: &str = "usage: cargo xtask live [--model <id>] [--only <task>] [--repeat <n>]";

/// What to run.
struct Options {
    model: String,
    /// Run only the tasks whose name contains this.
    only: Option<String>,
    /// Runs of each task. Models answer differently each time, so one run can mislead.
    repeat: u32,
}

/// Builds glave, runs every task (or those whose name contains `--only`), and reports.
pub fn run(args: &[String]) -> Result<(), String> {
    let Options {
        model,
        only,
        repeat,
    } = options(args)?;
    let _guard = lock::acquire(Mode::Shared)?;
    let root = crate::workspace::root()?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), |dir| root.join(PathBuf::from(dir)));
    crate::cmd::run("cargo", &["build", "--release", "-p", "glave"])?;
    // Outside any repository, so the projects are not mistaken for part of glave's workspace
    // and the model sees nothing but the project.
    let base = std::env::temp_dir().join("glave-live");
    let setup = Setup {
        binary: target
            .join("release")
            .join(format!("glave{}", std::env::consts::EXE_SUFFIX)),
        model,
        cargo_target: base.join("cargo-target"),
        timeout: Duration::from_secs(300),
        api_key: std::env::var("GLAVE_API_KEY")
            .ok()
            .filter(|key| key.len() >= 8),
    };
    let tasks: Vec<_> = TASKS
        .iter()
        .chain(SANDBOX_TASKS.iter().filter(|_| glave_sandbox_available()))
        .filter(|task| only.as_deref().is_none_or(|only| task.name.contains(only)))
        .collect();
    if !glave_sandbox_available() {
        println!("(the sandbox tasks need the macOS sandbox, so they are skipped here)");
    }
    if tasks.is_empty() {
        return Err(format!("no task matches {only:?}"));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("the clock is before 1970: {err}"))?
        .as_secs();
    let dir = base.join(stamp.to_string());
    println!(
        "\n== live: {} · {} tasks · {}",
        setup.model,
        tasks.len(),
        dir.display()
    );
    println!(
        "{:<16} {:<6} {:>6} {:>6} {:>9} {:>9} {:>7}",
        "task", "result", "calls", "tools", "tokens", "cost", "time"
    );
    let mut reports = Vec::new();
    for task in tasks {
        for attempt in 1..=repeat {
            let label = if repeat > 1 {
                format!("{}#{attempt}", task.name)
            } else {
                task.name.to_owned()
            };
            let report = run::task(&setup, task, &dir.join(&label))?;
            print_row(&label, &report);
            reports.push(report);
        }
    }
    summarize(&reports)
}

/// The sandbox tasks need the macOS sandbox; elsewhere they are left out, and say so.
fn glave_sandbox_available() -> bool {
    cfg!(target_os = "macos") && std::path::Path::new("/usr/bin/sandbox-exec").is_file()
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut model = std::env::var("GLAVE_MODEL").ok();
    let mut only = None;
    let mut repeat = 1;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().cloned().ok_or(USAGE);
        match arg.as_str() {
            "--model" => model = Some(value()?),
            "--only" => only = Some(value()?),
            "--repeat" => {
                repeat = value()?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("--repeat needs a whole number above 0")?;
            }
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    let model =
        model.ok_or_else(|| format!("no model: pass --model or set GLAVE_MODEL\n{USAGE}"))?;
    Ok(Options {
        model,
        only,
        repeat,
    })
}

fn print_row(label: &str, report: &Report) {
    let trace = &report.trace;
    println!(
        "{:<16} {:<6} {:>6} {:>6} {:>9} {:>9} {:>6.1}s",
        label,
        if report.failures.is_empty() {
            "pass"
        } else {
            "FAIL"
        },
        trace.model_calls,
        trace.tool_calls,
        trace.input_tokens.saturating_add(trace.output_tokens),
        dollars(trace.cost_micros),
        report.elapsed.as_secs_f64(),
    );
    for failure in &report.failures {
        println!("    {failure}");
    }
}

fn summarize(reports: &[Report]) -> Result<(), String> {
    let failed = reports.iter().filter(|r| !r.failures.is_empty()).count();
    let cost = reports
        .iter()
        .try_fold(0_i64, |total, r| total.checked_add(r.trace.cost_micros?));
    let cached: u64 = reports.iter().map(|r| r.trace.cached_tokens).sum();
    // Repeated runs of one task sit together, so each task's pass rate reads off in order.
    let mut names: Vec<&str> = reports.iter().map(|r| r.name).collect();
    names.dedup();
    for name in names {
        let runs = reports.iter().filter(|r| r.name == name).count();
        if runs > 1 {
            let passed = reports
                .iter()
                .filter(|r| r.name == name && r.failures.is_empty())
                .count();
            println!("{name:<16} {passed}/{runs} passed");
        }
    }
    println!(
        "\n{} of {} passed · {} cached tokens · {}",
        reports.len().saturating_sub(failed),
        reports.len(),
        cached,
        dollars(cost),
    );
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("{failed} live task(s) failed"))
    }
}

/// Micros as dollars to a hundredth of a cent, or a dash when a cost was not reported.
fn dollars(micros: Option<i64>) -> String {
    match micros {
        Some(micros) => format!(
            "${}.{:04}",
            micros / 1_000_000,
            (micros % 1_000_000).unsigned_abs() / 100
        ),
        None => "-".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costs_read_to_a_hundredth_of_a_cent() {
        assert_eq!(dollars(Some(7_731)), "$0.0077");
        assert_eq!(dollars(Some(1_234_500)), "$1.2345");
        assert_eq!(dollars(None), "-");
    }

    #[test]
    fn task_names_are_unique() {
        let all: Vec<_> = TASKS.iter().chain(SANDBOX_TASKS).collect();
        let mut names: Vec<_> = all.iter().map(|task| task.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}
