//! The commands that need the person in any mode that asks: removing what cannot be got back,
//! discarding uncommitted work, running fetched code, acting as the administrator, planting
//! something that runs later, and reading secrets. Each says why in plain words.

use std::path::Path;

use crate::credentials::prints_credential;
use crate::forge::forge;
use crate::judge::{Caution, Context, Kind};
use crate::parse::{Command, Word};
use crate::paths::{basename, critical, inside, resolve};
use crate::remote::remote;

/// The command a wrapper runs: `sudo`, `env X=1`, `timeout 5`, `nice -n 5`, `nohup`, `time`,
/// `command`, `builtin`, `exec`, `stdbuf -oL` and `xargs` are looked through.
pub(crate) fn unwrapped(words: &[Word]) -> &[Word] {
    let mut rest = words;
    loop {
        let Some(name) = rest.first().and_then(Word::literal).map(basename) else {
            return rest;
        };
        let skip = match name {
            "sudo" | "doas" => flags(rest, &["-u", "-g", "-C", "-D"]),
            "env" => {
                let mut at = flags(rest, &["-u", "-C", "-S"]);
                while rest
                    .get(at)
                    .and_then(Word::literal)
                    .is_some_and(|word| word.contains('=') && !word.starts_with('-'))
                {
                    at = at.saturating_add(1);
                }
                at
            }
            "timeout" => flags(rest, &["-s", "-k", "--signal", "--kill-after"]).saturating_add(1),
            "nice" | "ionice" => flags(rest, &["-n", "-c"]),
            "stdbuf" | "xargs" => flags(rest, &["-I", "-n", "-P", "-L", "-d", "-E", "-s"]),
            "nohup" | "time" | "command" | "builtin" | "exec" => 1,
            _ => return rest,
        };
        rest = rest.get(skip..).unwrap_or_default();
    }
}

/// How many words a wrapper and its flags take: `takes_value` flags take the next word too.
fn flags(words: &[Word], takes_value: &[&str]) -> usize {
    let mut at = 1;
    while let Some(word) = words.get(at).and_then(Word::literal) {
        if !word.starts_with('-') || word == "-" {
            break;
        }
        at = at.saturating_add(if takes_value.contains(&word) { 2 } else { 1 });
        if word == "--" {
            break;
        }
    }
    at
}

/// Why `command` needs the person, if it does.
pub(crate) fn caution(command: &Command, context: &Context<'_>) -> Option<Caution> {
    let cwd = command.cwd.as_deref();
    let words = unwrapped(&command.words);
    let name = words.first().and_then(Word::literal).map(basename);
    let forge = name.and_then(|name| forge(name, words));
    // Before `sudo`'s own caution, which a judge may clear: these it never may.
    if let Some(found) = name.and_then(|name| prints_credential(name, words)) {
        return Some(found);
    }
    if let Some(found) = forge.as_ref().filter(|found| found.kind.firm()) {
        return Some(found.clone());
    }
    if let Some(found) = admin(&command.words) {
        return Some(found);
    }
    let found = match name {
        Some("rm") => removes(words, cwd, context),
        Some("git") => git(words),
        Some(name) => system(name, words, cwd, context)
            .or(forge)
            .or_else(|| remote(name, words)),
        None => None,
    };
    found
        .or_else(|| persists(command, words, cwd, context))
        .or_else(|| secrets(command, words, cwd, context))
}

fn found(kind: Kind, reason: impl Into<String>) -> Caution {
    Caution {
        kind,
        reason: reason.into(),
    }
}

/// `sudo` and friends run as the administrator, beyond anything the project needs.
fn admin(words: &[Word]) -> Option<Caution> {
    let name = words.first().and_then(Word::literal).map(basename)?;
    if matches!(name, "sudo" | "doas" | "su") {
        Some(found(Kind::System, "it runs as the administrator"))
    } else {
        None
    }
}

/// Where a path argument points, when it can be known.
fn target(word: &Word, cwd: Option<&Path>, context: &Context<'_>) -> Option<std::path::PathBuf> {
    let text = word.literal().or_else(|| {
        let text = word.text();
        (text.starts_with("$HOME") || text.starts_with("${HOME}")).then_some(text)
    })?;
    Some(resolve(cwd?, text, context.home))
}

/// `rm` of something critical, of anything outside the project, or of a folder it cannot name.
fn removes(words: &[Word], cwd: Option<&Path>, context: &Context<'_>) -> Option<Caution> {
    let mut recursive = false;
    let mut targets = Vec::new();
    let mut options = true;
    for word in words.iter().skip(1) {
        match word.literal() {
            Some("--") if options => options = false,
            Some(flag) if options && flag.starts_with("--") => {
                recursive |= flag == "--recursive";
            }
            Some(flag) if options && flag.starts_with('-') && flag.len() > 1 => {
                recursive |= flag.contains(['r', 'R']);
            }
            _ => targets.push(word),
        }
    }
    for word in targets {
        let path = match word {
            Word::Literal(_) => target(word, cwd, context),
            Word::Dynamic(text) => match glob_folder(text) {
                // A glob removes what its folder holds: judge the folder.
                Some(folder) => cwd.map(|cwd| resolve(cwd, folder, context.home)),
                None => target(word, cwd, context),
            },
        };
        let Some(path) = path else {
            if recursive {
                return Some(found(
                    Kind::Deletes,
                    format!(
                        "it deletes {}, which glave cannot tell in advance",
                        word.text()
                    ),
                ));
            }
            continue;
        };
        if critical(&path, context.project, context.home) {
            return Some(found(
                Kind::Deletes,
                format!("it deletes {}", path.display()),
            ));
        }
        if !inside(&path, context.project) && !context.temporary(&path) {
            return Some(found(
                Kind::Deletes,
                format!("it deletes {}, outside the project", path.display()),
            ));
        }
    }
    None
}

/// The folder a glob such as `build/*` or `*` matches in, when the glob has no variables.
fn glob_folder(text: &str) -> Option<&str> {
    if text.contains(['$', '`']) {
        return None;
    }
    let first = text.find(['*', '?', '['])?;
    let folder = text
        .get(..first)?
        .rsplit_once('/')
        .map_or(".", |(folder, _)| folder);
    Some(if folder.is_empty() { "/" } else { folder })
}

/// Git commands that throw away uncommitted work or rewrite what others share.
fn git(words: &[Word]) -> Option<Caution> {
    let mut at = 1;
    while let Some(word) = words.get(at).and_then(Word::literal) {
        if matches!(
            word,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            at = at.saturating_add(2);
        } else if word.starts_with('-') {
            at = at.saturating_add(1);
        } else {
            break;
        }
    }
    let subcommand = words.get(at).and_then(Word::literal)?;
    let args: Vec<&str> = words
        .iter()
        .skip(at.saturating_add(1))
        .map(Word::text)
        .collect();
    let has = |flag: &str| args.contains(&flag);
    let discards = |what: &str| Some(found(Kind::Discards, format!("it {what}")));
    match subcommand {
        "reset" if has("--hard") || has("--merge") || has("--keep") => {
            discards("discards uncommitted changes")
        }
        "clean"
            if !has("-n")
                && !has("--dry-run")
                && args.iter().any(|a| short(a, 'f') || *a == "--force") =>
        {
            discards("deletes untracked files")
        }
        // Reverting a file the person names is ordinary; wiping the whole tree is not.
        "checkout" if args.iter().any(|a| whole_tree(a)) => {
            discards("discards all uncommitted changes")
        }
        "restore"
            if (!(has("--staged") || has("-S")) || has("--worktree") || has("-W"))
                && args.iter().any(|a| whole_tree(a)) =>
        {
            discards("discards all uncommitted changes")
        }
        "stash" if matches!(args.first(), Some(&"drop" | &"clear")) => {
            discards("throws away stashed changes")
        }
        "branch" if has("-D") || (has("--delete") && has("--force")) => {
            discards("deletes a branch even if it isn't merged (git branch -D)")
        }
        "push"
            if args.iter().any(|a| {
                short(a, 'f')
                    || a.starts_with("--force")
                    || matches!(*a, "--mirror" | "--delete" | "-d")
                    || a.starts_with('+')
            }) =>
        {
            Some(found(
                Kind::Discards,
                "it rewrites or deletes what others share",
            ))
        }
        "push" => Some(found(Kind::Remote, "it pushes to a remote")),
        "config"
            if (has("--global") || has("--system") || has("core.hooksPath"))
                && config_writes(&args) =>
        {
            Some(found(
                Kind::Persists,
                "it changes git settings beyond this project",
            ))
        }
        _ => None,
    }
}

/// A pathspec that covers the whole work tree: `.`, `:/`, or a glob.
fn whole_tree(arg: &str) -> bool {
    matches!(arg, "." | "./" | ":/" | ":/." | "*") || arg.starts_with(":(top)")
}

/// Whether `git config` with these arguments writes a setting rather than reading one: a key
/// with a value, or an option that changes settings.
fn config_writes(args: &[&str]) -> bool {
    const WRITES: &[&str] = &[
        "--add",
        "--unset",
        "--unset-all",
        "--replace-all",
        "--edit",
        "-e",
        "--rename-section",
        "--remove-section",
    ];
    const READS: &[&str] = &["--get", "--get-all", "--get-regexp", "--list", "-l"];
    if args.iter().any(|a| WRITES.contains(a)) {
        return true;
    }
    if args.iter().any(|a| READS.contains(a)) {
        return false;
    }
    args.iter().filter(|a| !a.starts_with('-')).count() >= 2
}

/// A short flag group such as `-fd` holding `letter`.
fn short(arg: &str, letter: char) -> bool {
    arg.starts_with('-') && !arg.starts_with("--") && arg.contains(letter)
}

/// Commands that act on the computer, not the project.
fn system(
    name: &str,
    words: &[Word],
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let args: Vec<&str> = words.iter().skip(1).map(Word::text).collect();
    let reason = match name {
        "mkfs" | "fdisk" | "sfdisk" | "parted" | "wipefs" => {
            Some("it formats or repartitions a disk")
        }
        _ if name.starts_with("mkfs.") => Some("it formats a disk"),
        "diskutil"
            if args.iter().any(|a| {
                a.starts_with("erase") || matches!(*a, "partitionDisk" | "zeroDisk" | "secureErase")
            }) =>
        {
            Some("it erases a disk")
        }
        "dd" if args.iter().any(|a| a.starts_with("of=/dev/")) => {
            Some("it writes straight to a device")
        }
        "shutdown" | "reboot" | "halt" | "poweroff" => {
            Some("it shuts down or restarts the computer")
        }
        "chmod" | "chown" | "chgrp"
            if args.iter().any(|a| short(a, 'R') || *a == "--recursive") =>
        {
            let outside = words
                .iter()
                .skip(1)
                .filter(|w| w.literal().is_some_and(|w| !w.starts_with('-')))
                .any(|w| {
                    target(w, cwd, context).is_some_and(|path| {
                        critical(&path, context.project, context.home)
                            || !inside(&path, context.project)
                    })
                });
            outside.then_some("it changes permissions beyond the project")
        }
        _ => None,
    };
    reason.map(|reason| found(Kind::System, reason))
}

/// Writing where something runs later: shell start-up files, ssh, git hooks, launch agents,
/// scheduled jobs, system settings.
fn persists(
    command: &Command,
    words: &[Word],
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let name = words.first().and_then(Word::literal).map(basename);
    let args: Vec<&str> = words.iter().skip(1).map(Word::text).collect();
    match name {
        Some("crontab") if !args.contains(&"-l") => {
            return Some(found(Kind::Persists, "it changes scheduled jobs"));
        }
        Some("launchctl")
            if args
                .iter()
                .any(|a| matches!(*a, "load" | "bootstrap" | "enable" | "submit")) =>
        {
            return Some(found(
                Kind::Persists,
                "it installs something that runs at login",
            ));
        }
        Some("systemctl")
            if args
                .iter()
                .any(|a| matches!(*a, "enable" | "link" | "edit")) =>
        {
            return Some(found(
                Kind::Persists,
                "it installs something that runs at start-up",
            ));
        }
        _ => {}
    }
    let copies = matches!(name, Some("cp" | "mv" | "install" | "ln" | "rsync"));
    let tees = name == Some("tee");
    let mut written: Vec<&Word> = command.writes.iter().collect();
    if tees {
        written.extend(
            words
                .iter()
                .skip(1)
                .filter(|w| w.literal().is_some_and(|w| !w.starts_with('-'))),
        );
    }
    if copies && let Some(last) = words.last().filter(|_| words.len() > 2) {
        written.push(last);
    }
    written.into_iter().find_map(|word| {
        let path = target(word, cwd, context)?;
        startup_file(&path, context).then(|| Caution {
            kind: Kind::Persists,
            reason: format!("it writes {}, which runs later on its own", path.display()),
        })
    })
}

/// Files and folders whose contents run later without anyone asking.
fn startup_file(path: &Path, context: &Context<'_>) -> bool {
    const IN_HOME: &[&str] = &[
        ".bashrc",
        ".bash_profile",
        ".bash_login",
        ".profile",
        ".zshrc",
        ".zprofile",
        ".zshenv",
        ".zlogin",
        ".config/fish/config.fish",
        ".ssh",
        "Library/LaunchAgents",
        ".config/autostart",
        ".gitconfig",
    ];
    let home = context
        .home
        .is_some_and(|home| IN_HOME.iter().any(|file| path.starts_with(home.join(file))));
    let hooks =
        path.components().collect::<Vec<_>>().windows(2).any(
            |pair| matches!(pair, [a, b] if a.as_os_str() == ".git" && b.as_os_str() == "hooks"),
        );
    home || hooks
        || ["/etc", "/Library/LaunchAgents", "/Library/LaunchDaemons"]
            .iter()
            .any(|folder| path.starts_with(folder))
}

/// Reading a file that usually holds secrets, through any command but those that only look at
/// whether it exists.
fn secrets(
    command: &Command,
    words: &[Word],
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let name = words.first().and_then(Word::literal).map(basename);
    if matches!(
        name,
        Some("ls" | "stat" | "test" | "[" | "file" | "touch" | "rm")
    ) {
        return None;
    }
    words
        .iter()
        .skip(1)
        .chain(command.reads.iter())
        .filter(|word| word.literal().is_some_and(|w| !w.starts_with('-')))
        .find_map(|word| {
            let path = target(word, cwd, context)?;
            (context.secret)(&path).then(|| Caution {
                kind: Kind::Secrets,
                reason: format!("it reads {}, which usually holds secrets", word.text()),
            })
        })
}
