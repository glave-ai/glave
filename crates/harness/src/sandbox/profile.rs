//! The macOS sandbox profile for one project: Codex's deny-everything base, then reads everywhere
//! except secret locations, writes to the project and temp folder except protected paths, and
//! the network only when allowed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::{PROTECTED, SandboxLevel};

const BASE: &str = include_str!("base.sbpl");
const PREFERENCES: &str = include_str!("preferences.sbpl");
const NETWORK: &str = include_str!("network.sbpl");

/// Under the home folder: where credentials live. Never readable from the sandbox.
const HOME_SECRETS: &[&str] = &[
    ".ssh",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".docker/config.json",
    ".gnupg",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Firefox",
];

/// Secret files by name, the rule the read tool uses: env files, unless templates...
const ENV_FILES: &str = r"(\.env|\.env\.[^/]*|[^/]*\.env)";
const TEMPLATE_WORDS: &str = "(example|sample|template|dist|defaults)";
/// ...and keys and credential files.
const OTHER_SECRET_FILES: &str = r"(\.envrc|\.netrc|\.npmrc|\.pypirc|\.pgpass|\.git-credentials|\.htpasswd|credentials|credentials\.json|id_rsa|id_dsa|id_ecdsa|id_ed25519|secrets\.[^/]*|[^/]*\.(pem|key|p12|pfx|jks|keystore))";

/// When the network is allowed: the internet and local servers, and name lookups, but not
/// other Unix sockets (`docker.sock` is root on the machine; the ssh agent signs as the person).
const NETWORK_RULES: &str = r#"; network: allowed
(allow network-outbound (remote ip))
(allow network-inbound (local ip))
(allow network-bind (local ip))
(allow network-outbound (remote unix-socket (literal "/private/var/run/mDNSResponder")))
(allow mach-lookup (global-name "com.apple.TrustEvaluationAgent"))
"#;

/// The real paths a profile refers to. macOS matches resolved paths, so these must be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Places {
    pub root: PathBuf,
    pub temp: PathBuf,
    pub home: Option<PathBuf>,
    pub glave_home: Option<PathBuf>,
}

/// A profile and the paths it names, passed as `sandbox-exec -p <text> -D<name>=<path>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Profile {
    pub text: String,
    pub params: Vec<(String, PathBuf)>,
}

pub(crate) fn profile(places: &Places, level: SandboxLevel, network: bool) -> Profile {
    let mut profile = Profile {
        text: format!("{BASE}\n{PREFERENCES}\n"),
        params: Vec::new(),
    };
    reads(&mut profile, places);
    writes(&mut profile, places, level);
    if network {
        profile.text.push_str(NETWORK_RULES);
        profile.text.push_str(NETWORK);
    }
    profile
}

fn reads(profile: &mut Profile, places: &Places) {
    let mut conditions = vec![r#"(subpath "/")"#.to_owned()];
    let home_secrets = places
        .home
        .iter()
        .flat_map(|home| HOME_SECRETS.iter().map(move |secret| home.join(secret)));
    for (index, path) in home_secrets.chain(places.glave_home.clone()).enumerate() {
        let name = format!("SECRET_{index}");
        conditions.push(format!(r#"(require-not (subpath (param "{name}")))"#));
        profile.params.push((name, path));
    }
    let root = regex_escape(&places.root);
    let (env, templates, others) = (
        any_case(ENV_FILES),
        any_case(TEMPLATE_WORDS),
        any_case(OTHER_SECRET_FILES),
    );
    conditions.push(format!(
        r#"(require-not (regex #"^{root}/(.*/)?{others}$"))"#
    ));
    conditions.push(format!(
        r#"(require-any (require-not (regex #"^{root}/(.*/)?{env}$")) (regex #"^{root}/(.*/)?[^/]*{templates}[^/]*$"))"#
    ));
    let _ = writeln!(
        profile.text,
        "; reads: everywhere except secret locations\n(allow file-read*\n  (require-all\n    {}))",
        conditions.join("\n    ")
    );
}

fn writes(profile: &mut Profile, places: &Places, level: SandboxLevel) {
    profile.params.push(("TEMP".into(), places.temp.clone()));
    profile.text.push_str(
        "; writes: the temp folder and output devices\n\
         (allow file-write* (subpath (param \"TEMP\")))\n\
         (allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n",
    );
    if level != SandboxLevel::Project {
        return;
    }
    profile.params.push(("ROOT".into(), places.root.clone()));
    profile
        .params
        .push(("GIT".into(), places.root.join(".git")));
    let mut conditions = vec![
        r#"(subpath (param "ROOT"))"#.to_owned(),
        r#"(require-not (literal (param "GIT")))"#.to_owned(),
    ];
    for (index, path) in PROTECTED.iter().enumerate() {
        let name = format!("PROTECTED_{index}");
        // Both forms: `subpath` alone leaves room to create the protected path itself.
        conditions.push(format!(r#"(require-not (literal (param "{name}")))"#));
        conditions.push(format!(r#"(require-not (subpath (param "{name}")))"#));
        profile.params.push((name, places.root.join(path)));
    }
    let root = regex_escape(&places.root);
    conditions.push(format!(
        r#"(require-not (regex #"^{root}/\.git/(worktrees|modules)/.+/(hooks(/.*)?|config(\.worktree)?)$"))"#
    ));
    let _ = writeln!(
        profile.text,
        "; writes: the project, except what runs code later\n(allow file-write*\n  (require-all\n    {}))",
        conditions.join("\n    ")
    );
}

/// `path` as a literal inside a profile regular expression.
fn regex_escape(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if r#"\.+*?()|[]{}^$""#.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A pattern that matches whatever the letters' case: `env` becomes `[eE][nN][vV]`.
fn any_case(pattern: &str) -> String {
    let mut out = String::new();
    for c in pattern.chars() {
        if c.is_ascii_alphabetic() {
            out.push('[');
            out.push(c.to_ascii_lowercase());
            out.push(c.to_ascii_uppercase());
            out.push(']');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            root: PathBuf::from("/Users/me/my.project"),
            temp: PathBuf::from("/private/var/folders/x/T"),
            home: Some(PathBuf::from("/Users/me")),
            glave_home: Some(PathBuf::from("/Users/me/.glave")),
        }
    }

    fn param<'a>(profile: &'a Profile, name: &str) -> Option<&'a Path> {
        profile
            .params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, path)| path.as_path())
    }

    #[test]
    fn the_project_level_writes_the_project_but_not_what_runs_code() {
        let profile = profile(&places(), SandboxLevel::Project, false);
        assert!(profile.text.starts_with("; From OpenAI Codex"));
        assert!(profile.text.contains("(deny default)"));
        assert_eq!(
            param(&profile, "ROOT"),
            Some(Path::new("/Users/me/my.project"))
        );
        let protected: Vec<&Path> = (0..PROTECTED.len())
            .filter_map(|i| param(&profile, &format!("PROTECTED_{i}")))
            .collect();
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/hooks")));
        assert!(protected.contains(&Path::new("/Users/me/my.project/.glave")));
        assert!(
            profile
                .text
                .contains(r"^/Users/me/my\.project/\.git/(worktrees|modules)")
        );
        assert!(!profile.text.contains("network-outbound"), "network off");
    }

    #[test]
    fn read_only_writes_only_the_temp_folder() {
        let profile = profile(&places(), SandboxLevel::ReadOnly, false);
        assert_eq!(param(&profile, "ROOT"), None);
        assert_eq!(
            param(&profile, "TEMP"),
            Some(Path::new("/private/var/folders/x/T"))
        );
    }

    #[test]
    fn secret_locations_are_unreadable() {
        let profile = profile(&places(), SandboxLevel::Project, false);
        let secrets: Vec<&Path> = profile
            .params
            .iter()
            .filter(|(name, _)| name.starts_with("SECRET_"))
            .map(|(_, path)| path.as_path())
            .collect();
        assert!(secrets.contains(&Path::new("/Users/me/.ssh")));
        assert!(secrets.contains(&Path::new("/Users/me/.glave")));
        assert!(profile.text.contains("[eE][nN][vV]"), "env files, any case");
    }

    #[test]
    fn the_network_is_added_only_when_allowed() {
        let profile = profile(&places(), SandboxLevel::Project, true);
        assert!(
            profile
                .text
                .contains("(allow network-outbound (remote ip))")
        );
        assert!(
            profile
                .text
                .contains("com.apple.SystemConfiguration.DNSConfiguration")
        );
    }

    #[test]
    fn paths_are_escaped_for_regular_expressions() {
        assert_eq!(regex_escape(Path::new("/a.b/(c)")), r"/a\.b/\(c\)");
        assert_eq!(any_case("ab.c"), "[aA][bB].[cC]");
    }
}
