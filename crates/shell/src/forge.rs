//! Code forges' command-line tools: `gh` (GitHub), `glab` (GitLab) and `tea` (Gitea). Opening,
//! commenting on, changing and merging pull requests and issues, and forks, gists, labels,
//! releases and workflow runs, are soft remote cautions: ordinary when the person asked for
//! exactly that. Changing a forge's sign-in, keys, CI secrets or variables, and writing through
//! its API, which reaches anything the token can, are firm. And what these tools print is what
//! others wrote there, so their output is untrusted.

use std::path::Path;

use crate::judge::{Caution, Kind};
use crate::parse::{Word, parse};
use crate::paths::basename;
use crate::rules::unwrapped;

/// Whether what `script`, run in `project`, prints may hold what others wrote on a forge: it
/// runs `gh`, `glab` or `tea` somewhere, or nests too deep to tell.
pub fn untrusted_output(script: &str, project: &Path) -> bool {
    let parsed = parse(script, project);
    parsed.too_deep
        || parsed.commands.iter().any(|command| {
            unwrapped(&command.words)
                .first()
                .and_then(Word::literal)
                .map(basename)
                .is_some_and(|name| matches!(name, "gh" | "glab" | "tea"))
        })
}

/// Why `words` (a forge's command after any wrapper, named `name`) acts on the forge, if it does.
pub(crate) fn forge(name: &str, words: &[Word]) -> Option<Caution> {
    let args: Vec<&str> = words.iter().skip(1).map(Word::text).collect();
    let plain: Vec<&str> = args
        .iter()
        .copied()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let group = plain.first().copied()?;
    let verb = plain.get(1).copied().unwrap_or_default();
    match name {
        "gh" => gh(group, verb, &args),
        "glab" => glab(group, verb, &args),
        "tea" => tea(group, verb),
        _ => None,
    }
}

fn soft(reason: &str) -> Caution {
    Caution {
        kind: Kind::Remote,
        reason: reason.to_owned(),
    }
}

fn firm(reason: &str) -> Caution {
    Caution {
        kind: Kind::Credentials,
        reason: reason.to_owned(),
    }
}

/// `gh` subcommands that change something on GitHub.
fn gh(group: &str, verb: &str, args: &[&str]) -> Option<Caution> {
    let found = match (group, verb) {
        ("api", _) if api_writes(args) => {
            firm("it writes through GitHub's API, which reaches anything the token can")
        }
        ("auth", "login" | "logout" | "refresh" | "setup-git" | "switch") => {
            firm("it changes how this computer signs in to GitHub")
        }
        ("secret", "set" | "delete" | "remove") => firm("it changes a repository's secrets"),
        ("variable", "set" | "delete") => firm("it changes a repository's variables"),
        ("ssh-key" | "gpg-key", "add" | "delete") => firm("it changes a GitHub account's keys"),
        ("pr", "create" | "new") => soft("it opens a pull request"),
        ("pr", "comment") => soft("it comments on a pull request"),
        ("pr", "merge") => soft("it merges a pull request"),
        ("pr", "edit" | "close" | "reopen" | "review" | "ready" | "lock" | "unlock") => {
            soft("it changes a pull request")
        }
        ("issue", "create" | "new") => soft("it opens an issue"),
        ("issue", "comment") => soft("it comments on an issue"),
        (
            "issue",
            "edit" | "close" | "reopen" | "delete" | "transfer" | "lock" | "unlock" | "pin"
            | "unpin",
        ) => soft("it changes an issue"),
        ("release", "create" | "new") => soft("it publishes a release"),
        ("release", "upload" | "edit" | "delete" | "delete-asset") => soft("it changes a release"),
        ("repo", "create" | "new") => soft("it creates a repository"),
        ("repo", "fork") => soft("it forks a repository"),
        ("repo", "delete" | "archive" | "unarchive" | "edit" | "rename") => {
            soft("it changes a repository")
        }
        ("gist", "create" | "new") => soft("it publishes a gist"),
        ("gist", "edit" | "delete" | "rename") => soft("it changes a gist"),
        ("label", "create" | "edit" | "delete" | "clone") => {
            soft("it changes a repository's labels")
        }
        ("workflow", "run") => soft("it runs a workflow"),
        ("workflow", "enable" | "disable") | ("run", "rerun" | "cancel" | "delete") => {
            soft("it changes a repository's workflows")
        }
        _ => return None,
    };
    Some(found)
}

/// `glab` subcommands that change something on GitLab.
fn glab(group: &str, verb: &str, args: &[&str]) -> Option<Caution> {
    let found = match (group, verb) {
        ("api", _) if api_writes(args) => {
            firm("it writes through GitLab's API, which reaches anything the token can")
        }
        ("auth", "login" | "logout") => firm("it changes how this computer signs in to GitLab"),
        ("variable" | "var", "set" | "update" | "delete") => {
            firm("it changes a project's variables")
        }
        ("mr", "create" | "new") => soft("it opens a merge request"),
        ("mr", "note" | "comment") => soft("it comments on a merge request"),
        ("mr", "merge" | "accept") => soft("it merges a merge request"),
        ("mr", "update" | "close" | "reopen" | "approve" | "revoke" | "delete" | "rebase") => {
            soft("it changes a merge request")
        }
        ("issue", "create" | "new") => soft("it opens an issue"),
        ("issue", "note" | "comment") => soft("it comments on an issue"),
        ("issue", "update" | "close" | "reopen" | "delete") => soft("it changes an issue"),
        ("release", "create" | "new") => soft("it publishes a release"),
        ("release", "upload" | "delete") => soft("it changes a release"),
        ("repo", "create" | "new") => soft("it creates a repository"),
        ("repo", "fork") => soft("it forks a repository"),
        ("repo", "delete" | "archive" | "transfer") => soft("it changes a repository"),
        ("snippet", "create" | "new") => soft("it publishes a snippet"),
        ("label", "create" | "new" | "delete") => soft("it changes a project's labels"),
        ("ci", "run" | "retry") => soft("it runs a pipeline"),
        _ => return None,
    };
    Some(found)
}

/// `tea` subcommands that change something on Gitea.
fn tea(group: &str, verb: &str) -> Option<Caution> {
    let found = match (group, verb) {
        ("login" | "logins", "add" | "delete" | "edit") => {
            firm("it changes how this computer signs in to Gitea")
        }
        ("pulls" | "pull" | "pr", "create") => soft("it opens a pull request"),
        ("pulls" | "pull" | "pr", "merge") => soft("it merges a pull request"),
        ("pulls" | "pull" | "pr", "close" | "reopen" | "approve" | "reject" | "review") => {
            soft("it changes a pull request")
        }
        ("issues" | "issue", "create") => soft("it opens an issue"),
        ("issues" | "issue", "close" | "reopen" | "edit") => soft("it changes an issue"),
        ("comment", _) => soft("it comments on an issue or a pull request"),
        ("releases" | "release", "create") => soft("it publishes a release"),
        ("releases" | "release", "delete" | "edit") => soft("it changes a release"),
        ("repos" | "repo", "create") => soft("it creates a repository"),
        ("repos" | "repo", "fork") => soft("it forks a repository"),
        ("repos" | "repo", "delete") => soft("it changes a repository"),
        ("labels" | "label", "create" | "update" | "delete") => {
            soft("it changes a repository's labels")
        }
        _ => return None,
    };
    Some(found)
}

/// Whether a `gh api` or `glab api` call writes: a method other than GET or HEAD, or, with
/// none, fields or a body, which make the method POST. A GraphQL call always posts; it writes
/// when its query is a mutation, or comes from a file or standard input glave cannot read.
fn api_writes(args: &[&str]) -> bool {
    let mut method = None;
    let mut body = false;
    let mut mutates = false;
    let mut words = args.iter().copied();
    while let Some(arg) = words.next() {
        let value = match arg {
            "-X" | "--method" => {
                method = words.next();
                continue;
            }
            "-f" | "-F" | "--field" | "--raw-field" | "--input" => words.next().unwrap_or_default(),
            _ => {
                if let Some(given) = arg.strip_prefix("--method=") {
                    method = Some(given);
                } else if let Some(given) = arg.strip_prefix("-X") {
                    method = Some(given);
                }
                let fields = ["--field=", "--raw-field=", "--input=", "-f", "-F"];
                if !fields.iter().any(|flag| arg.starts_with(flag)) {
                    continue;
                }
                arg
            }
        };
        body = true;
        mutates |= arg.starts_with("--input") || value.contains('@') || value.contains("mutation");
    }
    if args.contains(&"graphql") {
        return mutates;
    }
    match method {
        Some(method) => !["GET", "HEAD"]
            .iter()
            .any(|m| m.eq_ignore_ascii_case(method)),
        None => body,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::untrusted_output;

    #[test]
    fn forge_commands_make_output_untrusted_wherever_they_sit() {
        let project = Path::new("/work/app");
        for script in [
            "gh issue view 12",
            "gh pr diff 7 | head -50",
            "gh api repos/acme/app/issues/12/comments",
            "glab mr view 3",
            "tea issues 4",
            "cd src && /usr/local/bin/gh pr view",
            "bash -c 'gh repo view acme/app'",
            "GH_PAGER=cat gh run view 99 --log",
        ] {
            assert!(untrusted_output(script, project), "{script}");
        }
        for script in ["cargo test", "echo gh issue view 12", "git log", ""] {
            assert!(!untrusted_output(script, project), "{script}");
        }
    }
}
