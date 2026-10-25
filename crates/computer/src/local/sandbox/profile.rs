//! The macOS sandbox profile for one project: Codex's deny-everything base, then reads everywhere
//! except secret locations, writes to the project and temp folder except protected paths, and
//! the network only when allowed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glave_ports::computer::{PROTECTED, SandboxLevel};

use super::secrets::{ENV_FILES, HOME_SECRETS, OTHER_SECRET_FILES, TEMPLATE_WORDS};

const BASE: &str = include_str!("base.sbpl");
const PREFERENCES: &str = include_str!("preferences.sbpl");
const NETWORK: &str = include_str!("network.sbpl");

/// Under the home folder, on macOS besides the common list: the keychains, and browser
/// profiles (cookies are credentials).
const MACOS_HOME_SECRETS: &[&str] = &[
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Firefox",
    "Library/Application Support/ngrok",
];

/// The home folder's credential stores, as paths: hidden whether they exist yet or not.
pub(crate) fn home_secrets(home: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    HOME_SECRETS
        .iter()
        .chain(MACOS_HOME_SECRETS)
        .map(move |secret| home.join(secret))
}

/// When the network is allowed: the internet and local servers, and name lookups, but not
/// other Unix sockets (`docker.sock` is root on the machine; the ssh agent signs as the person).
const NETWORK_RULES: &str = r#"; network: allowed
(allow network-outbound (remote ip))
; Seatbelt names a remote host only as `*` or `localhost`, so cloud metadata (169.254.169.254)
; cannot be withheld here: that needs the outbound proxy (specs/sandbox.md, "Cloud metadata").
(allow network-inbound (local ip))
(allow network-bind (local ip))
(allow network-outbound (remote unix-socket (literal "/private/var/run/mDNSResponder")))
(allow mach-lookup (global-name "com.apple.TrustEvaluationAgent"))
"#;

/// The real paths a profile refers to. macOS matches resolved paths, so these must be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Places {
    pub root: PathBuf,
    pub glave_home: Option<PathBuf>,
    /// Credential stores, hidden: the home folder's, and where links among them lead.
    pub hidden: Vec<PathBuf>,
    /// glave's sandbox cache folder, readable and writable inside glave's own.
    pub cache: Option<PathBuf>,
    /// For a worktree: the repository's shared git folder, where its commits go.
    pub git_common: Option<PathBuf>,
    /// For a worktree: its own folder in the shared one (`worktrees/<name>`).
    pub git_own: Option<PathBuf>,
}

/// The parameter naming a command's own temp folder, made for it when it starts.
pub(crate) const TEMP_PARAM: &str = "TEMP";

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
    for (index, path) in places.hidden.iter().cloned().enumerate() {
        let name = format!("SECRET_{index}");
        conditions.push(format!(r#"(require-not (subpath (param "{name}")))"#));
        profile.params.push((name, path));
    }
    if let Some(glave_home) = &places.glave_home {
        // glave's own folder, except the workspace when it lives there (a thread's worktree)
        // and the sandbox's cache folder.
        let cache = if places.cache.is_some() {
            r#" (subpath (param "CACHE"))"#
        } else {
            ""
        };
        conditions.push(format!(
            r#"(require-any (require-not (subpath (param "GLAVE_HOME"))) (subpath (param "ROOT")){cache})"#
        ));
        profile
            .params
            .push(("GLAVE_HOME".into(), glave_home.clone()));
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
    if let Some(glave_home) = &places.glave_home {
        // That glave's folder is a folder, not what it holds, and so is each folder between it
        // and the workspace when the workspace lives there (a thread's worktree, under
        // `worktrees/<repository>`): `mkdir -p` checks each folder on the way to the cache or
        // a worktree, and git resolves a worktree's real path one folder at a time. Metadata
        // only: what these folders hold stays unreadable.
        let between =
            places.root.ancestors().skip(1).take_while(|folder| {
                *folder != glave_home.as_path() && folder.starts_with(glave_home)
            });
        let mut folders = vec![r#"(literal (param "GLAVE_HOME"))"#.to_owned()];
        for (index, folder) in between.enumerate() {
            let name = format!("ROOT_UP_{index}");
            folders.push(format!(r#"(literal (param "{name}"))"#));
            profile.params.push((name, folder.to_path_buf()));
        }
        let _ = writeln!(
            profile.text,
            "(allow file-read-metadata {})",
            folders.join(" ")
        );
    }
}

fn writes(profile: &mut Profile, places: &Places, level: SandboxLevel) {
    profile.params.push(("ROOT".into(), places.root.clone()));
    // `TEMP`, each command's own temp folder, is passed with each command (`TEMP_PARAM`).
    // Temp folders never grant the project itself: a project that lives inside one keeps its
    // protections, and at `read-only` stays unwritable.
    profile.text.push_str(
        "; writes: the run's temp folder, /tmp and output devices, never the project through them\n\
         (allow file-write* (require-all (subpath (param \"TEMP\")) (require-not (subpath (param \"ROOT\")))))\n\
         (allow file-write* (require-all (subpath \"/private/tmp\") (require-not (subpath (param \"ROOT\")))))\n\
         (allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n",
    );
    if let Some(cache) = &places.cache {
        // What is inside it, never the folder itself, which a link could replace.
        profile.params.push(("CACHE".into(), cache.clone()));
        profile.text.push_str(
            "; writes: the sandbox's package caches, kept between runs\n\
             (allow file-write* (require-all (subpath (param \"CACHE\")) (require-not (literal (param \"CACHE\")))))\n",
        );
    }
    if level != SandboxLevel::Project {
        return;
    }
    profile
        .params
        .push(("GIT".into(), places.root.join(".git")));
    // Never the folders themselves: Seatbelt matches a rename by its two paths, not by what
    // the moved folder holds, so the project or `.git` swapped for a prepared copy would
    // bring its own hooks past every rule below.
    let mut conditions = vec![
        r#"(subpath (param "ROOT"))"#.to_owned(),
        r#"(require-not (literal (param "ROOT")))"#.to_owned(),
        r#"(require-not (literal (param "GIT")))"#.to_owned(),
    ];
    let git = places.root.join(".git");
    let protected = PROTECTED
        .iter()
        .map(|path| places.root.join(path))
        .chain(NESTED_GIT.iter().map(|name| git.join(name)));
    unless(profile, &mut conditions, "PROTECTED", protected);
    allow_writes(profile, "the project", &conditions);
    if let Some(common) = &places.git_common {
        git_common_writes(profile, common, places.git_own.as_deref());
    }
}

/// Inside a git folder: the folders of other worktrees and of submodules, each a git folder
/// with hooks and config of its own. Whole, since a folder moved in whole brings its own.
const NESTED_GIT: &[&str] = &["worktrees", "modules"];

/// Inside any git folder: what runs code later.
const GIT_RUNS_CODE: &[&str] = &["hooks", "config", "config.worktree"];

/// A worktree's repository's shared git folder: its objects, refs and logs, so commits work,
/// but never its hooks or config, which would run code in every checkout of it, nor other
/// worktrees' or submodules' folders. Its own folder there (`worktrees/<name>`) is writable
/// but for its hooks, config and submodules, and cannot itself be replaced.
fn git_common_writes(profile: &mut Profile, common: &Path, own: Option<&Path>) {
    profile.params.push(("COMMON".into(), common.to_owned()));
    let mut conditions = vec![
        r#"(subpath (param "COMMON"))"#.to_owned(),
        r#"(require-not (literal (param "COMMON")))"#.to_owned(),
    ];
    let protected = GIT_RUNS_CODE
        .iter()
        .chain(NESTED_GIT)
        .map(|name| common.join(name));
    unless(profile, &mut conditions, "COMMON_PROTECTED", protected);
    allow_writes(profile, "the repository's shared git folder", &conditions);
    let Some(own) = own else {
        return;
    };
    profile.params.push(("OWN".into(), own.to_owned()));
    let mut conditions = vec![
        r#"(subpath (param "OWN"))"#.to_owned(),
        r#"(require-not (literal (param "OWN")))"#.to_owned(),
    ];
    let protected = GIT_RUNS_CODE
        .iter()
        .chain(&["modules"])
        .map(|name| own.join(name));
    unless(profile, &mut conditions, "OWN_PROTECTED", protected);
    allow_writes(profile, "the worktree's own git folder", &conditions);
}

/// Leaves out each of `paths` and everything below it, as parameters `<prefix>_<index>`.
fn unless(
    profile: &mut Profile,
    conditions: &mut Vec<String>,
    prefix: &str,
    paths: impl Iterator<Item = PathBuf>,
) {
    for (index, path) in paths.enumerate() {
        let name = format!("{prefix}_{index}");
        // Both forms: `subpath` alone leaves room to create the protected path itself.
        conditions.push(format!(r#"(require-not (literal (param "{name}")))"#));
        conditions.push(format!(r#"(require-not (subpath (param "{name}")))"#));
        profile.params.push((name, path));
    }
}

fn allow_writes(profile: &mut Profile, what: &str, conditions: &[String]) {
    let _ = writeln!(
        profile.text,
        "; writes: {what}, except what runs code later\n(allow file-write*\n  (require-all\n    {}))",
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
            glave_home: Some(PathBuf::from("/Users/me/.glave")),
            hidden: home_secrets(Path::new("/Users/me")).collect(),
            cache: Some(PathBuf::from("/Users/me/.glave/sandbox-cache")),
            git_common: None,
            git_own: None,
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
        let protected: Vec<&Path> = (0..PROTECTED.len() + NESTED_GIT.len())
            .filter_map(|i| param(&profile, &format!("PROTECTED_{i}")))
            .collect();
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/hooks")));
        assert!(protected.contains(&Path::new("/Users/me/my.project/.glave")));
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/modules")));
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/worktrees")));
        assert!(
            profile
                .text
                .contains(r#"(require-not (literal (param "ROOT")))"#),
            "the project itself cannot be swapped"
        );
        assert!(!profile.text.contains("network-outbound"), "network off");
    }

    #[test]
    fn read_only_writes_only_the_temp_folder() {
        let profile = profile(&places(), SandboxLevel::ReadOnly, false);
        assert!(!profile.text.contains("; writes: the project"));
        assert!(
            profile
                .text
                .contains(r#"(require-not (subpath (param "ROOT")))"#)
        );
        assert!(
            profile
                .text
                .contains(&format!(r#"(subpath (param "{TEMP_PARAM}"))"#))
        );
        assert_eq!(param(&profile, TEMP_PARAM), None, "given with each command");
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
        for path in [
            "/Users/me/.ssh",
            "/Users/me/.config/gh",
            "/Users/me/.cargo/credentials.toml",
            "/Users/me/.zsh_history",
            "/Users/me/Library/Keychains",
        ] {
            assert!(secrets.contains(&Path::new(path)), "{path}");
        }
        assert!(
            !secrets.contains(&Path::new("/Users/me/.cargo")),
            "Cargo's home stays readable"
        );
        assert_eq!(
            param(&profile, "GLAVE_HOME"),
            Some(Path::new("/Users/me/.glave"))
        );
        assert!(profile.text.contains(
            r#"(require-any (require-not (subpath (param "GLAVE_HOME"))) (subpath (param "ROOT")) (subpath (param "CACHE")))"#
        ));
        assert_eq!(
            param(&profile, "CACHE"),
            Some(Path::new("/Users/me/.glave/sandbox-cache"))
        );
        assert!(profile.text.contains(
            r#"(allow file-write* (require-all (subpath (param "CACHE")) (require-not (literal (param "CACHE")))))"#
        ));
        assert_eq!(param(&profile, "COMMON"), None, "not a worktree");
        let mut worktree = places();
        worktree.git_common = Some(PathBuf::from("/Users/me/code/app/.git"));
        worktree.git_own = Some(PathBuf::from("/Users/me/code/app/.git/worktrees/w"));
        let profile = super::profile(&worktree, SandboxLevel::Project, false);
        assert_eq!(
            param(&profile, "COMMON_PROTECTED_0"),
            Some(Path::new("/Users/me/code/app/.git/hooks"))
        );
        assert_eq!(
            param(&profile, "COMMON_PROTECTED_3"),
            Some(Path::new("/Users/me/code/app/.git/worktrees"))
        );
        assert_eq!(
            param(&profile, "OWN_PROTECTED_3"),
            Some(Path::new("/Users/me/code/app/.git/worktrees/w/modules"))
        );
        assert!(
            profile
                .text
                .contains(r#"(require-not (literal (param "OWN")))"#)
        );
        assert!(profile.text.contains("[eE][nN][vV]"), "env files, any case");
    }

    #[test]
    fn a_worktree_in_glave_s_folder_may_see_the_folders_above_it_but_not_into_them() {
        let mut worktree = places();
        worktree.root = PathBuf::from("/Users/me/.glave/worktrees/app-0a1b/glave-0123");
        let profile = profile(&worktree, SandboxLevel::Project, false);
        assert_eq!(
            param(&profile, "ROOT_UP_0"),
            Some(Path::new("/Users/me/.glave/worktrees/app-0a1b"))
        );
        assert_eq!(
            param(&profile, "ROOT_UP_1"),
            Some(Path::new("/Users/me/.glave/worktrees"))
        );
        assert_eq!(
            param(&profile, "ROOT_UP_2"),
            None,
            "glave's folder is named already"
        );
        assert!(profile.text.contains(
            r#"(allow file-read-metadata (literal (param "GLAVE_HOME")) (literal (param "ROOT_UP_0")) (literal (param "ROOT_UP_1")))"#
        ));
        let elsewhere = super::profile(&places(), SandboxLevel::Project, false);
        assert_eq!(
            param(&elsewhere, "ROOT_UP_0"),
            None,
            "a project outside glave's folder"
        );
        assert!(
            elsewhere
                .text
                .contains(r#"(allow file-read-metadata (literal (param "GLAVE_HOME")))"#)
        );
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
