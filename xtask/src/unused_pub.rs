//! `cargo xtask unused-pub`: anything `pub` has a caller outside its crate (AGENTS.md,
//! "Nothing before its second user"). `unreachable_pub` already denies a `pub` item its crate's
//! root does not export; this check reads what each library crate does export (`exported.rs`)
//! and fails on an item no other crate names, nor the interface of an exported item that is
//! named: a type in a used function's signature, a public field's type, a variant's, what a
//! used trait asks, which must stay public while their item is.
//!
//! An item counts as named when its name is a word in a tracked Rust source of the workspace
//! outside its crate's own module tree: another crate's, a binary's, or a test's, its crate's
//! own integration tests included, since a test can reach only what is public and the
//! conformance suites and test doubles exist for other crates' tests. A common name may be
//! another item's, which errs toward "used", the safe way for a gate. Binaries (glave, xtask,
//! eval's command line) export nothing, so only their callers are read. An item that must stay
//! public with no caller is on [`ALLOWED`], with its reason; an entry that no longer holds
//! fails until it is taken off.

mod exported;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde_json::Value;

use crate::{cmd, workspace};
use exported::{Exported, Library};

/// Exported items no other crate names that stay public on purpose: the crate, the item's name,
/// and why.
const ALLOWED: &[(&str, &str, &str)] = &[];

/// A workspace package: its name, its folder from the workspace root (with a trailing `/`),
/// and its library's root, when it has a library.
struct Package {
    name: String,
    folder: String,
    lib: Option<String>,
}

/// The workspace's tracked Rust sources, and where each word is in them.
struct Sources {
    paths: Vec<String>,
    texts: Vec<String>,
    /// Each word, and the indices of the sources it is in.
    named_in: HashMap<String, Vec<usize>>,
}

/// Fails on an item a library crate exports that nothing outside it names, and on an entry of
/// [`ALLOWED`] that no longer stands for one.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace::root()?;
    let packages = packages(&workspace::metadata()?, &root)?;
    let sources = Sources::read(&root, &packages)?;
    let mut problems = Vec::new();
    let mut allowed_found = BTreeSet::new();
    let (mut libraries, mut exported) = (0_usize, 0_usize);
    for package in &packages {
        let Some(lib) = &package.lib else { continue };
        let library = exported::read(&root, lib)?;
        libraries += 1;
        exported += library.exported.len();
        let named = |name: &str| sources.named_outside(name, &library.files);
        for item in unused(&library, named) {
            match ALLOWED
                .iter()
                .position(|(name, allowed, _)| (*name, *allowed) == (&package.name, &item.name))
            {
                Some(entry) => {
                    allowed_found.insert(entry);
                }
                None => problems.push(sources.located(item)),
            }
        }
    }
    problems.extend(
        ALLOWED
            .iter()
            .enumerate()
            .filter(|(entry, _)| !allowed_found.contains(entry))
            .map(|(_, (name, item, _))| {
                format!(
                    "{name}: `{item}` is allowed, but is no longer exported unused; take it off \
                     ALLOWED in xtask/src/unused_pub.rs"
                )
            }),
    );
    if problems.is_empty() {
        println!(
            "xtask: the {exported} items {libraries} library crates export each have a caller \
             outside their crate ({} allowed)",
            ALLOWED.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} exported item(s) with no caller outside their crate; make each pub(crate), or \
             delete it if its own crate does not use it either (only a library API that must \
             stay public goes on ALLOWED in xtask/src/unused_pub.rs, with its reason):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// What `library` exports that `named` (whether a name is a word outside the crate) does not
/// find, nor the interface of an item it finds, reached through every interface in turn.
fn unused(library: &Library, named: impl Fn(&str) -> bool) -> Vec<&Exported> {
    let mut used: BTreeSet<&str> = library
        .exported
        .iter()
        .map(|item| item.name.as_str())
        .filter(|name| named(name))
        .collect();
    let mut reached: Vec<&str> = used.iter().copied().collect();
    while let Some(name) = reached.pop() {
        for held in library.interfaces.get(name).into_iter().flatten() {
            if library.interfaces.contains_key(held) && used.insert(held) {
                reached.push(held);
            }
        }
    }
    library
        .exported
        .iter()
        .filter(|item| !used.contains(item.name.as_str()))
        .collect()
}

impl Sources {
    /// Reads every tracked Rust source in a workspace package's folder.
    fn read(root: &Path, packages: &[Package]) -> Result<Self, String> {
        let listed = cmd::query("git", &["ls-files", "-z", "--", "*.rs"])?;
        let paths: Vec<String> = listed
            .split('\0')
            .filter(|path| {
                packages
                    .iter()
                    .any(|package| path.starts_with(&package.folder))
            })
            .map(str::to_owned)
            .collect();
        let mut texts = Vec::with_capacity(paths.len());
        let mut named_in: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, path) in paths.iter().enumerate() {
            let text = std::fs::read_to_string(root.join(path))
                .map_err(|err| format!("could not read {path}: {err}"))?;
            for word in words(&text).collect::<BTreeSet<_>>() {
                named_in.entry(word.to_owned()).or_default().push(index);
            }
            texts.push(text);
        }
        Ok(Self {
            paths,
            texts,
            named_in,
        })
    }

    /// Whether `name` is a word in a source outside `inside`.
    fn named_outside(&self, name: &str, inside: &BTreeSet<String>) -> bool {
        self.named_in.get(name).is_some_and(|found| {
            found.iter().any(|index| {
                self.paths
                    .get(*index)
                    .is_some_and(|path| !inside.contains(path))
            })
        })
    }

    /// Where `item` is, as `path:line: pub <kind> <name>`: the first line of its file that
    /// starts with `pub` and names it, else the first that names it.
    fn located(&self, item: &Exported) -> String {
        let text = self
            .paths
            .iter()
            .position(|path| *path == item.file)
            .and_then(|index| self.texts.get(index))
            .map_or("", String::as_str);
        let names = |line: &str| words(line).any(|word| word == item.name);
        let lines = || text.lines().enumerate();
        let line = lines()
            .find(|(_, line)| line.trim_start().starts_with("pub") && names(line))
            .or_else(|| lines().find(|(_, line)| names(line)))
            .map_or(1, |(n, _)| n + 1);
        format!("{}:{line}: pub {} {}", item.file, item.kind, item.name)
    }
}

/// The words of `text`: each run of letters, digits and `_` that does not start with a digit.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.chars().next().is_some_and(|c| !c.is_ascii_digit()))
}

/// The workspace's packages from `cargo metadata`'s JSON, their paths from `root`.
fn packages(metadata: &Value, root: &Path) -> Result<Vec<Package>, String> {
    let listed = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("`cargo metadata` named no packages")?;
    let relative = |path: &str| {
        Path::new(path)
            .strip_prefix(root)
            .ok()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{path} is outside the workspace"))
    };
    let mut packages = Vec::new();
    for package in listed {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("`cargo metadata` named a package without its name")?;
        let manifest = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("`cargo metadata` gave {name} no manifest"))?;
        let folder = relative(manifest)?
            .strip_suffix("Cargo.toml")
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}'s manifest is not a Cargo.toml"))?;
        let lib = package
            .get("targets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|target| {
                target
                    .get("kind")
                    .and_then(Value::as_array)
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
            })
            .and_then(|target| target.get("src_path").and_then(Value::as_str))
            .map(relative)
            .transpose()?;
        packages.push(Package {
            name: name.to_owned(),
            folder,
            lib,
        });
    }
    Ok(packages)
}

#[cfg(test)]
mod tests;
