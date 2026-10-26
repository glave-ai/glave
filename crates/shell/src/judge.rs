//! The verdict on one command: what glave's approvals act on.

use std::path::Path;

use crate::parse::parse;
use crate::reads::reads_only;
use crate::rules::caution;

/// Where a command runs, which decides what counts as outside it.
pub struct Context<'a> {
    /// The project's root, where commands run.
    pub project: &'a Path,
    /// The person's home folder, for `~` and `$HOME`.
    pub home: Option<&'a Path>,
    /// The temporary folder, where removing things is ordinary.
    pub temp: Option<&'a Path>,
    /// Whether a path names a file that usually holds secrets.
    pub secret: &'a dyn Fn(&Path) -> bool,
}

impl Context<'_> {
    /// Whether `path` is in a temporary folder.
    pub(crate) fn temporary(&self, path: &Path) -> bool {
        ["/tmp", "/private/tmp", "/var/folders"]
            .iter()
            .any(|folder| path.starts_with(folder))
            || self.temp.is_some_and(|temp| path.starts_with(temp))
    }
}

/// What a command is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It only reads.
    ReadOnly,
    /// It needs the person in any mode that asks, for this reason.
    Caution(Caution),
    /// Neither: the mode decides.
    Unknown,
}

/// Why a command needs the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caution {
    pub kind: Kind,
    /// In plain words, for the person and the model: "it deletes /home/ana".
    pub reason: String,
}

/// The kinds of harm a command may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Removes what cannot be got back: something critical, or outside the project.
    Deletes,
    /// Throws away uncommitted work, or rewrites what others share.
    Discards,
    /// Runs code fetched from the network.
    RunsFetchedCode,
    /// Acts on the computer: as the administrator, on disks, power.
    System,
    /// Plants something that runs later on its own.
    Persists,
    /// Would show the model a secret: reads a file that usually holds one, or prints one.
    Secrets,
    /// Acts beyond this computer: publishes, pushes, deploys, or changes shared systems.
    Remote,
    /// Changes credentials or what holds them, or acts with them where no one can see what it
    /// does: a forge's sign-in and keys, its CI secrets and variables, writes through its API.
    Credentials,
    /// Too tangled to read.
    Unreadable,
}

impl Kind {
    /// Whether a caution of this kind needs the person wherever one is asked, never a judge. The
    /// others may be cleared by a judge that finds the person's say-so in what they asked: secrets must never reach the
    /// model without the person, credentials never change without them, and an unreadable
    /// command cannot be judged.
    pub fn firm(self) -> bool {
        matches!(self, Self::Secrets | Self::Credentials | Self::Unreadable)
    }
}

/// The verdict on `script`, run in `context.project`.
pub fn judge(script: &str, context: &Context<'_>) -> Verdict {
    let parsed = parse(script, context.project);
    if parsed.too_deep {
        return Verdict::Caution(Caution {
            kind: Kind::Unreadable,
            reason: "it nests shells too deep to read".to_owned(),
        });
    }
    if parsed.runs_fetched_code {
        return Verdict::Caution(Caution {
            kind: Kind::RunsFetchedCode,
            reason: "it runs code fetched from the network".to_owned(),
        });
    }
    if let Some(found) = parsed
        .commands
        .iter()
        .find_map(|command| caution(command, context))
    {
        return Verdict::Caution(found);
    }
    if parsed.plain && !parsed.commands.is_empty() && parsed.commands.iter().all(reads_only) {
        Verdict::ReadOnly
    } else {
        Verdict::Unknown
    }
}
