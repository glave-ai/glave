//! `cargo xtask credits`: no commit a change adds credits someone it shouldn't on GitHub.
//! GitHub counts any `<login>@users.noreply.github.com` address, as a commit's author, its
//! committer or a `Co-authored-by` trailer, as the real account with that login, so an address
//! made up for an agent (`noreply@`, `contributor@`) lists someone unrelated among the
//! contributors, and a squash merge copies each author of a pull request's commits into such
//! a trailer. Agents set no git identity of their own: they commit as the machine's git config
//! says, which is the owner's, and add no `Co-authored-by` line.
//!
//! The commits checked are those since the change's base (`crate::base`); history already on
//! main is left as it is. An address at GitHub's no-reply domain passes only when it is on
//! [`ALLOWED`], each with its reason, and the identity agents once committed as
//! ([`OLD_AGENT`]) is refused wherever it stands.

use crate::base::{self, Base};
use crate::cmd;

/// GitHub's no-reply domain, whose every address is a GitHub account's.
const NO_REPLY: &str = "users.noreply.github.com";

/// The name and address agents once committed as, before they set no identity of their own.
const OLD_AGENT: (&str, &str) = ("Glave contributor", "contributor@glave.invalid");

/// The no-reply addresses a commit may carry, and why.
const ALLOWED: &[(&str, &str)] = &[
    (
        "seashell86@users.noreply.github.com",
        "the repository's owner, whose squash merges GitHub authors with it",
    ),
    (
        "2745864+seashell86@users.noreply.github.com",
        "the repository's owner, whose git config agents commit with",
    ),
    (
        "49699333+dependabot[bot]@users.noreply.github.com",
        "Dependabot, which writes the dependency updates (.github/dependabot.yml)",
    ),
    (
        "41898282+github-actions[bot]@users.noreply.github.com",
        "the account a workflow commits as with the token GitHub gives it",
    ),
];

/// Where a commit names someone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Author,
    Committer,
    CoAuthor,
}

impl Role {
    fn said(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Committer => "committer",
            Self::CoAuthor => "Co-authored-by trailer",
        }
    }
}

/// Someone a commit names: where, their name and their address.
#[derive(Debug, PartialEq, Eq)]
struct Named<'a> {
    role: Role,
    name: &'a str,
    address: &'a str,
}

/// A commit, as `git log` prints it with [`FORMAT`].
#[derive(Debug, PartialEq, Eq)]
struct Commit<'a> {
    id: &'a str,
    subject: &'a str,
    /// Each one it names.
    named: Vec<Named<'a>>,
}

/// Each commit's id, author's and committer's names and addresses, and message, fields apart
/// by a unit separator and commits by a record separator, which none of them holds.
const FORMAT: &str = "--format=%H%x1f%an%x1f%ae%x1f%cn%x1f%ce%x1f%B%x1e";

/// Fails on a commit since the change's base that names a no-reply address not on
/// [`ALLOWED`], or [`OLD_AGENT`].
pub(crate) fn check() -> Result<(), String> {
    let Some(base) = base::find()? else {
        println!(
            "xtask: commits' addresses are not checked here, as there is no origin/main to \
             compare this branch with; CI checks them"
        );
        return Ok(());
    };
    let log = log_since(&base)?;
    let commits = parsed(&log)?;
    let refused: Vec<String> = commits.iter().flat_map(refusals).collect();
    if !refused.is_empty() {
        return Err(format!(
            "{} that GitHub must not credit:\n  {}\n\
             GitHub counts any address at {NO_REPLY} as the account with that login. Agents set \
             no git identity of their own and add no Co-authored-by line: they commit as this \
             machine's git config says. To mend the commits named, remove any such line \
             (git commit --amend), take this machine's identity with `git rebase -x 'git commit \
             --amend --reset-author --no-edit' {}`, and push the result to a new branch. If the \
             address is one GitHub should credit, add it to ALLOWED in xtask/src/credits.rs \
             with the reason.",
            counted(refused.len()),
            refused.join("\n  "),
            base.commit()
        ));
    }
    println!(
        "xtask: no commit since {} credits anyone it shouldn't on GitHub ({} checked)",
        base.said(),
        commits.len()
    );
    Ok(())
}

/// What `git log` prints of the commits since `base`. CI's checkout holds one commit, which
/// hides the ones before it, so a shallow clone fetches its history first.
fn log_since(base: &Base) -> Result<String, String> {
    let shallow = cmd::query("git", &["rev-parse", "--is-shallow-repository"])?;
    if shallow.trim() == "true" {
        let head = cmd::query("git", &["rev-parse", "HEAD"])?;
        let fetch = ["fetch", "--quiet", "--no-tags", "--unshallow", "origin"];
        cmd::query("git", &[fetch.as_slice(), &[head.trim()]].concat())
            .map_err(|err| format!("could not fetch this commit's history to check it: {err}"))?;
    }
    let range = format!("{}..HEAD", base.commit());
    cmd::query("git", &["log", FORMAT, &range])
}

/// The commits `log` holds, as [`FORMAT`] prints them.
fn parsed(log: &str) -> Result<Vec<Commit<'_>>, String> {
    log.split('\x1e')
        .map(str::trim_start)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.splitn(6, '\x1f').collect();
            let [
                id,
                author,
                author_address,
                committer,
                committer_address,
                message,
            ] = fields.as_slice()
            else {
                return Err(format!(
                    "git log printed a commit xtask can't read: {record}"
                ));
            };
            let mut named = vec![
                Named {
                    role: Role::Author,
                    name: author,
                    address: author_address,
                },
                Named {
                    role: Role::Committer,
                    name: committer,
                    address: committer_address,
                },
            ];
            named.extend(co_authors(message));
            Ok(Commit {
                id,
                subject: message.lines().next().unwrap_or_default(),
                named,
            })
        })
        .collect()
}

/// Each `Co-authored-by` line in `message`, wherever it stands: GitHub reads them from the
/// trailers, and a line that only looks like one is refused too rather than argued over.
fn co_authors(message: &str) -> impl Iterator<Item = Named<'_>> {
    message.lines().filter_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        if !key.trim().eq_ignore_ascii_case("co-authored-by") {
            return None;
        }
        let (name, address) = value.rsplit_once('<')?;
        let (address, _) = address.split_once('>')?;
        Some(Named {
            role: Role::CoAuthor,
            name: name.trim(),
            address: address.trim(),
        })
    })
}

/// The login a no-reply `address` belongs to, without the `<id>+` GitHub puts before it;
/// `None` for an address elsewhere.
fn login(address: &str) -> Option<&str> {
    let (local, domain) = address.rsplit_once('@')?;
    if !domain.eq_ignore_ascii_case(NO_REPLY) {
        return None;
    }
    Some(match local.split_once('+') {
        Some((id, login)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => login,
        _ => local,
    })
}

/// Why GitHub must not credit `named`, or `None` when it may.
fn refused(named: &Named<'_>) -> Option<String> {
    let (old_name, old_address) = OLD_AGENT;
    if named.name.eq_ignore_ascii_case(old_name) || named.address.eq_ignore_ascii_case(old_address)
    {
        Some("the identity agents once committed as".to_owned())
    } else if ALLOWED
        .iter()
        .any(|(allowed, _)| allowed.eq_ignore_ascii_case(named.address))
    {
        None
    } else {
        login(named.address).map(|login| format!("the GitHub account `{login}`"))
    }
}

/// Each one `commit` names whom GitHub must not credit, in a sentence.
fn refusals(commit: &Commit<'_>) -> Vec<String> {
    let short = commit.id.get(..10).unwrap_or(commit.id);
    commit
        .named
        .iter()
        .filter_map(|named| {
            let why = refused(named)?;
            Some(format!(
                "{short} \"{}\": its {} {} <{}> is {why}",
                commit.subject,
                named.role.said(),
                named.name,
                named.address
            ))
        })
        .collect()
}

fn counted(n: usize) -> String {
    match n {
        1 => "1 name".to_owned(),
        n => format!("{n} names"),
    }
}

#[cfg(test)]
mod tests;
