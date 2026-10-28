//! Removing a folder and everything in it, including what was made read-only: Go's module
//! cache, for one, makes its folders unwritable, so a plain recursive removal fails there.

use std::io::ErrorKind;
use std::path::Path;

/// Removes `path` and everything in it, letting its owner write every folder inside when a
/// plain removal fails (links are never followed). A path that is already gone is not an
/// error. Blocking.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    // Tried first: it works below `path` through open folders, so no path inside is too long
    // for the system, as the walk that makes folders writable, by whole paths, can be (#922).
    if gone(std::fs::remove_dir_all(path)).is_ok() {
        return Ok(());
    }
    writable(path)?;
    gone(std::fs::remove_dir_all(path))
}

/// `removed`, with a path that was gone already taken as removed.
fn gone(removed: std::io::Result<()>) -> std::io::Result<()> {
    match removed {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Lets the owner enter and write `path` and every folder under it.
fn writable(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = metadata.permissions().mode();
        if mode & 0o700 != 0o700 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o700))?;
        }
    }
    for entry in std::fs::read_dir(path)? {
        writable(&entry?.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::remove_tree;

    #[test]
    fn removes_read_only_folders_and_what_is_gone_already() {
        let dir = std::env::temp_dir().join(format!("glave-remove-{}", std::process::id()));
        let locked = dir.join("a/b");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("file"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let read_only = std::fs::Permissions::from_mode(0o500);
            std::fs::set_permissions(&locked, read_only.clone()).unwrap();
            std::fs::set_permissions(dir.join("a"), read_only).unwrap();
        }
        remove_tree(&dir).unwrap();
        assert!(!dir.exists());
        remove_tree(&dir).unwrap();
    }

    #[test]
    fn removes_a_tree_whose_paths_are_longer_than_the_system_takes() {
        let dir = std::env::temp_dir().join(format!("glave-remove-long-{}", std::process::id()));
        let long = |n: char| n.to_string().repeat(250);
        // Each path made is short; the folders moved under one another nest past 4,096 bytes,
        // longer than macOS (1,024) or Linux (4,096) takes in one path.
        std::fs::create_dir_all(&dir).unwrap();
        let mut deepest = dir.clone();
        for n in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
            let made = dir.join(format!("{n}.made"));
            let chain = made.join(long(n)).join(long(n));
            std::fs::create_dir_all(&chain).unwrap();
            std::fs::write(chain.join("file"), "x").unwrap();
            if deepest != dir {
                std::fs::rename(&deepest, chain.join("under")).unwrap();
            }
            deepest = made;
        }
        remove_tree(&dir).unwrap();
        assert!(!dir.exists());
    }
}
