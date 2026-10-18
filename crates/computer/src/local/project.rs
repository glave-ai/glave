//! A new project's folder: made empty in the folder for projects, named after the project,
//! and never one that was there, so making a project never takes over a folder of the
//! person's.

use std::path::{Path, PathBuf};

use glave_ports::computer::ComputerError;

use super::paths;

/// The longest name a project's folder is given.
const MAX_NAME: usize = 64;

/// The names tried for one project: `name`, then `name-2` up to this.
const TRIES: u32 = 100;

/// A new, empty folder for the project `name` under `base`, made with any folder above it
/// that is missing: `name`, or the first of `name-2`, `name-3` and on that is free.
pub(crate) async fn make(base: &Path, name: &str) -> Result<PathBuf, ComputerError> {
    check(name)?;
    tokio::fs::create_dir_all(base).await.map_err(|error| {
        ComputerError::Failed(format!("could not make {}: {error}", base.display()))
    })?;
    for n in 1..=TRIES {
        let leaf = if n == 1 {
            name.to_owned()
        } else {
            format!("{name}-{n}")
        };
        let dest = base.join(leaf);
        // Made, never reused: a folder that is there already, or a link, stays the person's.
        match tokio::fs::create_dir(&dest).await {
            Ok(()) => return paths::folder(&dest.to_string_lossy()).await,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ComputerError::Failed(format!(
                    "could not make {}: {error}",
                    dest.display()
                )));
            }
        }
    }
    Err(ComputerError::Refused(format!(
        "{} and the {TRIES} names after it are taken; give the project another name",
        base.join(name).display()
    )))
}

/// Refuses a name that is not one folder's plain name.
fn check(name: &str) -> Result<(), ComputerError> {
    let plain = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty()
        || name.chars().count() > MAX_NAME
        || name.starts_with(['.', '-'])
        || !name.chars().all(plain)
    {
        return Err(ComputerError::Refused(format!(
            "{name:?} is not a project folder's name: 1 to {MAX_NAME} letters, digits, -, _ and \
             ., not starting with . or -"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_name_names_a_project_folder() {
        for good in ["app", "my-app", "Café_2.0", "x"] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            ".git",
            "-rf",
            "a/b",
            "..",
            "a b",
            "a\nb",
            &"x".repeat(65),
        ] {
            assert!(check(bad).is_err(), "{bad:?}");
        }
    }
}
