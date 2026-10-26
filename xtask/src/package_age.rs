//! `cargo xtask package-age`: every package version a change brings into `Cargo.lock` or
//! `web/package-lock.json` was published at least [`OLD_ENOUGH`] ago (#744). A poisoned
//! release (a maintainer's account taken over, a malicious version published) is usually found
//! and pulled within days, and a change could otherwise bring one in the hour it appears.
//!
//! What is new is what the lockfiles name that they didn't at the change's base (`base.rs`),
//! and its registry says when it was published (`registry.rs`). A version from a path, the
//! workspace or git has no registry to say, and is named and skipped. A security fix that
//! can't wait is listed in `allowed.rs` with its reason, and its entry fails the check once
//! the version is old enough, so the list doesn't rot. When a registry can't be reached, CI
//! fails; elsewhere the check warns and passes, as CI runs it again.
//!
//! Dependabot's version updates wait as long (`cooldown`, .github/dependabot.yml), and npm
//! 11.10 and later installs only versions as old (`min-release-age`, web/.npmrc).

mod allowed;
mod base;
mod locked;
mod registry;

use std::collections::{BTreeMap, BTreeSet};

use glave_ports::{Clock as _, SystemClock};
use jiff::{SignedDuration, Timestamp};

use base::Base;
use locked::{Added, Elsewhere, Package, Registry};
use registry::{Registries, Unknown};

/// How old a package version must be before it comes in.
const OLD_ENOUGH: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Where versions let in early are listed, as the messages name it.
const ALLOW_LIST: &str = "xtask/src/package_age/allowed.rs";

/// A version a change brings in, and the registry that dates it.
#[derive(Debug)]
struct New {
    registry: Registry,
    added: Added,
}

/// What a change brings into the lockfiles.
#[derive(Debug, Default)]
struct BroughtIn {
    /// Each version from a registry, to be dated.
    new: Vec<New>,
    /// Each version from elsewhere, which no registry dates.
    skipped: Vec<(Package, Elsewhere)>,
}

/// What the age rule says.
#[derive(Debug, Default, PartialEq, Eq)]
struct Verdict {
    /// Each new version too young that the allow-list doesn't let in, in a sentence.
    too_new: Vec<String>,
    /// Each version the allow-list lets in that is old enough now, whose entry can go.
    stale: Vec<String>,
    /// How many new versions the allow-list lets in early.
    early: usize,
}

impl Verdict {
    /// Why the check fails, with `undated` the versions no registry would date; `None` when
    /// it passes.
    fn refusal(&self, undated: &[String]) -> Option<String> {
        let mut said = Vec::new();
        if !self.too_new.is_empty() {
            said.push(format!(
                "{} too new to come in:\n  {}\nA package version must be {} days old before it \
                 comes in, so a poisoned release is found and pulled first. Wait until then, \
                 or, for a security fix that can't wait, list it in {ALLOW_LIST} with its \
                 reason.",
                counted(
                    self.too_new.len(),
                    "package version is",
                    "package versions are"
                ),
                self.too_new.join("\n  "),
                days(),
            ));
        }
        if !undated.is_empty() {
            said.push(format!(
                "{} can't be dated:\n  {}",
                counted(undated.len(), "package version", "package versions"),
                undated.join("\n  ")
            ));
        }
        if !self.stale.is_empty() {
            let entries = if self.stale.len() == 1 {
                "its entry"
            } else {
                "their entries"
            };
            said.push(format!(
                "{ALLOW_LIST} lets in early {} old enough now: remove {entries}\n  {}",
                counted(self.stale.len(), "version", "versions"),
                self.stale.join("\n  ")
            ));
        }
        (!said.is_empty()).then(|| said.join("\n"))
    }
}

/// Fails on a package version a change brings in that was published less than
/// [`OLD_ENOUGH`] ago and isn't allowed, on one its registry doesn't list, and on an allowed
/// one that is old enough now.
pub(crate) fn check() -> Result<(), String> {
    let Some(base) = base::find()? else {
        println!(
            "xtask: package ages are not checked here, as there is no origin/main to compare \
             the lockfiles with; CI checks them"
        );
        return Ok(());
    };
    let BroughtIn { new, skipped } = brought_in(&base)?;
    let mut registries = Registries::default();
    let mut published = BTreeMap::new();
    let mut undated = Vec::new();
    for ((registry, name), versions) in wanted(&new) {
        match registries.published(registry, &name, &versions) {
            Ok(dates) => published.extend(dates.into_iter().map(|(version, at)| {
                let package = Package {
                    name: name.clone(),
                    version,
                };
                ((registry, package), at)
            })),
            Err(Unknown::Unlisted(why)) => undated.push(why),
            Err(Unknown::Unanswered(why)) => return unanswered(&why),
        }
    }
    let verdict = judged(&new, allowed::ALLOWED, &published, SystemClock.now());
    if !skipped.is_empty() {
        let named: Vec<String> = skipped
            .iter()
            .map(|(package, from)| {
                format!("{} {} ({})", package.name, package.version, from.said())
            })
            .collect();
        println!(
            "xtask: {} from no registry, so no age is checked: {}",
            counted(skipped.len(), "new version comes", "new versions come"),
            named.join(", ")
        );
    }
    if let Some(refusal) = verdict.refusal(&undated) {
        return Err(refusal);
    }
    println!("xtask: {}", summary(new.len(), verdict.early, &base.said()));
    Ok(())
}

/// The versions the lockfiles name that they didn't at `base`: those from a registry, and
/// those from elsewhere, which no registry dates.
fn brought_in(base: &Base) -> Result<BroughtIn, String> {
    let root = crate::workspace::root()?;
    let mut brought = BroughtIn::default();
    for registry in Registry::ALL {
        let path = registry.lockfile();
        let read = |text: Option<String>, at: &str| {
            text.map(|text| registry.read(&text).map_err(|err| format!("{err} ({at})")))
                .transpose()
                .map(Option::unwrap_or_default)
        };
        let here = match std::fs::read_to_string(root.join(path)) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(format!("could not read {path}: {err}")),
        };
        let head = read(here, "in this change")?;
        let before = read(base.file(path)?, &base.said())?;
        let change = locked::change(&before, &head);
        brought.new.extend(
            change
                .added
                .into_iter()
                .map(|added| New { registry, added }),
        );
        brought.skipped.extend(change.skipped);
    }
    Ok(brought)
}

/// Each package to ask its registry about, with its versions: each version `new` brings in,
/// and each the allow-list names, to see whether it is old enough now.
fn wanted(new: &[New]) -> BTreeMap<(Registry, String), BTreeSet<String>> {
    let mut wanted: BTreeMap<_, BTreeSet<String>> = BTreeMap::new();
    let added = new.iter().map(|new| {
        let package = &new.added.package;
        (
            new.registry,
            package.name.as_str(),
            package.version.as_str(),
        )
    });
    let allowed = allowed::ALLOWED
        .iter()
        .map(|(registry, name, version, _)| (*registry, *name, *version));
    for (registry, name, version) in added.chain(allowed) {
        wanted
            .entry((registry, name.to_owned()))
            .or_default()
            .insert(version.to_owned());
    }
    wanted
}

/// The age rule at `now`, for the versions `new` brings in and those `allowed` lets in early,
/// with `published` saying when each was published. One not dated is passed over: why it
/// isn't is a problem of its own.
fn judged(
    new: &[New],
    allowed: &[(Registry, &str, &str, &str)],
    published: &BTreeMap<(Registry, Package), Timestamp>,
    now: Timestamp,
) -> Verdict {
    let mut verdict = Verdict::default();
    let is_allowed = |registry: Registry, package: &Package| {
        allowed.iter().any(|(r, name, version, _)| {
            (*r, *name, *version) == (registry, &package.name, &package.version)
        })
    };
    for New { registry, added } in new {
        let package = &added.package;
        let Some(&at) = published.get(&(*registry, package.clone())) else {
            continue;
        };
        if now.duration_since(at) >= OLD_ENOUGH {
            continue;
        }
        if is_allowed(*registry, package) {
            verdict.early += 1;
            continue;
        }
        let replacing = if added.replaces.is_empty() {
            String::new()
        } else {
            format!(", replacing {},", added.replaces.join(" and "))
        };
        verdict.too_new.push(format!(
            "{}: {} {}{replacing} was published {}, at {}; it may come in from {}",
            registry.lockfile(),
            package.name,
            package.version,
            ago(now.duration_since(at)),
            when(at),
            when(at.checked_add(OLD_ENOUGH).unwrap_or(at)),
        ));
    }
    for (registry, name, version, _) in allowed {
        let package = Package {
            name: (*name).to_owned(),
            version: (*version).to_owned(),
        };
        let Some(&at) = published.get(&(*registry, package)) else {
            continue;
        };
        if now.duration_since(at) >= OLD_ENOUGH {
            verdict.stale.push(format!(
                "{name} {version} from {}, published {}, at {}",
                registry.name(),
                ago(now.duration_since(at)),
                when(at),
            ));
        }
    }
    verdict
}

/// Says that `why`, a registry not answering, kept the ages from being checked: a failure on
/// CI, and elsewhere a warning, as CI checks them.
fn unanswered(why: &str) -> Result<(), String> {
    if crate::parallel::on_ci() {
        Err(format!(
            "{why}, so the new package versions' ages can't be checked; run the check again \
             once it answers"
        ))
    } else {
        println!("xtask: warning: {why}, so package ages are not checked here; CI checks them");
        Ok(())
    }
}

/// What passed, in a sentence: `new` versions since `base`, `early` of them let in early.
fn summary(new: usize, early: usize, base: &str) -> String {
    if new == 0 {
        return format!("no package version is new since {base}");
    }
    let versions = counted(new, "package version", "package versions");
    if early == 0 {
        let each = if new == 1 {
            "published"
        } else {
            "each published"
        };
        return format!(
            "{versions} new since {base}, {each} at least {} days ago",
            days()
        );
    }
    format!(
        "{versions} new since {base}: {} published at least {} days ago, and {} let in early by \
         {ALLOW_LIST}",
        new - early,
        days(),
        early
    )
}

/// [`OLD_ENOUGH`] in days.
fn days() -> i64 {
    OLD_ENOUGH.as_hours() / 24
}

/// How long ago something `age` old was, in its largest whole unit: "2 days ago".
fn ago(age: SignedDuration) -> String {
    let (count, unit) = match age.as_secs() {
        secs if secs >= 86_400 => (secs / 86_400, "day"),
        secs if secs >= 3_600 => (secs / 3_600, "hour"),
        secs if secs >= 60 => (secs / 60, "minute"),
        _ => return "under a minute ago".to_owned(),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// `at` to the minute, in UTC: "2026-10-24 14:02 UTC".
fn when(at: Timestamp) -> String {
    at.strftime("%Y-%m-%d %H:%M UTC").to_string()
}

/// `count` things, `one` or `many` of them: "1 package version", "2 package versions".
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
mod tests;
