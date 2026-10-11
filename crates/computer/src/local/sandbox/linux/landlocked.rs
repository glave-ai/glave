//! The fallback where bubblewrap cannot start: Landlock, applied by glave's helper to itself
//! before it becomes the command, at the `read-only` level only. Landlock grants access to
//! whole folders and cannot take it back below them, so it cannot keep protected paths
//! read-only inside a writable project; a `project` sandbox is refused instead.
//!
//! Reads are granted everywhere except secret locations by granting every neighbour on the
//! way down to them instead of their parent (folders stay listable). Writes go to the run's
//! temp folder and `/tmp`, never the project through them (a project inside `/tmp` then
//! costs new files directly in `/tmp`). With the network off, TCP is refused too where the
//! kernel can (ABI 4), on top of seccomp; signals and abstract Unix sockets stay inside the
//! sandbox where it can (ABI 6).

use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, PathFd,
    Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
};
use serde::{Deserialize, Serialize};

/// The newest Landlock this code knows; older kernels get what they support of it...
const NEWEST: ABI = ABI::V9;
/// ...but never less than this: before ABI 3, truncating a file was not restricted.
const OLDEST: ABI = ABI::V3;

/// Devices any command writes.
const DEVICES: &[&str] = &["/dev/null", "/dev/zero", "/dev/full", "/dev/tty"];

/// What the helper restricts itself to, worked out by glave before it starts the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub(super) network: bool,
    /// Writable, all of each...
    pub(super) writable: Vec<PathBuf>,
    /// ...except these...
    pub(super) unwritable: Vec<PathBuf>,
    /// ...except these again.
    pub(super) rewritable: Vec<PathBuf>,
    /// Readable everywhere except these...
    pub(super) unreadable: Vec<PathBuf>,
    /// ...except these again.
    pub(super) rereadable: Vec<PathBuf>,
}

/// Whether this kernel has Landlock, at least ABI 3.
pub(super) fn available() -> bool {
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(OLDEST))
        .and_then(Ruleset::create)
        .is_ok()
}

/// Restricts this thread, and what it runs, to `plan`.
pub(super) fn restrict(plan: &Plan) -> Result<(), String> {
    let failed = |error: &dyn std::fmt::Display| format!("could not apply Landlock: {error}");
    let rules = rules(plan).map_err(|error| failed(&error))?;
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(OLDEST))
        .and_then(|ruleset| {
            ruleset
                .set_compatibility(CompatLevel::BestEffort)
                .handle_access(AccessFs::from_all(NEWEST))
        })
        .and_then(|ruleset| ruleset.scope(Scope::from_all(NEWEST)))
        .map_err(|error| failed(&error))?;
    if !plan.network {
        ruleset = ruleset
            .handle_access(AccessNet::from_all(NEWEST))
            .map_err(|error| failed(&error))?;
    }
    let mut created = ruleset.create().map_err(|error| failed(&error))?;
    for (path, access) in rules {
        let Ok(fd) = PathFd::new(&path) else {
            // Gone since it was listed: nothing there to grant.
            continue;
        };
        created = created
            .add_rule(PathBeneath::new(fd, access))
            .map_err(|error| failed(&error))?;
    }
    let status = created.restrict_self().map_err(|error| failed(&error))?;
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err("Landlock is not enforced on this computer".into());
    }
    Ok(())
}

/// Each path to grant, and what.
fn rules(plan: &Plan) -> std::io::Result<Vec<(PathBuf, BitFlags<AccessFs>)>> {
    let read = AccessFs::from_read(NEWEST);
    let write = AccessFs::from_write(NEWEST);
    let mut rules = vec![(PathBuf::from("/"), BitFlags::from(AccessFs::ReadDir))];
    grant(
        Path::new("/"),
        read & !AccessFs::ReadDir,
        &plan.unreadable,
        &plan.rereadable,
        &mut rules,
    )?;
    for folder in &plan.writable {
        grant(
            folder,
            write,
            &plan.unwritable,
            &plan.rewritable,
            &mut rules,
        )?;
    }
    let device = AccessFs::WriteFile | AccessFs::Truncate | AccessFs::IoctlDev;
    for device_path in DEVICES {
        if Path::new(device_path).exists() {
            rules.push((PathBuf::from(device_path), device));
        }
    }
    Ok(rules
        .into_iter()
        .map(|(path, access)| {
            let file = std::fs::metadata(&path).is_ok_and(|meta| !meta.is_dir());
            let access = if file {
                access & AccessFs::from_file(NEWEST)
            } else {
                access
            };
            (path, access)
        })
        .filter(|(_, access)| !access.is_empty())
        .collect())
}

/// Grants `access` on `path`, except below the paths in `except`, except again below those
/// in `again`: where something under a folder is marked, its entries are granted one by one
/// instead of the folder. Links are skipped; what they lead to is granted, or not, alone.
fn grant(
    path: &Path,
    access: BitFlags<AccessFs>,
    except: &[PathBuf],
    again: &[PathBuf],
    rules: &mut Vec<(PathBuf, BitFlags<AccessFs>)>,
) -> std::io::Result<()> {
    let marked_below = except
        .iter()
        .chain(again)
        .any(|mark| mark != path && mark.starts_with(path));
    if !marked_below {
        if granted(path, except, again) {
            rules.push((path.to_owned(), access));
        }
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        // A folder glave cannot list, the command cannot read either.
        return Ok(());
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| !kind.is_symlink()) {
            grant(&entry.path(), access, except, again, rules)?;
        }
    }
    Ok(())
}

/// Whether `path` is granted: the deepest mark above it decides.
fn granted(path: &Path, except: &[PathBuf], again: &[PathBuf]) -> bool {
    let deepest = |marks: &[PathBuf]| {
        marks
            .iter()
            .filter(|mark| path.starts_with(mark))
            .map(|mark| mark.components().count())
            .max()
    };
    match (deepest(except), deepest(again)) {
        (None, _) => true,
        (Some(except), Some(again)) => again > except,
        (Some(_), None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deepest_mark_decides() {
        let except = [PathBuf::from("/h/.glave")];
        let again = [PathBuf::from("/h/.glave/worktrees/a")];
        assert!(granted(Path::new("/h/notes"), &except, &again));
        assert!(!granted(Path::new("/h/.glave/glave.db"), &except, &again));
        assert!(granted(
            Path::new("/h/.glave/worktrees/a/src"),
            &except,
            &again
        ));
        assert!(!granted(
            Path::new("/h/.glave/worktrees/b"),
            &except,
            &again
        ));
    }
}
