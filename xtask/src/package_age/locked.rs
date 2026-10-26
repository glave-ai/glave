//! The package versions a lockfile names, read as its tool writes it (`Cargo.lock`, and npm's
//! `web/package-lock.json`), and the versions a change brings in. A version from a registry is
//! dated by that registry; one from a path, the workspace or git has no registry to date it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// crates.io's index, as Cargo.lock names it: the git index, and the sparse one.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

/// Where npm's lockfile says a package from the npm registry was downloaded.
const NPM_REGISTRY: &str = "https://registry.npmjs.org/";

/// How npm's lockfile begins what a package from git was resolved to.
const GIT: [&str; 2] = ["git+", "git:"];

/// A registry glave takes packages from, each with the lockfile that names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Registry {
    /// crates.io, for the Rust workspace.
    Crates,
    /// The npm registry, for the web app.
    Npm,
}

impl Registry {
    /// Every registry, each with its lockfile.
    pub(super) const ALL: [Self; 2] = [Self::Crates, Self::Npm];

    /// Its lockfile, from the workspace's root.
    pub(super) fn lockfile(self) -> &'static str {
        match self {
            Self::Crates => "Cargo.lock",
            Self::Npm => "web/package-lock.json",
        }
    }

    /// Its name, as people say it.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Crates => "crates.io",
            Self::Npm => "the npm registry",
        }
    }

    /// What `text`, its lockfile, names.
    pub(super) fn read(self, text: &str) -> Result<Locked, String> {
        match self {
            Self::Crates => cargo(text),
            Self::Npm => npm(text),
        }
    }
}

/// A package at one version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Package {
    pub(super) name: String,
    pub(super) version: String,
}

impl Package {
    fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }
}

/// Where a package no registry dates comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Elsewhere {
    /// A folder of the workspace, or a path beside it.
    Path,
    /// A git repository.
    Git,
}

impl Elsewhere {
    /// Where, in words.
    pub(super) fn said(self) -> &'static str {
        match self {
            Self::Path => "a path",
            Self::Git => "git",
        }
    }
}

/// What a lockfile names: the versions from its registry, and those from elsewhere.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Locked {
    registry: BTreeSet<Package>,
    elsewhere: BTreeMap<Package, Elsewhere>,
}

/// A version from the registry a change brings in, and the versions of the same package it
/// replaces, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Added {
    pub(super) package: Package,
    pub(super) replaces: Vec<String>,
}

/// What a change brings in.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Change {
    /// The versions from the registry, each to be dated.
    pub(super) added: Vec<Added>,
    /// The versions from elsewhere, which no registry dates.
    pub(super) skipped: Vec<(Package, Elsewhere)>,
}

/// What the lockfile `head` names that `base` did not; what it no longer names is not asked
/// about.
pub(super) fn change(base: &Locked, head: &Locked) -> Change {
    let added = head
        .registry
        .difference(&base.registry)
        .map(|package| Added {
            package: package.clone(),
            replaces: base
                .registry
                .iter()
                .filter(|old| old.name == package.name && !head.registry.contains(*old))
                .map(|old| old.version.clone())
                .collect(),
        })
        .collect();
    let skipped = head
        .elsewhere
        .iter()
        .filter(|(package, _)| !base.elsewhere.contains_key(*package))
        .map(|(package, from)| (package.clone(), *from))
        .collect();
    Change { added, skipped }
}

/// What `Cargo.lock`, the text of one, names: each `[[package]]`, from crates.io when its
/// `source` names it, from git when that names a repository, and from a path when it has none.
fn cargo(text: &str) -> Result<Locked, String> {
    let lock: toml::Table = text
        .parse()
        .map_err(|err| format!("Cargo.lock is not TOML: {err}"))?;
    let mut locked = Locked::default();
    let packages = lock.get("package").and_then(toml::Value::as_array);
    for entry in packages.into_iter().flatten() {
        let field = |key: &str| entry.get(key).and_then(toml::Value::as_str);
        let name = field("name").ok_or("Cargo.lock names a package without its name")?;
        let version =
            field("version").ok_or_else(|| format!("Cargo.lock gives {name} no version"))?;
        let package = Package::new(name, version);
        match field("source") {
            None => {
                locked.elsewhere.insert(package, Elsewhere::Path);
            }
            Some(source) if source.starts_with("git+") => {
                locked.elsewhere.insert(package, Elsewhere::Git);
            }
            Some(source) if CRATES_IO.contains(&source) => {
                locked.registry.insert(package);
            }
            Some(source) => {
                return Err(format!(
                    "Cargo.lock takes {name} {version} from {source}, a registry this check \
                     can't ask; glave takes crates from crates.io alone (deny.toml)"
                ));
            }
        }
    }
    Ok(locked)
}

/// What `web/package-lock.json`, the text of one, names: each entry of its `packages` under a
/// `node_modules` folder, from the npm registry unless it was resolved to a path or to git. A
/// folder of the project's own (the root, a workspace) is from a path, and a link to one adds
/// nothing, as the folder is listed itself.
fn npm(text: &str) -> Result<Locked, String> {
    let lock: Value = serde_json::from_str(text)
        .map_err(|err| format!("web/package-lock.json is not JSON: {err}"))?;
    let packages = lock
        .get("packages")
        .and_then(Value::as_object)
        .ok_or("web/package-lock.json lists no `packages`, as npm 7 and later write them")?;
    let mut locked = Locked::default();
    for (path, entry) in packages {
        let field = |key: &str| entry.get(key).and_then(Value::as_str);
        if entry.get("link").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        // The root has no version of its own to date.
        let Some(version) = field("version") else {
            continue;
        };
        let installed = path.rsplit_once("node_modules/").map(|(_, name)| name);
        // An alias installs a package under another name, and says the package's own.
        let Some(name) = field("name").or(installed) else {
            continue;
        };
        let package = Package::new(name, version);
        let resolved = field("resolved").unwrap_or(NPM_REGISTRY);
        if installed.is_none() || resolved.starts_with("file:") {
            locked.elsewhere.insert(package, Elsewhere::Path);
        } else if GIT.iter().any(|git| resolved.starts_with(git)) {
            locked.elsewhere.insert(package, Elsewhere::Git);
        } else if resolved.starts_with(NPM_REGISTRY) {
            locked.registry.insert(package);
        } else {
            return Err(format!(
                "web/package-lock.json takes {name} {version} from {resolved}, which this check \
                 can't ask; glave takes npm packages from {NPM_REGISTRY} alone"
            ));
        }
    }
    Ok(locked)
}

#[cfg(test)]
mod tests;
