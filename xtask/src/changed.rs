//! What `cargo xtask ci` lints and tests on a laptop: the crates a change touches, and every
//! crate of the workspace that depends on one of them, since a change reaches its dependents
//! too; the web checks only when `web/` or the session API's schema changed. The change is
//! everything that differs from where this branch left `origin/main`, committed or not, and
//! files git does not track yet. `--all`, and CI always, check everything.

use std::collections::BTreeSet;

use serde_json::Value;

/// Files outside every crate that one crate reads, and that crate.
const READ_BY: &[(&str, &str)] = &[("evals/", "glave-eval")];

/// Crates whose tests run a program another crate builds, and that crate: checked with them,
/// so the program is built from today's code (glave-mcp's test server).
const RUNS: &[(&str, &str)] = &[
    ("glave", "glave-mcp"),
    ("glave-gateway-service", "glave-mcp"),
];

/// The session API's schema, which the web client's types are made from.
const SCHEMA: &str = "specs/session.schema.json";

/// What to check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Everything, and why.
    All(String),
    /// Only what a change reaches.
    Changed(Change),
}

/// What a change reaches: the crates it touches, those that depend on them, and the web.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Change {
    /// The crates the changed files are in.
    pub(crate) touched: BTreeSet<String>,
    /// Those, and every crate that depends on one of them.
    pub(crate) reached: BTreeSet<String>,
    /// Whether the web checks run.
    pub(crate) web: bool,
}

/// A crate of the workspace: its name, its folder from the workspace's root, and the crates of
/// the workspace it depends on, for its tests too.
#[derive(Debug)]
pub(crate) struct Package {
    name: String,
    folder: String,
    depends_on: Vec<String>,
}

/// What this run checks: everything with `--all` or on CI, else what changed since this
/// branch left `origin/main`.
pub(crate) fn scope(all: bool, metadata: &Value) -> Result<Scope, String> {
    if all {
        return Ok(Scope::All("asked for with --all".to_owned()));
    }
    if crate::parallel::on_ci() {
        return Ok(Scope::All(
            "on CI, which always checks everything".to_owned(),
        ));
    }
    let Ok(base) = git(&["merge-base", "HEAD", "origin/main"]) else {
        return Ok(Scope::All(
            "there is no origin/main to compare with".to_owned(),
        ));
    };
    let base = base.trim().to_owned();
    let mut files = lines(&git(&["diff", "--name-only", &base])?);
    files.extend(lines(&git(&[
        "ls-files",
        "--others",
        "--exclude-standard",
    ])?));
    Ok(reached(&files, &packages(metadata)?))
}

/// What changing `files` reaches among `packages`.
fn reached(files: &[String], packages: &[Package]) -> Scope {
    let mut change = Change::default();
    for file in files {
        if file.starts_with("web/") || file == SCHEMA {
            change.web = true;
        } else if let Some(package) = owner(file, packages) {
            change.touched.insert(package);
        } else if !aside(file) {
            return Scope::All(format!("{file} changed, which any crate may read"));
        }
    }
    change.reached = dependents(&change.touched, packages);
    let run: Vec<String> = RUNS
        .iter()
        .filter(|(tested, _)| change.reached.contains(*tested))
        .map(|(_, built)| (*built).to_owned())
        .collect();
    change.reached.extend(run);
    Scope::Changed(change)
}

/// The crate `file` belongs to: the one whose folder holds it most closely, or the one that
/// reads it.
fn owner(file: &str, packages: &[Package]) -> Option<String> {
    let inside = packages
        .iter()
        .filter(|package| {
            file.strip_prefix(&package.folder)
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|package| package.folder.len())
        .map(|package| package.name.clone());
    inside.or_else(|| {
        READ_BY
            .iter()
            .find(|(folder, _)| file.starts_with(folder))
            .map(|(_, package)| (*package).to_owned())
    })
}

/// Whether `file`, outside every crate, is read by no crate's build or tests: prose, and CI's
/// own workflows, which CI runs anyway.
fn aside(file: &str) -> bool {
    let prose = std::path::Path::new(file)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    prose || file.starts_with(".github/")
}

/// `touched`, and every crate of `packages` that depends on one of them, directly or not.
fn dependents(touched: &BTreeSet<String>, packages: &[Package]) -> BTreeSet<String> {
    let mut reached = touched.clone();
    loop {
        let more: Vec<String> = packages
            .iter()
            .filter(|package| !reached.contains(&package.name))
            .filter(|package| package.depends_on.iter().any(|dep| reached.contains(dep)))
            .map(|package| package.name.clone())
            .collect();
        if more.is_empty() {
            return reached;
        }
        reached.extend(more);
    }
}

/// The workspace's crates, as `cargo metadata --no-deps` describes them.
fn packages(metadata: &Value) -> Result<Vec<Package>, String> {
    let root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .ok_or("`cargo metadata` named no workspace root")?;
    let listed = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("`cargo metadata` named no packages")?;
    let names: BTreeSet<&str> = listed
        .iter()
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .collect();
    listed
        .iter()
        .map(|package| {
            let field = |name: &str| package.get(name).and_then(Value::as_str);
            let name = field("name").ok_or("`cargo metadata` named a package without its name")?;
            let manifest = field("manifest_path")
                .ok_or_else(|| format!("`cargo metadata` gave {name} no manifest"))?;
            let folder = manifest
                .strip_prefix(root)
                .and_then(|rest| rest.strip_suffix("/Cargo.toml"))
                .map(|folder| folder.trim_start_matches('/').to_owned())
                .ok_or_else(|| format!("{name}'s manifest {manifest} is outside {root}"))?;
            let depends_on = package
                .get("dependencies")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|dep| dep.get("name").and_then(Value::as_str))
                .filter(|dep| names.contains(dep))
                .map(str::to_owned)
                .collect();
            Ok(Package {
                name: name.to_owned(),
                folder,
                depends_on,
            })
        })
        .collect()
}

fn git(args: &[&str]) -> Result<String, String> {
    crate::cmd::output("git", args)
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests;
