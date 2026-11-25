//! The macOS sandbox profile for one project: Codex's deny-everything base, then reads everywhere
//! except secret locations, other terminals and, for someone else's commands, the person's own
//! folders, writes to the project and temp folder except protected paths, and the network only
//! when allowed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glave_ports::computer::{GIT_RUNS_CODE, PROTECTED, SandboxLevel, SandboxNetwork};

use super::folders_leading_to;
use super::shut::Shut;

const BASE: &str = include_str!("base.sbpl");
const PREFERENCES: &str = include_str!("preferences.sbpl");
const NETWORK: &str = include_str!("network.sbpl");

/// Every terminal's device but `/dev/tty`, the command's own controlling terminal, if it has
/// one: another's (the person's Terminal tab, Terminal.app's windows, an ssh session's) would
/// let a command read what is typed there. A terminal the command makes itself (`script`, a
/// test runner's) stays its own, through the base profile's `com.apple.sandbox.pty` rule.
const OTHER_TERMINALS: &str = r#"(require-not (regex #"^/dev/(tty|pty)."))"#;

/// When the network is allowed: the internet and local servers, and name lookups, but not
/// other Unix sockets (`docker.sock` is root on the machine; the ssh agent signs as the person).
const NETWORK_RULES: &str = r#"; network: allowed
(allow network-outbound (remote ip))
; Seatbelt names a remote host only as `*` or `localhost`, so cloud metadata (169.254.169.254)
; cannot be withheld here (specs/sandbox.md, "Cloud metadata").
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
    /// Read-only wherever the project is ([`kept_read_only`](super::kept_read_only)).
    pub kept: Vec<PathBuf>,
    /// Hidden: the home folder's credential stores and whatever else the computer hides, and
    /// where links among them lead.
    pub hidden: Vec<PathBuf>,
    /// glave's sandbox cache folder, readable and writable inside glave's own.
    pub cache: Option<PathBuf>,
    /// glave's own folder on commands' `PATH`, readable, never writable, inside glave's own.
    pub glave_on_path: Option<PathBuf>,
    /// For a worktree: the repository's shared git folder, where its commits go.
    pub git_common: Option<PathBuf>,
    /// For a worktree: its own folder in the shared one (`worktrees/<name>`).
    pub git_own: Option<PathBuf>,
    /// Whether commands share `/tmp`, readable and writable, as the person's do. Someone else's
    /// neither read nor write it (`RunsFor::SomeoneElse`): Seatbelt can't give them one of
    /// their own, so they use their own temp folder (`TMPDIR`) alone, and read neither
    /// `/var/tmp` nor the person's temp folder (`person_temp`), but that and the project.
    pub shared_tmp: bool,
    /// The person's own temp folder (`/var/folders/…/T`), which holds each command's.
    pub person_temp: Option<PathBuf>,
    /// The person's folders someone else's commands never read, and what stays readable there.
    pub shut: Shut,
    /// Whether commands read preferences through `cfprefsd`, which reads the person's files
    /// past every read rule: only the person's own, whose reads are everywhere but secrets
    /// (Codex's own note on it says as much).
    pub preferences: bool,
}

/// The parameter naming a command's own temp folder, made for it when it starts.
pub(crate) const TEMP_PARAM: &str = "TEMP";

/// A profile and the paths it names, passed as `sandbox-exec -p <text> -D<name>=<path>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Profile {
    pub text: String,
    pub params: Vec<(String, PathBuf)>,
}

pub(crate) fn profile(places: &Places, level: SandboxLevel, network: SandboxNetwork) -> Profile {
    let mut profile = Profile {
        text: format!("{BASE}\n"),
        params: Vec::new(),
    };
    if places.preferences {
        profile.text.push_str(PREFERENCES);
        profile.text.push('\n');
    }
    reads(&mut profile, places);
    writes(&mut profile, places, level);
    match network {
        SandboxNetwork::Off => {}
        SandboxNetwork::Direct => {
            profile.text.push_str(NETWORK_RULES);
            profile.text.push_str(NETWORK);
        }
    }
    profile
}

fn reads(profile: &mut Profile, places: &Places) {
    let mut conditions = vec![r#"(subpath "/")"#.to_owned(), OTHER_TERMINALS.to_owned()];
    if !places.shared_tmp {
        // But for their own temp folder and workspace, where the system's temp folder is in
        // one of these (`TMPDIR=/tmp/...`, as `cargo xtask ci` sets it).
        for shared in ["/private/tmp", "/private/var/tmp"] {
            conditions.push(format!(
                r#"(require-any (require-not (subpath "{shared}")) (subpath (param "TEMP")) (subpath (param "ROOT")))"#
            ));
        }
        if let Some(person_temp) = &places.person_temp {
            conditions.push(
                r#"(require-any (require-not (subpath (param "PERSON_TEMP"))) (subpath (param "TEMP")) (subpath (param "ROOT")))"#
                    .to_owned(),
            );
            profile
                .params
                .push(("PERSON_TEMP".into(), person_temp.clone()));
        }
    }
    shut(profile, &mut conditions, places);
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
        let on_path = if places.glave_on_path.is_some() {
            r#" (subpath (param "ON_PATH"))"#
        } else {
            ""
        };
        // Not a workspace that holds glave's folder, though: that's no worktree of glave's.
        let workspace = if glave_home.starts_with(&places.root) {
            ""
        } else {
            r#" (subpath (param "ROOT"))"#
        };
        conditions.push(format!(
            r#"(require-any (require-not (subpath (param "GLAVE_HOME"))){workspace}{cache}{on_path})"#
        ));
        profile
            .params
            .push(("GLAVE_HOME".into(), glave_home.clone()));
        if let Some(on_path) = &places.glave_on_path {
            profile.params.push(("ON_PATH".into(), on_path.clone()));
        }
    }
    let _ = writeln!(
        profile.text,
        "; reads: everywhere except other terminals and secret locations\n(allow file-read*\n  (require-all\n    {}))",
        conditions.join("\n    ")
    );
    shut_on_the_way(profile, places);
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

/// The person's folders hidden whole from someone else's commands, but for what is theirs in
/// them (the workspace, the command's own temp folder, glave's folder on `PATH`) and what stays
/// readable there (`OPEN_<n>`).
fn shut(profile: &mut Profile, conditions: &mut Vec<String>, places: &Places) {
    if places.shut.folders.is_empty() {
        return;
    }
    let mut theirs = vec![
        r#"(subpath (param "ROOT"))"#.to_owned(),
        format!(r#"(subpath (param "{TEMP_PARAM}"))"#),
    ];
    // Named by glave's folder's own rule, which comes with glave's folder.
    if places.glave_home.is_some() && places.glave_on_path.is_some() {
        theirs.push(r#"(subpath (param "ON_PATH"))"#.to_owned());
    }
    for (index, open) in places.shut.open.iter().enumerate() {
        let name = format!("OPEN_{index}");
        theirs.push(format!(r#"(subpath (param "{name}"))"#));
        profile.params.push((name, open.clone()));
    }
    let theirs = theirs.join(" ");
    for (index, folder) in places.shut.folders.iter().enumerate() {
        let name = format!("SHUT_{index}");
        conditions.push(format!(
            r#"(require-any (require-not (subpath (param "{name}"))) {theirs})"#
        ));
        profile.params.push((name, folder.clone()));
    }
}

/// That each folder hidden from someone else's commands is a folder, and so is each between it
/// and what stays readable in it, not what they hold: a tool resolving a path there looks at
/// each folder on the way.
fn shut_on_the_way(profile: &mut Profile, places: &Places) {
    let readable = std::iter::once(&places.root)
        .chain(places.glave_on_path.iter())
        .chain(places.shut.open.iter());
    let mut folders: Vec<&Path> = places.shut.folders.iter().map(PathBuf::as_path).collect();
    for path in readable {
        for folder in &places.shut.folders {
            folders.extend(
                path.ancestors()
                    .skip(1)
                    .take_while(|up| up.starts_with(folder) && *up != folder.as_path()),
            );
        }
    }
    folders.sort();
    folders.dedup();
    if folders.is_empty() {
        return;
    }
    let mut literals = Vec::new();
    for (index, folder) in folders.into_iter().enumerate() {
        let name = format!("SHUT_UP_{index}");
        literals.push(format!(r#"(literal (param "{name}"))"#));
        profile.params.push((name, folder.to_path_buf()));
    }
    let _ = writeln!(
        profile.text,
        "(allow file-read-metadata {})",
        literals.join(" ")
    );
}

fn writes(profile: &mut Profile, places: &Places, level: SandboxLevel) {
    profile.params.push(("ROOT".into(), places.root.clone()));
    // `TEMP`, each command's own temp folder, is passed with each command (`TEMP_PARAM`).
    // Temp folders never grant the project itself: a project that lives inside one keeps its
    // protections, and at `read-only` stays unwritable.
    profile.text.push_str(
        "; writes: the run's temp folder and output devices, never the project through them\n\
         (allow file-write* (require-all (subpath (param \"TEMP\")) (require-not (subpath (param \"ROOT\")))))\n\
         (allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n",
    );
    if places.shared_tmp {
        profile.text.push_str(
            "; writes: the shared /tmp, never the project through it\n\
             (allow file-write* (require-all (subpath \"/private/tmp\") (require-not (subpath (param \"ROOT\")))))\n",
        );
    }
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
    let protected = PROTECTED.iter().map(|path| places.root.join(path)).chain(
        GIT_RUNS_CODE
            .iter()
            .chain(NESTED_GIT)
            .map(|name| git.join(name)),
    );
    unless(profile, &mut conditions, "PROTECTED", protected);
    unless(
        profile,
        &mut conditions,
        "KEPT",
        places.kept.iter().cloned(),
    );
    for (index, folder) in folders_leading_to(&places.root, &places.kept).enumerate() {
        let name = format!("KEPT_UP_{index}");
        conditions.push(format!(r#"(require-not (literal (param "{name}")))"#));
        profile.params.push((name, folder));
    }
    allow_writes(profile, "the project", &conditions);
    if let Some(common) = &places.git_common {
        git_common_writes(profile, common, places.git_own.as_deref());
    }
}

/// Inside a git folder: the folders of other worktrees and of submodules, each a git folder
/// with hooks and config of its own. Whole, since a folder moved in whole brings its own.
const NESTED_GIT: &[&str] = &["worktrees", "modules"];

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

#[cfg(test)]
mod tests {
    use super::super::secrets::{HOME_SECRETS, MACOS_HOME_SECRETS};
    use super::*;

    fn places() -> Places {
        Places {
            root: PathBuf::from("/Users/me/my.project"),
            glave_home: Some(PathBuf::from("/Users/me/.glave")),
            kept: Vec::new(),
            hidden: HOME_SECRETS
                .iter()
                .chain(MACOS_HOME_SECRETS)
                .map(|secret| Path::new("/Users/me").join(secret))
                .collect(),
            cache: Some(PathBuf::from("/Users/me/.glave/sandbox-cache")),
            glave_on_path: Some(PathBuf::from("/Users/me/.glave/bin")),
            git_common: None,
            git_own: None,
            shared_tmp: true,
            person_temp: None,
            shut: Shut::default(),
            preferences: true,
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
        let profile = profile(&places(), SandboxLevel::Project, SandboxNetwork::Off);
        assert!(profile.text.starts_with("; From OpenAI Codex"));
        assert!(profile.text.contains("(deny default)"));
        assert_eq!(
            param(&profile, "ROOT"),
            Some(Path::new("/Users/me/my.project"))
        );
        let protected: Vec<&Path> = (0..PROTECTED.len() + GIT_RUNS_CODE.len() + NESTED_GIT.len())
            .filter_map(|i| param(&profile, &format!("PROTECTED_{i}")))
            .collect();
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/hooks")));
        assert!(protected.contains(&Path::new("/Users/me/my.project/.git/commondir")));
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
        let profile = profile(&places(), SandboxLevel::ReadOnly, SandboxNetwork::Off);
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
        let profile = profile(&places(), SandboxLevel::Project, SandboxNetwork::Off);
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
            r#"(require-any (require-not (subpath (param "GLAVE_HOME"))) (subpath (param "ROOT")) (subpath (param "CACHE")) (subpath (param "ON_PATH")))"#
        ));
        assert_eq!(
            param(&profile, "CACHE"),
            Some(Path::new("/Users/me/.glave/sandbox-cache"))
        );
        assert_eq!(
            param(&profile, "ON_PATH"),
            Some(Path::new("/Users/me/.glave/bin"))
        );
        assert!(
            !profile
                .text
                .contains(r#"file-write* (require-all (subpath (param "ON_PATH"))"#),
            "glave's folder on PATH is read, never written"
        );
        assert!(profile.text.contains(
            r#"(allow file-write* (require-all (subpath (param "CACHE")) (require-not (literal (param "CACHE")))))"#
        ));
        assert_eq!(param(&profile, "COMMON"), None, "not a worktree");
        let mut worktree = places();
        worktree.git_common = Some(PathBuf::from("/Users/me/code/app/.git"));
        worktree.git_own = Some(PathBuf::from("/Users/me/code/app/.git/worktrees/w"));
        let profile = super::profile(&worktree, SandboxLevel::Project, SandboxNetwork::Off);
        assert_eq!(
            param(&profile, "COMMON_PROTECTED_0"),
            Some(Path::new("/Users/me/code/app/.git/hooks"))
        );
        assert_eq!(
            param(&profile, "COMMON_PROTECTED_3"),
            Some(Path::new("/Users/me/code/app/.git/commondir"))
        );
        assert_eq!(
            param(&profile, "COMMON_PROTECTED_4"),
            Some(Path::new("/Users/me/code/app/.git/worktrees"))
        );
        assert_eq!(
            param(&profile, "OWN_PROTECTED_3"),
            Some(Path::new("/Users/me/code/app/.git/worktrees/w/commondir"))
        );
        assert_eq!(
            param(&profile, "OWN_PROTECTED_4"),
            Some(Path::new("/Users/me/code/app/.git/worktrees/w/modules"))
        );
        assert!(
            profile
                .text
                .contains(r#"(require-not (literal (param "OWN")))"#)
        );
        assert!(
            !profile.text.contains("[eE][nN][vV]"),
            "the project's own env files stay readable"
        );
    }

    #[test]
    fn a_worktree_in_glave_s_folder_may_see_the_folders_above_it_but_not_into_them() {
        let mut worktree = places();
        worktree.root = PathBuf::from("/Users/me/.glave/worktrees/app-0a1b/glave-0123");
        let profile = profile(&worktree, SandboxLevel::Project, SandboxNetwork::Off);
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
        let elsewhere = super::profile(&places(), SandboxLevel::Project, SandboxNetwork::Off);
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
        let profile = profile(&places(), SandboxLevel::Project, SandboxNetwork::Direct);
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
        assert!(
            profile.text.contains("mDNSResponder"),
            "names are looked up"
        );
    }

    #[test]
    fn someone_else_s_commands_read_nothing_of_the_person_s_folders_but_toolchains() {
        let owner = profile(&places(), SandboxLevel::Project, SandboxNetwork::Off);
        assert!(!owner.text.contains("SHUT_"), "the person's own read them");
        assert!(
            owner.text.contains("cfprefsd"),
            "the person's own read preferences"
        );
        let mut someone = places();
        someone.preferences = false;
        someone.shared_tmp = false;
        someone.root = PathBuf::from("/Users/me/.glave/scratch/t");
        someone.shut = Shut {
            folders: vec![PathBuf::from("/Users/me"), PathBuf::from("/srv/projects")],
            open: vec![PathBuf::from("/Users/me/.cargo")],
            hidden: Vec::new(),
        };
        let profile = profile(&someone, SandboxLevel::Project, SandboxNetwork::Off);
        assert_eq!(param(&profile, "SHUT_0"), Some(Path::new("/Users/me")));
        assert_eq!(param(&profile, "SHUT_1"), Some(Path::new("/srv/projects")));
        assert_eq!(
            param(&profile, "OPEN_0"),
            Some(Path::new("/Users/me/.cargo"))
        );
        let theirs = format!(
            r#"(subpath (param "ROOT")) (subpath (param "{TEMP_PARAM}")) (subpath (param "ON_PATH")) (subpath (param "OPEN_0")))"#
        );
        for shut in ["SHUT_0", "SHUT_1"] {
            let rule = format!(r#"(require-any (require-not (subpath (param "{shut}"))) {theirs}"#);
            assert!(profile.text.contains(&rule), "{shut}: {}", profile.text);
        }
        let on_the_way: Vec<&Path> = (0..)
            .map_while(|index| param(&profile, &format!("SHUT_UP_{index}")))
            .collect();
        assert_eq!(
            on_the_way,
            [
                "/Users/me",
                "/Users/me/.glave",
                "/Users/me/.glave/scratch",
                "/srv/projects"
            ]
            .map(Path::new),
            "folders, never what they hold"
        );
        assert!(
            profile
                .text
                .contains("(allow file-read-metadata (literal (param \"SHUT_UP_0\"))")
        );
        assert!(
            !profile.text.contains("cfprefsd"),
            "nor theirs, preferences"
        );
        assert!(
            profile.text.contains(&format!(
                r#"(require-any (require-not (subpath "/private/tmp")) (subpath (param "{TEMP_PARAM}")) (subpath (param "ROOT")))"#
            )),
            "their own temp folder stays theirs where the system's is in /tmp"
        );
    }

    #[test]
    fn no_profile_reaches_the_data_protection_keychain() {
        for level in [SandboxLevel::ReadOnly, SandboxLevel::Project] {
            for network in [SandboxNetwork::Off, SandboxNetwork::Direct] {
                let profile = profile(&places(), level, network);
                assert!(!profile.text.contains("securityd"), "{level:?} {network:?}");
            }
        }
    }
}
