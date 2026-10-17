//! `cargo xtask eval`: glave's harness on the task folders in `evals/tasks`, against real
//! models, graded from the events and files left behind (`glave-eval --help` lists the
//! options). Not part of `ci`: it needs a model endpoint and costs money, so run it after
//! changes the model will feel. `cargo xtask eval judge` scores the action judge on
//! `evals/judge` instead.
//!
//! The endpoint comes from the environment, or from a `.env` file: the one `--env-file <path>`
//! names, else the nearest at or above the workspace, else the main checkout's when this is a
//! git worktree. Only the variables the evals read are taken from it, only for the run, and
//! never over one the environment already sets.

use std::path::PathBuf;

use crate::env_file;
use crate::lock::{self, Mode};

/// What the evals read from the environment: the model endpoint.
const ENDPOINT: [&str; 2] = ["GLAVE_BASE_URL", "GLAVE_API_KEY"];

/// Runs `glave-eval` with `args`, sharing the machine with tests but never with benchmarks.
pub fn run(args: &[String]) -> Result<(), String> {
    let (given, args) = env_file_option(args)?;
    let file = match given {
        Some(file) => Some(file),
        None => env_file::nearest(&crate::workspace::root()?),
    };
    let mut endpoint = match &file {
        Some(file) => env_file::read(file, &ENDPOINT)?,
        None => Vec::new(),
    };
    endpoint.retain(|(name, _)| std::env::var_os(name).is_none());
    if let (Some(file), false) = (&file, endpoint.is_empty()) {
        let names: Vec<&str> = endpoint.iter().map(|(name, _)| *name).collect();
        eprintln!("xtask: {} from {}", names.join(" and "), file.display());
    }
    let _guard = lock::acquire(Mode::Shared)?;
    let mut command = vec!["run", "--quiet", "--package", "glave-eval", "--"];
    command.extend(args.iter().map(String::as_str));
    crate::cmd::run_with("cargo", &command, &endpoint)
}

/// The file `--env-file <path>` names, and the arguments left for `glave-eval`.
fn env_file_option(args: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let mut file = None;
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--env-file" {
            let path = args.next().ok_or("--env-file needs a path")?;
            file = Some(PathBuf::from(path));
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((file, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|&arg| arg.to_owned()).collect()
    }

    #[test]
    fn env_file_is_taken_out_and_the_rest_passed_on() {
        assert_eq!(
            env_file_option(&words(&["--repeat", "3", "--env-file", "e.env", "--todo"])),
            Ok((Some("e.env".into()), words(&["--repeat", "3", "--todo"])))
        );
        assert_eq!(
            env_file_option(&words(&["judge", "--wrong"])),
            Ok((None, words(&["judge", "--wrong"])))
        );
        assert!(env_file_option(&words(&["--env-file"])).is_err());
    }
}
