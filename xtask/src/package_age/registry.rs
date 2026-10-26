//! When a package version was published, as its registry says: crates.io's API, a version's
//! `created_at`, asked as its crawler policy asks (a User-Agent that says who is asking, at
//! most one request a second); and the npm registry's document for a package, its `time` for
//! each version. Asked with curl, as xtask asks git and cargo, so xtask builds no HTTP client
//! of its own for a few requests per change.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::Value;

use super::locked::Registry;

/// Who is asking, as crates.io's crawler policy asks every client to say.
const USER_AGENT: &str = "glave-xtask-package-age (https://github.com/glave-ai/glave)";

/// The least time between two requests to crates.io: its crawler policy's one a second.
const CRATES_IO_EVERY: Duration = Duration::from_secs(1);

/// What curl writes after the body: the HTTP status, three digits (`000` when none came).
const STATUS: &str = "%{http_code}";

/// Why a version's publication isn't known, in a sentence.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unknown {
    /// The registry couldn't be reached, or gave no answer this check can read.
    Unanswered(String),
    /// It answered, and doesn't list the version.
    Unlisted(String),
}

/// The registries, asked one request at a time.
#[derive(Default)]
pub(super) struct Registries {
    /// When crates.io was last asked.
    crates_io: Option<Instant>,
}

impl Registries {
    /// When each of `versions` of the package `name` was published to `registry`: one request
    /// for each version on crates.io, one for them all on npm.
    pub(super) fn published(
        &mut self,
        registry: Registry,
        name: &str,
        versions: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, Timestamp>, Unknown> {
        match registry {
            Registry::Crates => versions
                .iter()
                .map(|version| Ok((version.clone(), self.crate_published(name, version)?)))
                .collect(),
            Registry::Npm => {
                let url = format!("https://registry.npmjs.org/{}", npm_path(name)?);
                let body = get(registry, &url, name)?.ok_or_else(|| {
                    Unknown::Unlisted(format!("the npm registry has no package {name}"))
                })?;
                npm_times(&body, name, versions)
            }
        }
    }

    /// When `version` of the crate `name` was published, asked no sooner than
    /// [`CRATES_IO_EVERY`] after the last request.
    fn crate_published(&mut self, name: &str, version: &str) -> Result<Timestamp, Unknown> {
        let url = format!(
            "https://crates.io/api/v1/crates/{}/{}",
            plain(name)?,
            plain(version)?
        );
        if let Some(wait) = self
            .crates_io
            .and_then(|last| CRATES_IO_EVERY.checked_sub(last.elapsed()))
        {
            std::thread::sleep(wait);
        }
        let body = get(Registry::Crates, &url, &format!("{name} {version}"));
        self.crates_io = Some(Instant::now());
        let body =
            body?.ok_or_else(|| Unknown::Unlisted(format!("crates.io has no {name} {version}")))?;
        created_at(&body).ok_or_else(|| {
            Unknown::Unanswered(format!(
                "crates.io's answer for {name} {version} has no `created_at` this check can read"
            ))
        })
    }
}

/// What `registry` answered to a GET of `url`, which asks when `about` was published: the
/// body, or `None` when it has no such page.
fn get(registry: Registry, url: &str, about: &str) -> Result<Option<String>, Unknown> {
    let said = crate::cmd::query(
        "curl",
        &[
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "30",
            "--user-agent",
            USER_AGENT,
            "--write-out",
            STATUS,
            url,
        ],
    )
    .map_err(|err| {
        Unknown::Unanswered(format!(
            "could not reach {} to ask when {about} was published ({err})",
            registry.name()
        ))
    })?;
    answered(&said).map_err(|status| {
        Unknown::Unanswered(format!(
            "{} answered HTTP {status} when asked when {about} was published",
            registry.name()
        ))
    })
}

/// curl's output, a body and then [`STATUS`]: the body when the status is 200, `None` for a
/// 404, else the status.
fn answered(said: &str) -> Result<Option<String>, String> {
    let (body, status) = said
        .len()
        .checked_sub(3)
        .and_then(|at| said.split_at_checked(at))
        .unwrap_or(("", said));
    match status {
        "200" => Ok(Some(body.to_owned())),
        "404" => Ok(None),
        other => Err(other.to_owned()),
    }
}

/// When crates.io's answer for one version says it was published.
fn created_at(body: &str) -> Option<Timestamp> {
    let answer: Value = serde_json::from_str(body).ok()?;
    answer
        .pointer("/version/created_at")
        .and_then(Value::as_str)?
        .parse()
        .ok()
}

/// When each of `versions` of `name` was published, as the npm registry's document for the
/// package, `body`, says in its `time`.
fn npm_times(
    body: &str,
    name: &str,
    versions: &BTreeSet<String>,
) -> Result<BTreeMap<String, Timestamp>, Unknown> {
    let document: Value = serde_json::from_str(body).map_err(|err| {
        Unknown::Unanswered(format!(
            "the npm registry's answer for {name} is not JSON: {err}"
        ))
    })?;
    versions
        .iter()
        .map(|version| {
            document
                .get("time")
                .and_then(|time| time.get(version))
                .and_then(Value::as_str)
                .and_then(|at| at.parse().ok())
                .map(|at| (version.clone(), at))
                .ok_or_else(|| {
                    Unknown::Unlisted(format!("the npm registry lists no {name} {version}"))
                })
        })
        .collect()
}

/// `name` as the npm registry's path takes it: a scoped name's slash escaped
/// (`@angular%2fcore`).
fn npm_path(name: &str) -> Result<String, Unknown> {
    Ok(plain(name)?.replacen('/', "%2f", 1))
}

/// `text`, a package's name or version, when it holds only what names and versions may, so it
/// goes into a URL as it is.
fn plain(text: &str) -> Result<&str, Unknown> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~+@/".contains(c);
    if !text.is_empty() && text.chars().all(allowed) && !text.contains("..") {
        Ok(text)
    } else {
        Err(Unknown::Unlisted(format!(
            "`{text}` is not a package name or version a registry can be asked about"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(versions: &[&str]) -> BTreeSet<String> {
        versions
            .iter()
            .map(|version| (*version).to_owned())
            .collect()
    }

    #[test]
    fn curl_s_answer_is_its_body_then_its_status() {
        assert_eq!(answered("{\"a\":1}200"), Ok(Some("{\"a\":1}".to_owned())));
        assert_eq!(
            answered("line one\nline two\n200"),
            Ok(Some("line one\nline two\n".to_owned()))
        );
        assert_eq!(answered("{\"errors\":[]}404"), Ok(None));
        assert_eq!(answered("busy503"), Err("503".to_owned()));
        assert_eq!(answered("000"), Err("000".to_owned()));
        assert_eq!(answered(""), Err(String::new()));
    }

    #[test]
    fn crates_io_says_when_a_version_was_published() {
        let body =
            r#"{"version": {"num": "1.48.0", "created_at": "2026-10-24T14:02:11.069149+00:00"}}"#;
        assert_eq!(
            created_at(body),
            Some("2026-10-24T14:02:11.069149Z".parse().unwrap())
        );
        assert_eq!(created_at(r#"{"version": {"num": "1.48.0"}}"#), None);
        assert_eq!(created_at("not json"), None);
    }

    #[test]
    fn the_npm_registry_says_when_each_version_was_published() {
        let body = r#"{"name": "marked", "time": {"created": "2011-07-24T00:00:00.000Z",
            "18.0.13": "2026-09-01T10:00:00.000Z", "18.0.14": "2026-10-01T10:00:00.000Z"}}"#;
        let times = npm_times(body, "marked", &versions(&["18.0.13", "18.0.14"])).unwrap();
        assert_eq!(
            times.get("18.0.14"),
            Some(&"2026-10-01T10:00:00Z".parse().unwrap())
        );
        assert_eq!(times.len(), 2);
        let missing = npm_times(body, "marked", &versions(&["19.0.0"])).unwrap_err();
        assert_eq!(
            missing,
            Unknown::Unlisted("the npm registry lists no marked 19.0.0".to_owned())
        );
        assert!(matches!(
            npm_times("<html>", "marked", &versions(&["1.0.0"])),
            Err(Unknown::Unanswered(_))
        ));
    }

    #[test]
    fn only_a_plain_name_goes_into_a_url_and_a_scope_s_slash_is_escaped() {
        assert_eq!(npm_path("@angular/core"), Ok("@angular%2fcore".to_owned()));
        assert_eq!(npm_path("highlight.js"), Ok("highlight.js".to_owned()));
        assert_eq!(plain("1.0.0-rc.1+build.5"), Ok("1.0.0-rc.1+build.5"));
        for wrong in ["", "a b", "../../me", "x?y=1", "x#y", "a\nb"] {
            assert!(plain(wrong).is_err(), "{wrong:?}");
        }
    }
}
