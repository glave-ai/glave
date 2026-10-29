//! Code forges' command-line tools: `gh` (GitHub), `glab` (GitLab) and `tea` (Gitea). Opening,
//! commenting on, changing and merging pull requests and issues, and forks, gists, labels,
//! releases and workflow runs, are soft remote cautions: ordinary when the person asked for
//! exactly that. Changing a forge's sign-in, keys, CI secrets or variables, and writing through
//! its API, which reaches anything the token can, are firm. And what these tools print is what
//! others wrote there, so their output is untrusted.

use std::path::Path;

use glave_event::Risk;

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

fn soft(reason: Risk) -> Caution {
    Caution {
        kind: Kind::Remote,
        reason,
    }
}

fn firm(reason: Risk) -> Caution {
    Caution {
        kind: Kind::Credentials,
        reason,
    }
}

/// `gh` subcommands that change something on GitHub.
fn gh(group: &str, verb: &str, args: &[&str]) -> Option<Caution> {
    let found = match (group, verb) {
        ("api", _) if api_writes(args) => firm(Risk::WritesThroughApi {
            forge: "GitHub".to_owned(),
        }),
        ("auth", "login" | "logout" | "refresh" | "setup-git" | "switch") => {
            firm(Risk::ChangesSignIn {
                forge: "GitHub".to_owned(),
            })
        }
        ("secret", "set" | "delete" | "remove") => firm(Risk::ChangesRepositorySecrets),
        ("variable", "set" | "delete") => firm(Risk::ChangesRepositoryVariables),
        ("ssh-key" | "gpg-key", "add" | "delete") => firm(Risk::ChangesAccountKeys),
        ("pr", "create" | "new") => soft(Risk::OpensPullRequest),
        ("pr", "comment") => soft(Risk::CommentsOnPullRequest),
        ("pr", "merge") => soft(Risk::MergesPullRequest),
        ("pr", "edit" | "close" | "reopen" | "review" | "ready" | "lock" | "unlock") => {
            soft(Risk::ChangesPullRequest)
        }
        ("issue", "create" | "new") => soft(Risk::OpensIssue),
        ("issue", "comment") => soft(Risk::CommentsOnIssue),
        (
            "issue",
            "edit" | "close" | "reopen" | "delete" | "transfer" | "lock" | "unlock" | "pin"
            | "unpin",
        ) => soft(Risk::ChangesIssue),
        ("release", "create" | "new") => soft(Risk::PublishesRelease),
        ("release", "upload" | "edit" | "delete" | "delete-asset") => soft(Risk::ChangesRelease),
        ("repo", "create" | "new") => soft(Risk::CreatesRepository),
        ("repo", "fork") => soft(Risk::ForksRepository),
        ("repo", "delete" | "archive" | "unarchive" | "edit" | "rename") => {
            soft(Risk::ChangesRepository)
        }
        ("gist", "create" | "new") => soft(Risk::PublishesGist),
        ("gist", "edit" | "delete" | "rename") => soft(Risk::ChangesGist),
        ("label", "create" | "edit" | "delete" | "clone") => soft(Risk::ChangesRepositoryLabels),
        ("workflow", "run") => soft(Risk::RunsWorkflow),
        ("workflow", "enable" | "disable") | ("run", "rerun" | "cancel" | "delete") => {
            soft(Risk::ChangesWorkflows)
        }
        _ => return None,
    };
    Some(found)
}

/// `glab` subcommands that change something on GitLab.
fn glab(group: &str, verb: &str, args: &[&str]) -> Option<Caution> {
    let found = match (group, verb) {
        ("api", _) if api_writes(args) => firm(Risk::WritesThroughApi {
            forge: "GitLab".to_owned(),
        }),
        ("auth", "login" | "logout") => firm(Risk::ChangesSignIn {
            forge: "GitLab".to_owned(),
        }),
        ("variable" | "var", "set" | "update" | "delete") => firm(Risk::ChangesProjectVariables),
        ("mr", "create" | "new") => soft(Risk::OpensMergeRequest),
        ("mr", "note" | "comment") => soft(Risk::CommentsOnMergeRequest),
        ("mr", "merge" | "accept") => soft(Risk::MergesMergeRequest),
        ("mr", "update" | "close" | "reopen" | "approve" | "revoke" | "delete" | "rebase") => {
            soft(Risk::ChangesMergeRequest)
        }
        ("issue", "create" | "new") => soft(Risk::OpensIssue),
        ("issue", "note" | "comment") => soft(Risk::CommentsOnIssue),
        ("issue", "update" | "close" | "reopen" | "delete") => soft(Risk::ChangesIssue),
        ("release", "create" | "new") => soft(Risk::PublishesRelease),
        ("release", "upload" | "delete") => soft(Risk::ChangesRelease),
        ("repo", "create" | "new") => soft(Risk::CreatesRepository),
        ("repo", "fork") => soft(Risk::ForksRepository),
        ("repo", "delete" | "archive" | "transfer") => soft(Risk::ChangesRepository),
        ("snippet", "create" | "new") => soft(Risk::PublishesSnippet),
        ("label", "create" | "new" | "delete") => soft(Risk::ChangesProjectLabels),
        ("ci", "run" | "retry") => soft(Risk::RunsPipeline),
        _ => return None,
    };
    Some(found)
}

/// `tea` subcommands that change something on Gitea.
fn tea(group: &str, verb: &str) -> Option<Caution> {
    let found = match (group, verb) {
        ("login" | "logins", "add" | "delete" | "edit") => firm(Risk::ChangesSignIn {
            forge: "Gitea".to_owned(),
        }),
        ("pulls" | "pull" | "pr", "create") => soft(Risk::OpensPullRequest),
        ("pulls" | "pull" | "pr", "merge") => soft(Risk::MergesPullRequest),
        ("pulls" | "pull" | "pr", "close" | "reopen" | "approve" | "reject" | "review") => {
            soft(Risk::ChangesPullRequest)
        }
        ("issues" | "issue", "create") => soft(Risk::OpensIssue),
        ("issues" | "issue", "close" | "reopen" | "edit") => soft(Risk::ChangesIssue),
        ("comment", _) => soft(Risk::CommentsOnIssueOrPullRequest),
        ("releases" | "release", "create") => soft(Risk::PublishesRelease),
        ("releases" | "release", "delete" | "edit") => soft(Risk::ChangesRelease),
        ("repos" | "repo", "create") => soft(Risk::CreatesRepository),
        ("repos" | "repo", "fork") => soft(Risk::ForksRepository),
        ("repos" | "repo", "delete") => soft(Risk::ChangesRepository),
        ("labels" | "label", "create" | "update" | "delete") => soft(Risk::ChangesRepositoryLabels),
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
