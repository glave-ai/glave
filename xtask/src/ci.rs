//! `cargo xtask ci`: the local gate, what CI runs: the Rust checks and docs, that no tracked
//! file holds a secret-shaped literal, that crates keep the dependency rule and start processes
//! only through glave-process, that every wait has a limit (`waits.rs`), that the tests leave
//! nothing behind (`leftovers.rs`), then the web checks (`web.rs`), which need Node unless
//! `GLAVE_SKIP_WEB=1` skips them. On a laptop the lint, docs and tests cover only the crates a
//! change reaches, and the web checks run only when the web changed (`changed.rs`); `--all`,
//! and CI always, check everything. `--target` builds the lint, docs and tests for a target
//! other than this machine's: CI's run on musl, which glave's Linux releases are built for.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::changed::{Change, Scope};
use crate::cmd;
use crate::lock::{self, Mode};

/// The feature that builds the web app into glave. It needs the app built first, so the Rust
/// checks run with every feature but this one, and the web checks test it.
pub(crate) const BUILT_IN_APP: (&str, &str) = ("glave", "web");

/// How `cargo xtask ci` is run.
const USAGE: &str = "usage: cargo xtask ci [--all] [--target <triple>]";

/// What `cargo xtask ci` was asked for: everything, or only what changed; and the target the
/// Rust checks build for, when it isn't this machine's.
struct Asked {
    all: bool,
    target: Option<String>,
}

/// The crates the Rust checks cover, as cargo takes them, their features, and the target they
/// build for, when it isn't this machine's. Given to cargo as `--target`, never in the
/// environment, so the commands the tests run build for this machine as they would anywhere.
struct Rust {
    packages: Vec<String>,
    features: String,
    target: Option<String>,
}

impl Rust {
    /// `cargo <command>` for these crates, then `rest`.
    fn args<'a>(&'a self, command: &[&'a str], rest: &[&'a str]) -> Vec<&'a str> {
        let mut args = command.to_vec();
        args.extend(self.packages.iter().map(String::as_str));
        if !self.features.is_empty() {
            args.extend(["--features", self.features.as_str()]);
        }
        if let Some(target) = &self.target {
            args.extend(["--target", target.as_str()]);
        }
        args.extend(rest);
        args
    }
}

/// Runs every check in order and stops at the first failure; `args` as `USAGE` says.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let Asked { all, target } = asked(args)?;
    let _guard = lock::acquire(Mode::Shared)?;
    let metadata = crate::workspace::metadata()?;
    let (rust, web) = plan(&crate::changed::scope(all, &metadata)?, &metadata, target)?;
    let threads = threads()?;
    step("format", || cmd::run("cargo", &["fmt", "--all", "--check"]))?;
    step("secrets", crate::secrets::check)?;
    step("dependency rule", crate::deps::check)?;
    step("one spawn path", crate::spawns::check)?;
    step("every wait has a limit", crate::waits::check)?;
    match &rust {
        Some(rust) => rust_checks(rust, &threads)?,
        None => println!("\n== lint, docs and tests: skipped, as no crate changed"),
    }
    step("dependencies", || cmd::run("cargo", &["deny", "check"]))?;
    // Called directly: run as `cargo machete` under `cargo xtask`, it takes `machete` for a path.
    step("unused dependencies", || cmd::run("cargo-machete", &[]))?;
    step("file sizes", crate::file_sizes::check)?;
    step("api schema", || crate::schema::run(&["--check".to_owned()]))?;
    step("release workflow", crate::release_workflow::check)?;
    if web {
        step("web", || crate::web::check(&threads))?;
    } else {
        println!("\n== web: skipped, as nothing in web/ or the API schema changed");
    }
    println!("\nxtask: all checks passed");
    Ok(())
}

/// The lint, the docs and the tests of `rust`'s crates, with `threads` saying how many tests
/// run at once.
fn rust_checks(rust: &Rust, threads: &[(&str, String)]) -> Result<(), String> {
    step("lint", || {
        let args = rust.args(&["clippy"], &["--all-targets", "--", "-D", "warnings"]);
        cmd::run("cargo", &args)
    })?;
    // Doc links that no longer resolve, or that point at private items, fail here rather than
    // rot (`[workspace.lints.rustdoc]`); every other rustdoc warning fails too.
    step("docs", || {
        cmd::run_with(
            "cargo",
            &rust.args(&["doc"], &["--no-deps"]),
            &[("RUSTDOCFLAGS", "-D warnings".to_owned())],
        )
    })?;
    let run = crate::leftovers::Run::start()?;
    let tests = step("tests", || {
        let env = [run.env(), threads.to_vec()].concat();
        crate::nextest::run(&rust.args(&[], &[]), &["--no-tests=pass"], &env)
    });
    // Checked whether or not the tests passed, so a failed run stops what it left running.
    let leftovers = step("leftovers", || run.check());
    tests?;
    leftovers
}

/// `cargo xtask ci`'s arguments, read.
fn asked(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked {
        all: false,
        target: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" if !asked.all => asked.all = true,
            "--target" if asked.target.is_none() => {
                asked.target = Some(args.next().ok_or(USAGE)?.clone());
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(asked)
}

/// What `scope` checks, said before anything runs: the Rust checks' crates, none when no crate
/// changed, built for `target`; and whether the web checks run.
fn plan(
    scope: &Scope,
    metadata: &Value,
    target: Option<String>,
) -> Result<(Option<Rust>, bool), String> {
    let change = match scope {
        Scope::All(why) => {
            println!("xtask: checking everything ({why})");
            let rust = Rust {
                packages: vec!["--workspace".to_owned()],
                features: features_in(metadata, None)?,
                target,
            };
            return Ok((Some(rust), true));
        }
        Scope::Changed(change) => change,
    };
    println!("{}", told(change));
    if change.reached.is_empty() {
        return Ok((None, change.web));
    }
    let rust = Rust {
        packages: change
            .reached
            .iter()
            .flat_map(|name| ["-p".to_owned(), name.clone()])
            .collect(),
        features: features_in(metadata, Some(&change.reached))?,
        target,
    };
    Ok((Some(rust), change.web))
}

/// What a run of only what `change` reaches checks, and what it skips, in words.
fn told(change: &Change) -> String {
    let crates = if change.reached.is_empty() {
        "no crate changed, so lint, docs and tests are skipped".to_owned()
    } else {
        let names = |set: &BTreeSet<String>| set.iter().cloned().collect::<Vec<_>>().join(", ");
        let others = &change.reached - &change.touched;
        let dependents = if others.is_empty() {
            String::new()
        } else {
            format!(", and the crates that depend on them: {}", names(&others))
        };
        format!(
            "lint, docs and tests for {}{dependents}; the other crates are skipped",
            names(&change.touched)
        )
    };
    let web = if change.web {
        "the web checks run"
    } else {
        "the web checks are skipped, as nothing in web/ or the API schema changed"
    };
    format!(
        "xtask: checking only what changed since this branch left origin/main: {crates}; {web}. \
         `cargo xtask ci --all` checks everything, as CI does."
    )
}

/// How many tests run at once, said once, as the variables every test runner reads: nextest's
/// own, and the one the web tests read.
fn threads() -> Result<Vec<(&'static str, String)>, String> {
    let Some((threads, why)) = crate::parallel::tests()? else {
        return Ok(Vec::new());
    };
    println!("xtask: {threads} tests at once ({why})");
    let threads = threads.to_string();
    Ok(vec![
        ("NEXTEST_TEST_THREADS", threads.clone()),
        (crate::parallel::VARIABLE, threads),
    ])
}

fn step(name: &str, check: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    println!("\n== {name}");
    check().map_err(|err| format!("{name}: {err}"))
}

/// Every feature of every package in `cargo metadata`'s JSON as `--features` takes them
/// (`glave/otlp`), but the one that builds the web app in; of the packages in `only`, when
/// given, since cargo refuses a feature of a package it was not asked to build.
fn features_in(metadata: &Value, only: Option<&BTreeSet<String>>) -> Result<String, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("`cargo metadata` named no packages")?;
    let mut features = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("`cargo metadata` named a package without its name")?;
        if only.is_some_and(|only| !only.contains(name)) {
            continue;
        }
        let declared = package
            .get("features")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("`cargo metadata` gave {name} no features"))?;
        features.extend(
            declared
                .keys()
                .filter(|feature| (name, feature.as_str()) != BUILT_IN_APP)
                .map(|feature| format!("{name}/{feature}")),
        );
    }
    Ok(features.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_is_checked_but_the_built_in_app() {
        let metadata = serde_json::json!({"packages": [
            {"name": "glave", "features": {"otlp": [], "web": []}},
            {"name": "glave-keys", "features": {"keychain": [], "conformance": []}},
            {"name": "glave-event", "features": {}},
        ]});
        assert_eq!(
            features_in(&metadata, None),
            Ok("glave/otlp,glave-keys/conformance,glave-keys/keychain".to_owned())
        );
        let only = BTreeSet::from(["glave-keys".to_owned()]);
        assert_eq!(
            features_in(&metadata, Some(&only)),
            Ok("glave-keys/conformance,glave-keys/keychain".to_owned())
        );
        assert!(features_in(&serde_json::json!({}), None).is_err());
    }

    #[test]
    fn everything_and_a_target_may_be_asked_for_once_each_in_any_order() {
        let asked_for = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
            asked(&args).map(|Asked { all, target }| (all, target))
        };
        let musl = || Some("x86_64-unknown-linux-musl".to_owned());
        assert_eq!(asked_for(&[]), Ok((false, None)));
        assert_eq!(asked_for(&["--all"]), Ok((true, None)));
        assert_eq!(
            asked_for(&["--target", "x86_64-unknown-linux-musl"]),
            Ok((false, musl()))
        );
        assert_eq!(
            asked_for(&["--target", "x86_64-unknown-linux-musl", "--all"]),
            Ok((true, musl()))
        );
        for wrong in [
            &["--target"][..],
            &["--all", "--all"],
            &["--target", "a", "--target", "b"],
            &["--fast"],
        ] {
            assert_eq!(asked_for(wrong), Err(USAGE.to_owned()), "{wrong:?}");
        }
    }

    #[test]
    fn the_target_follows_the_features_and_comes_before_the_commands_own_arguments() {
        let rust = Rust {
            packages: vec!["--workspace".to_owned()],
            features: "glave/otlp".to_owned(),
            target: Some("x86_64-unknown-linux-musl".to_owned()),
        };
        assert_eq!(
            rust.args(&["clippy"], &["--", "-D", "warnings"]),
            [
                "clippy",
                "--workspace",
                "--features",
                "glave/otlp",
                "--target",
                "x86_64-unknown-linux-musl",
                "--",
                "-D",
                "warnings"
            ]
        );
    }

    #[test]
    fn a_run_of_what_changed_says_what_it_skips() {
        let change = Change {
            touched: BTreeSet::from(["glave-store".to_owned()]),
            reached: BTreeSet::from(["glave-store".to_owned(), "glave".to_owned()]),
            web: false,
        };
        let said = told(&change);
        assert!(
            said.contains("lint, docs and tests for glave-store"),
            "{said}"
        );
        assert!(
            said.contains("the crates that depend on them: glave;"),
            "{said}"
        );
        assert!(said.contains("web checks are skipped"), "{said}");
        assert!(said.contains("--all"), "{said}");
        let said = told(&Change::default());
        assert!(said.contains("no crate changed"), "{said}");
    }
}
