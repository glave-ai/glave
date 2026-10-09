//! `glave-eval judge`: scores a judge of actions on the cases in `evals/judge`. A judge sees
//! what the person asked and the action the model wants to take, and says whether it may run
//! or needs the person. The score is how often it asks when the action should run (false
//! alarms), how often it asks when it should (caught), per category, and how long each
//! decision takes. `evals/judge/README.md` describes the cases and their labels.
//!
//! The judge scored today is glave's deterministic checks, as auto mode applies them: the
//! baseline any model judge has to beat.

mod baseline;
mod case;
mod score;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use score::Score;

const USAGE: &str = "\
usage: glave-eval judge [options]

Scores glave's deterministic checks, as auto mode applies them, on the judge cases: false
alarms on actions that should run and catches on actions that should ask, per category.

options:
  --cases <folder>  where the cases are (default evals/judge)
  --wrong           also list every wrong decision";

#[derive(Debug, PartialEq)]
struct Options {
    cases: PathBuf,
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
    Ok(())
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        cases: PathBuf::from("evals/judge"),
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
                wrong: false,
            }
        );
        let chosen = parse(&["--cases", "x", "--wrong"]).unwrap();
        assert_eq!(chosen.cases, PathBuf::from("x"));
        assert!(chosen.wrong);
        assert!(parse(&["--cases"]).is_err());
        assert!(parse(&["--model", "m"]).is_err());
    }
}
