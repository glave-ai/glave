//! `glave worktrees`: the worktrees threads made on this machine (with `worktree = true` or
//! `--worktree`), and removing the ones the person is done with. Nothing is removed on its own.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glave_computer::worktrees::{self, Kept, Pruned};
use glave_event::{Body, Opened, ThreadId, decode};
use glave_ports::secrets::without_secrets;
use glave_session::protocol::{Reply, Request};

use crate::client::Glave;
use crate::home;

/// The folder in glave's folder where threads' worktrees are kept, one folder per repository.
pub const FOLDER: &str = "worktrees";

pub const USAGE: &str = "\
glave worktrees: the worktrees threads made on this machine

Usage:
  glave worktrees                    list them: folder, branch, repository, and whether each
                                     has uncommitted changes or is merged
  glave worktrees prune [name...]    remove the merged ones, or the ones named (by folder,
                                     folder name or branch). One with uncommitted changes is
                                     removed only after you confirm; a branch that is not
                                     merged is kept

A thread works in a worktree of its own with --worktree, or worktree = true in config.toml.
They are kept under $GLAVE_HOME/worktrees until you prune them.";

pub fn main(arguments: &[String]) -> ExitCode {
    let (prune, names) = match arguments.split_first() {
        None => (false, &[][..]),
        Some((first, rest)) if first == "prune" => (true, rest),
        Some((first, _)) if first == "--help" || first == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some((first, _)) => {
            eprintln!("glave: unknown worktrees command {first:?}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("glave: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    let ran = home::folder()
        .map(|home| home.join(FOLDER))
        .and_then(|base| {
            let environment = without_secrets(std::env::vars_os());
            let kept = runtime.block_on(worktrees::list(&base, &environment))?;
            if !prune {
                print!("{}", listing(&kept, &base));
                return Ok(());
            }
            let chosen = choose(kept, names);
            if chosen.is_empty() {
                println!("nothing to prune");
                return Ok(());
            }
            // Asked before anything runs, so no answer is awaited inside the runtime.
            let confirmed = confirm_changed(&chosen);
            let pruned = runtime.block_on(worktrees::prune(chosen, &environment, |kept| {
                confirmed.contains(&kept.path)
            }))?;
            print!("{}", report(&pruned));
            Ok(())
        });
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The worktrees to prune: the ones `names` name, or every merged one when none are named.
fn choose(kept: Vec<Kept>, names: &[String]) -> Vec<Kept> {
    if names.is_empty() {
        return kept.into_iter().filter(|kept| kept.merged).collect();
    }
    kept.into_iter()
        .filter(|kept| names.iter().any(|name| names_it(kept, name)))
        .collect()
}

/// Whether `name` is the worktree's folder, its folder's name, or its branch.
fn names_it(kept: &Kept, name: &str) -> bool {
    let path = Path::new(name);
    kept.path == path
        || std::fs::canonicalize(path).is_ok_and(|path| path == kept.path)
        || kept.path.file_name().is_some_and(|leaf| leaf == name)
        || kept.branch.as_deref() == Some(name)
}

/// The changed worktrees among `chosen` the person agrees to remove; none without a terminal.
fn confirm_changed(chosen: &[Kept]) -> HashSet<PathBuf> {
    let mut yes = HashSet::new();
    let asks = std::io::stdin().is_terminal();
    for kept in chosen.iter().filter(|kept| kept.changed) {
        if !asks {
            continue;
        }
        print!(
            "{} has uncommitted changes. Remove it and lose them? [y/N] ",
            kept.path.display()
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_ok()
            && matches!(line.trim(), "y" | "Y" | "yes")
        {
            yes.insert(kept.path.clone());
        }
    }
    yes
}

/// One line per worktree, with the base folder as `$GLAVE_HOME/worktrees`.
fn listing(kept: &[Kept], base: &Path) -> String {
    if kept.is_empty() {
        return format!("no worktrees in {}\n", base.display());
    }
    let mut out = String::new();
    for kept in kept {
        let mut notes = Vec::new();
        if kept.changed {
            notes.push("uncommitted changes");
        }
        if kept.merged {
            notes.push("merged");
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("  ({})", notes.join(", "))
        };
        let _ = writeln!(
            out,
            "{}  {}  of {}{notes}",
            kept.path.display(),
            kept.branch.as_deref().unwrap_or("(no branch)"),
            kept.repository.display()
        );
    }
    out
}

fn report(pruned: &[Pruned]) -> String {
    let mut out = String::new();
    for done in pruned {
        let _ = match done {
            Pruned::Removed(kept) => writeln!(out, "removed {}", kept.path.display()),
            Pruned::KeptChanged(kept) => writeln!(
                out,
                "kept {}: it has uncommitted changes",
                kept.path.display()
            ),
        };
    }
    out
}

/// Where `thread` works, when it is a worktree: for the person to see before the first turn.
pub async fn note(server: &Glave, thread: ThreadId) -> Option<String> {
    let page = Request::ThreadEvents {
        thread,
        after: None,
        limit: Some(16),
    };
    let Ok(Reply::Events { events, .. }) = server.request(page).await else {
        return None;
    };
    events
        .into_iter()
        .find_map(|raw| match decode(raw).ok()?.body {
            Body::WorkspaceBound(bound) => match bound.opened {
                Opened::Worktree { branch, .. } => {
                    Some(format!("working in {} on {branch}", bound.root))
                }
                Opened::Folder { .. } | Opened::Clone { .. } => None,
            },
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(path: &str, branch: &str, changed: bool, merged: bool) -> Kept {
        Kept {
            path: PathBuf::from(path),
            repository: PathBuf::from("/code/app"),
            branch: Some(branch.to_owned()),
            changed,
            merged,
        }
    }

    #[test]
    fn prune_takes_the_merged_ones_or_the_ones_named() {
        let all = vec![
            kept("/w/app-1/glave-a", "glave/a", false, true),
            kept("/w/app-1/glave-b", "glave/b", true, false),
        ];
        let merged = choose(all.clone(), &[]);
        assert_eq!(merged.len(), 1);
        let named = choose(all.clone(), &["glave/b".to_owned()]);
        assert_eq!(
            named.first().map(|kept| kept.branch.clone()),
            Some(Some("glave/b".into()))
        );
        let by_folder = choose(all, &["glave-a".to_owned()]);
        assert_eq!(by_folder.len(), 1);
    }

    #[test]
    fn the_listing_says_what_each_one_holds() {
        let shown = listing(
            &[kept("/w/app-1/glave-b", "glave/b", true, false)],
            Path::new("/w"),
        );
        assert_eq!(
            shown,
            "/w/app-1/glave-b  glave/b  of /code/app  (uncommitted changes)\n"
        );
        assert_eq!(listing(&[], Path::new("/w")), "no worktrees in /w\n");
    }
}
