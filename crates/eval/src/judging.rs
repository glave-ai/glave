//! `glave-eval judge`: scores a judge of actions on the cases in `evals/judge`. A judge sees
//! what the person asked and the action the model wants to take, and says whether it may run
//! or needs the person. The score is how often it asks when the action should run (false
//! alarms), how often it asks when it should (caught), per category, and how long each
//! decision takes. `evals/judge/README.md` describes the cases and their labels.
//!
//! The judge scored today is glave's deterministic checks, as auto mode applies them: the
//! baseline any model judge has to beat.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the judge's command line: it reports in the terminal"
)]

mod baseline;
mod case;
mod model;
mod score;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use glave_provider_openai::OpenAiCompatible;
use model::{Metered, decide_all};
use score::Score;

const USAGE: &str = "\
usage: glave-eval judge [options]

Scores glave's deterministic checks, as auto mode applies them, on the judge cases: false
alarms on actions that should run and catches on actions that should ask, per category. With
--model, also scores the checks with that model judging every soft caution, as auto mode does
with a judge; the endpoint comes from GLAVE_BASE_URL and GLAVE_API_KEY.

options:
  --cases <folder>  where the cases are (default evals/judge)
  --model <id>      a model to judge soft cautions (repeat for several)
  --at-once <n>     judge calls at once (default 8)
  --wrong           also list every wrong decision";

#[derive(Debug, PartialEq)]
struct Options {
    cases: PathBuf,
    models: Vec<String>,
    at_once: usize,
    wrong: bool,
}

pub(crate) fn main(args: &[String]) -> ExitCode {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match options(args).and_then(|options| run(&options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("glave-eval judge: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(options: &Options) -> Result<(), String> {
    let cases = case::load(&options.cases).map_err(|err| err.to_string())?;
    let mut score = Score::default();
    for case in &cases {
        let started = Instant::now();
        let label = baseline::decide(&case.action);
        score.record(case, label, started.elapsed());
    }
    println!("{}", score.markdown("deterministic checks"));
    if options.wrong {
        println!("{}", score.wrong());
    }
    if options.models.is_empty() {
        return Ok(());
    }
    let endpoint = crate::cli::endpoint()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start: {err}"))?;
    for name in &options.models {
        let client = OpenAiCompatible::new(endpoint.clone()).map_err(|err| err.to_string())?;
        let provider = Arc::new(Metered::new(client));
        let decided = runtime.block_on(decide_all(
            &cases,
            Arc::clone(&provider),
            name,
            options.at_once,
        ));
        let mut score = Score::default();
        for (case, (label, took)) in cases.iter().zip(decided) {
            score.record(case, label, took);
        }
        let micros = provider.micros();
        println!(
            "{}cost: ${}.{:06}\n",
            score.markdown(&format!("checks + {name} judging soft cautions")),
            micros / 1_000_000,
            micros % 1_000_000
        );
        if options.wrong {
            println!("{}", score.wrong());
        }
    }
    Ok(())
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        cases: PathBuf::from("evals/judge"),
        models: Vec::new(),
        at_once: 8,
        wrong: false,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--cases" => {
                let folder = rest
                    .next()
                    .ok_or_else(|| format!("--cases needs a folder\n{USAGE}"))?;
                options.cases = PathBuf::from(folder);
            }
            "--model" => {
                let model = rest
                    .next()
                    .ok_or_else(|| format!("--model needs a model\n{USAGE}"))?;
                options.models.push(model.clone());
            }
            "--at-once" => {
                let n = rest
                    .next()
                    .ok_or_else(|| format!("--at-once needs a number\n{USAGE}"))?;
                options.at_once = n
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("--at-once must be a positive number, not {n:?}"))?;
            }
            "--wrong" => options.wrong = true,
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Options, String> {
        options(
            &words
                .iter()
                .map(|&word| word.to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn options_have_defaults_and_refuse_strangers() {
        assert_eq!(
            parse(&[]).unwrap(),
            Options {
                cases: PathBuf::from("evals/judge"),
                models: Vec::new(),
                at_once: 8,
                wrong: false,
            }
        );
        let chosen = parse(&["--cases", "x", "--wrong"]).unwrap();
        assert_eq!(chosen.cases, PathBuf::from("x"));
        assert!(chosen.wrong);
        assert!(parse(&["--cases"]).is_err());
        assert_eq!(
            parse(&["--model", "a", "--model", "b"]).unwrap().models,
            ["a", "b"]
        );
        assert!(parse(&["--at-once", "0"]).is_err());
        assert!(parse(&["--judge", "m"]).is_err());
    }
}
