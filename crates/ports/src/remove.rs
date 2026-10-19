//! Removing a folder and everything in it, including what was made read-only: Go's module
//! cache, for one, makes its folders unwritable, so a plain recursive removal fails there.

use std::io::ErrorKind;
use std::path::Path;

/// Removes `path` and everything in it, first letting its owner write every folder inside
/// (links are never followed). A path that is already gone is not an error. Blocking.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    writable(path)?;
    match std::fs::remove_dir_all(path) {
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
}
