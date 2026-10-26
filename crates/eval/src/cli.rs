//! `glave-eval`: the command that runs the suite and writes the report.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the eval's command line: it reports in the terminal"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use glave_event::Money;
use glave_ports::{Clock as _, SystemClock};
use glave_process::Processes;
use glave_provider_openai::Config;

use crate::report::{self, Report};
use crate::run::Setup;
use crate::suite::{self, Plan};
use crate::task::{self, Task, Tier};

/// The models every change is measured on: one each from three labs, all cheap and quick.
pub const DEFAULT_MODELS: [&str; 3] = [
    "anthropic/claude-haiku-5.5",
    "openai/gpt-6-luna",
    "deepseek/deepseek-v4.1-flash",
];

const USAGE: &str = "\
usage: glave-eval [options]
       glave-eval judge [options]   score the action judge on evals/judge (see judge --help)

Runs glave's harness in-process on the task folders against real models and grades the results.
The endpoint comes from GLAVE_BASE_URL and GLAVE_API_KEY, which `cargo xtask eval` reads from
.env (or the file --env-file <path> names) when the environment has none.

options:
  --model <id>         a model to run (repeat for several; default: the three default models)
  --only <text>        only the tasks whose name contains this
  --tier <tier>        fast (default), hard or all
  --repeat <n>         attempts of each task on each model (default 1)
  --concurrency <n>    attempts at once (default 6)
  --max-cost <dollars> start nothing new once this much is spent (default 2)
  --baseline <file>    an earlier report.json to compare with
  --tasks <folder>     where the tasks are (default evals/tasks)
  --no-plan            keep update_plan out of every mode but Plan, as glave does when plans are off
  --subagents          let the model hand work to child threads with task
  --memory <on|off>    memory on or off for every task (default: each task's own [glave] memory)";

/// The parsed command line.
#[derive(Debug, PartialEq)]
struct Options {
    models: Vec<String>,
    only: Option<String>,
    tier: Option<Tier>,
    repeat: u32,
    concurrency: usize,
    max_cost_micros: i64,
    baseline: Option<PathBuf>,
    tasks: PathBuf,
    plans: bool,
    subagents: bool,
    memory: Option<bool>,
}

pub fn main() -> ExitCode {
    // As the lifeline's helper that ties the model's commands to the eval (`tied`).
    let whole: Vec<OsString> = std::env::args_os().collect();
    if let Some(code) = glave_process::lifeline_helper(&whole) {
        return code;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(("judge", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return crate::judging::main(rest);
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match options(&args).and_then(|options| run(&options)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("glave-eval: {message}");
            ExitCode::from(2)
        }
    }
}

/// The model endpoint, from GLAVE_BASE_URL and GLAVE_API_KEY.
pub(crate) fn endpoint() -> Result<Config, String> {
    let base_url = std::env::var("GLAVE_BASE_URL").map_err(|_| {
        "GLAVE_BASE_URL is not set: give the model endpoint as GLAVE_BASE_URL (and \
         GLAVE_API_KEY, if it needs a key) in .env, which `cargo xtask eval` reads, or in the \
         environment"
            .to_owned()
    })?;
    let api_key = std::env::var("GLAVE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    Config::for_url(base_url.clone(), api_key)
        .ok_or_else(|| format!("GLAVE_BASE_URL is not a URL: {base_url}"))
}

/// Runs the suite; `Ok(false)` when an attempt failed or was skipped.
fn run(options: &Options) -> Result<bool, String> {
    let endpoint = endpoint()?;
    let tasks = chosen(
        task::load_all(&options.tasks).map_err(|err| err.to_string())?,
        options,
    )?;
    let baseline = options.baseline.as_deref().map(read_report).transpose()?;
    // Outside any repository, so the workspaces are not mistaken for part of glave's own.
    let base = std::env::temp_dir().join("glave-eval");
    prune(&base, KEEP_RUNS);
    let started = SystemClock.now();
    let dir = base.join(started.strftime("%Y%m%d-%H%M%S").to_string());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start: {err}"))?;
    let processes = runtime.block_on(tied())?;
    let setup = Arc::new(Setup {
        endpoint,
        environment: glave_first(std::env::vars_os().collect())?,
        plans: options.plans,
        subagents: options.subagents,
        memory: options.memory,
        processes,
    });
    let attempts = options
        .repeat
        .saturating_mul(u32::try_from(tasks.len() * options.models.len()).unwrap_or(u32::MAX));
    println!(
        "glave-eval: {} tasks × {} models × {} = {attempts} attempts · {}",
        tasks.len(),
        options.models.len(),
        options.repeat,
        dir.display()
    );
    let plan = Plan {
        tasks,
        models: options.models.clone(),
        repeat: options.repeat,
        concurrency: options.concurrency,
        max_cost_micros: options.max_cost_micros,
        dir: dir.clone(),
    };
    let attempts = runtime.block_on(suite::run(Arc::clone(&setup), plan, |attempt| {
        println!("{}", report::line(attempt));
    }));
    let report = Report::new(
        started.to_string(),
        setup.endpoint.provider.clone(),
        options.models.clone(),
        attempts,
    );
    let table = report.markdown(baseline.as_ref());
    let json = serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?;
    write(&dir.join("report.json"), &json)?;
    write(&dir.join("report.md"), &table)?;
    println!("\n{table}\nreport: {}", dir.join("report.json").display());
    Ok(report.passed())
}

/// `environment` with the folder of the glave beside this program first on `PATH`, where
/// `cargo xtask eval` builds it: a task's `glave doc` is then the glave being measured, never
/// one installed on this machine.
fn glave_first(
    mut environment: Vec<(OsString, OsString)>,
) -> Result<Vec<(OsString, OsString)>, String> {
    let here =
        std::env::current_exe().map_err(|err| format!("could not find this program: {err}"))?;
    let folder = here.parent().unwrap_or(Path::new("."));
    if !folder.join("glave").is_file() {
        return Err(format!(
            "no glave beside {}: cargo build -p glave (cargo xtask eval builds it)",
            here.display()
        ));
    }
    let path = environment
        .iter()
        .position(|(name, _)| name == "PATH")
        .map(|at| environment.swap_remove(at).1);
    let mut folders = vec![folder.to_path_buf()];
    folders.extend(path.iter().flat_map(std::env::split_paths));
    let path =
        std::env::join_paths(folders).map_err(|err| format!("could not make PATH: {err}"))?;
    environment.push(("PATH".into(), path));
    Ok(environment)
}

/// How the model's commands start: tied to this eval by glave's lifeline, with this program
/// as its helper (`main` runs it), so they end when the eval does, however it ends.
async fn tied() -> Result<Processes, String> {
    let helper = glave_process::this_program()
        .map_err(|err| format!("could not find this program to start its helper: {err}"))?;
    Processes::unrecorded()
        .with_lifeline(helper)
        .await
        .map_err(|err| err.to_string())
}

/// Runs kept on disk. Each holds a workspace per attempt, so old ones go.
const KEEP_RUNS: usize = 10;

/// Removes all but the newest `keep` run folders under `base`. Only folders named like a run
/// (`20261008-182952`) are touched.
fn prune(base: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let mut runs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_run)
        })
        .collect();
    runs.sort();
    let old = runs.len().saturating_sub(keep);
    for run in runs.iter().take(old) {
        // Best effort: a run that cannot be removed only costs disk space.
        let _ = std::fs::remove_dir_all(run);
    }
}

fn is_run(name: &str) -> bool {
    name.len() == 15
        && name.bytes().enumerate().all(|(i, b)| {
            if i == 8 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

/// The tasks to run: in the tier, matching `--only`, and runnable on this computer.
fn chosen(tasks: Vec<Task>, options: &Options) -> Result<Vec<Task>, String> {
    let sandbox = glave_computer::sandbox_available();
    let mut left_out = 0;
    let tasks: Vec<Task> = tasks
        .into_iter()
        .filter(|task| options.tier.is_none_or(|tier| task.tier == tier))
        .filter(|task| {
            options
                .only
                .as_deref()
                .is_none_or(|only| task.name.contains(only))
        })
        .filter(|task| {
            let runnable = sandbox || !task.sandboxed;
            left_out += usize::from(!runnable);
            runnable
        })
        .collect();
    if left_out > 0 {
        println!("({left_out} sandboxed tasks are left out: this computer has no sandbox)");
    }
    if tasks.is_empty() {
        return Err("no task matches".to_owned());
    }
    Ok(tasks)
}

fn read_report(path: &Path) -> Result<Report, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{} is not a report: {err}", path.display()))
}

/// Writes a report whole, as a later run reads it back as its baseline.
fn write(path: &Path, text: &str) -> Result<(), String> {
    glave_ports::file::replace(path, text.as_bytes())
        .map_err(|err| format!("could not write {}: {err}", path.display()))
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        models: Vec::new(),
        only: None,
        tier: Some(Tier::Fast),
        repeat: 1,
        concurrency: 6,
        max_cost_micros: 2_000_000,
        baseline: None,
        tasks: PathBuf::from("evals/tasks"),
        plans: true,
        subagents: false,
        memory: None,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--model" => options.models.push(value()?),
            "--only" => options.only = Some(value()?),
            "--tier" => {
                options.tier = match value()?.as_str() {
                    "fast" => Some(Tier::Fast),
                    "hard" => Some(Tier::Hard),
                    "all" => None,
                    other => return Err(format!("--tier is fast, hard or all, not {other:?}")),
                };
            }
            "--repeat" => options.repeat = positive(&value()?, "--repeat")?,
            "--concurrency" => {
                options.concurrency = usize::try_from(positive(&value()?, "--concurrency")?)
                    .map_err(|err| err.to_string())?;
            }
            "--max-cost" => options.max_cost_micros = micros(&value()?)?,
            "--baseline" => options.baseline = Some(PathBuf::from(value()?)),
            "--tasks" => options.tasks = PathBuf::from(value()?),
            "--no-plan" => options.plans = false,
            "--subagents" => options.subagents = true,
            "--memory" => {
                options.memory = match value()?.as_str() {
                    "on" => Some(true),
                    "off" => Some(false),
                    other => return Err(format!("--memory is on or off, not {other:?}")),
                };
            }
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    if options.models.is_empty() {
        options.models = DEFAULT_MODELS
            .iter()
            .map(|&model| model.to_owned())
            .collect();
    }
    Ok(options)
}

fn positive(text: &str, name: &str) -> Result<u32, String> {
    text.parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("{name} needs a whole number above 0"))
}

/// Dollars, `2` or `0.50`, as micros; never fewer than none.
fn micros(text: &str) -> Result<i64, String> {
    Money::parse_dollars(text)
        .ok()
        .map(|money| money.micros)
        .filter(|micros| *micros >= 0)
        .ok_or_else(|| format!("--max-cost is dollars, such as 2 or 0.50, not {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        options(&args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn defaults_run_the_fast_tier_on_three_models() {
        let options = parse(&[]).unwrap();
        assert_eq!(options.models, DEFAULT_MODELS);
        assert_eq!(options.tier, Some(Tier::Fast));
        assert_eq!(options.repeat, 1);
        assert_eq!(options.max_cost_micros, 2_000_000);
        assert!(options.plans, "as glave runs");
    }

    #[test]
    fn every_option_parses() {
        let options = parse(&[
            "--model",
            "a/b",
            "--model",
            "c/d",
            "--only",
            "fix",
            "--tier",
            "all",
            "--repeat",
            "3",
            "--concurrency",
            "2",
            "--max-cost",
            "0.5",
            "--baseline",
            "old.json",
            "--tasks",
            "t",
            "--no-plan",
            "--subagents",
            "--memory",
            "on",
        ])
        .unwrap();
        assert_eq!(
            options,
            Options {
                models: vec!["a/b".into(), "c/d".into()],
                only: Some("fix".into()),
                tier: None,
                repeat: 3,
                concurrency: 2,
                max_cost_micros: 500_000,
                baseline: Some("old.json".into()),
                tasks: "t".into(),
                plans: false,
                subagents: true,
                memory: Some(true),
            }
        );
    }

    #[test]
    fn only_the_newest_runs_are_kept() {
        let base = glave_ports::testing::TempDir::new("eval-prune").unwrap();
        for day in 10..22 {
            std::fs::create_dir_all(base.join(format!("202610{day}-120000"))).unwrap();
        }
        std::fs::create_dir_all(base.join("notes")).unwrap();
        prune(&base, 10);
        let mut left: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), 11, "{left:?}");
        assert_eq!(left.first().map(String::as_str), Some("20261012-120000"));
        assert!(
            left.contains(&"notes".to_owned()),
            "other folders are left alone"
        );
        std::fs::remove_dir_all(&base).unwrap();
        assert!(is_run("20261008-182952"));
        assert!(!is_run("2026100818295x"));
        assert!(!is_run("cargo-target"));
    }

    #[test]
    fn mistakes_are_refused() {
        assert!(parse(&["--repeat", "0"]).is_err());
        assert!(parse(&["--tier", "medium"]).is_err());
        assert!(parse(&["--model"]).is_err());
        assert!(parse(&["--colour"]).is_err());
        assert!(parse(&["--max-cost", "-1"]).is_err());
        assert!(parse(&["--max-cost", "-0.5"]).is_err());
        assert!(parse(&["--max-cost", "1.x"]).is_err());
        assert_eq!(micros("2"), Ok(2_000_000));
        assert_eq!(micros("0.05"), Ok(50_000));
    }
}
