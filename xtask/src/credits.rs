//! `cargo xtask credits`: no commit a change adds credits a stranger on GitHub. GitHub counts
//! any `<login>@users.noreply.github.com` address, as a commit's author, its committer or a
//! `Co-authored-by` trailer, as the real account with that login, so an address made up for an
//! agent (`noreply@`, `contributor@`) lists someone unrelated among the contributors, and a
//! squash merge copies each author of a pull request's commits into such a trailer. Agents
//! commit as [`AGENT`], at a domain that can never be anyone's.
//!
//! The commits checked are those since the change's base (`crate::base`); history already on
//! main is left as it is. An address at GitHub's no-reply domain passes only when its login is
//! on [`ALLOWED`], each with its reason.

use crate::base::{self, Base};
use crate::cmd;

/// GitHub's no-reply domain, whose every address is a GitHub account's.
const NO_REPLY: &str = "users.noreply.github.com";

/// The name and address agents commit as.
const AGENT: &str = "Glave contributor <contributor@glave.invalid>";

/// The logins whose no-reply addresses a commit may carry, and why.
const ALLOWED: &[(&str, &str)] = &[
    (
        "seashell86",
        "the repository's owner, whose own address signs and merges their pull requests",
    ),
    (
        "dependabot[bot]",
        "Dependabot, which writes the dependency updates (.github/dependabot.yml)",
    ),
    (
        "github-actions[bot]",
        "the account a workflow commits as with the token GitHub gives it",
    ),
];

/// Where a commit names an address.
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

/// A commit, as `git log` prints it with [`FORMAT`].
#[derive(Debug, PartialEq, Eq)]
struct Commit<'a> {
    id: &'a str,
    subject: &'a str,
    /// Each address it names, and where.
    addresses: Vec<(Role, &'a str)>,
}

/// Each commit's id, author's and committer's addresses and message, fields apart by a unit
/// separator and commits by a record separator, which no address or message holds.
const FORMAT: &str = "--format=%H%x1f%ae%x1f%ce%x1f%B%x1e";

/// Fails on a commit since the change's base that names a no-reply address of a login not on
/// [`ALLOWED`].
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
            "{} GitHub would credit to someone else's account:\n  {}\n\
             GitHub counts any address at {NO_REPLY} as the account with that login. Commit as \
             `{AGENT}` instead (git -c user.name=\"Glave contributor\" -c \
             user.email=contributor@glave.invalid), with no Co-authored-by line: amend or \
             replay the commits named, then push them as new commits. If the address is your \
             own GitHub account's, add your login to ALLOWED in xtask/src/credits.rs with the \
             reason.",
            counted(refused.len()),
            refused.join("\n  ")
        ));
    }
    println!(
        "xtask: no commit since {} credits anyone else on GitHub ({} checked)",
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
        cmd::query(
            "git",
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--unshallow",
                "origin",
                head.trim(),
            ],
        )
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
            let mut fields = record.splitn(4, '\x1f');
            let (Some(id), Some(author), Some(committer), Some(message)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(format!(
                    "git log printed a commit xtask can't read: {record}"
                ));
            };
            let mut addresses = vec![(Role::Author, author), (Role::Committer, committer)];
            addresses.extend(co_authors(message).map(|address| (Role::CoAuthor, address)));
            Ok(Commit {
                id,
                subject: message.lines().next().unwrap_or_default(),
                addresses,
            })
        })
        .collect()
}

/// The address of each `Co-authored-by` line in `message`, wherever it stands: GitHub reads
/// them from the trailers, and a line that only looks like one is refused too rather than
/// argued over.
fn co_authors(message: &str) -> impl Iterator<Item = &str> {
    message.lines().filter_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        if !key.trim().eq_ignore_ascii_case("co-authored-by") {
            return None;
        }
        let (_, address) = value.rsplit_once('<')?;
        address.split_once('>').map(|(address, _)| address.trim())
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

/// Each address in `commit` GitHub would credit to a login not on [`ALLOWED`], in a sentence.
fn refusals(commit: &Commit<'_>) -> Vec<String> {
    let short = commit.id.get(..10).unwrap_or(commit.id);
    commit
        .addresses
        .iter()
        .filter_map(|&(role, address)| {
            let login = login(address)?;
            let allowed = ALLOWED
                .iter()
                .any(|(allowed, _)| allowed.eq_ignore_ascii_case(login));
            (!allowed).then(|| {
                format!(
                    "{short} \"{}\": its {} {address} is the GitHub account `{login}`",
                    commit.subject,
                    role.said()
                )
            })
        })
        .collect()
}

fn counted(n: usize) -> String {
    match n {
        1 => "1 address".to_owned(),
        n => format!("{n} addresses"),
    }
}

#[cfg(test)]
mod tests;
