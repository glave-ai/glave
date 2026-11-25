//! Each Node install's own `etc/npmrc` in the person's home folder, which can hold a registry's
//! token: hidden from someone else's commands, though the Node manager that holds it stays
//! readable. Looked for at the depths an install's sits at, failing closed: a manager whose
//! folder is too large to look through whole is not reopened at all.

use std::path::{Path, PathBuf};

/// Where Node's version managers keep what they install, each Node in it with its own
/// `etc/npmrc`.
pub(super) const NODE_MANAGERS: &[&str] = &[
    ".nvm",
    ".fnm",
    ".local/share/fnm",
    "Library/Application Support/fnm",
    ".volta",
    ".local/share/pnpm",
    "Library/pnpm",
    ".local/share/mise",
    ".asdf",
];

/// How deep below a Node manager's folder an install's `etc` is looked for: Volta's is at
/// `tools/image/node/<version>/etc`.
const DEPTH: usize = 6;
/// The most entries looked at below one Node manager's folder.
const MOST_ENTRIES: usize = 20_000;
/// Folders no install's own `etc` is in, however deep: packages and repositories.
const NEVER_IN: &[&str] = &["node_modules", ".git"];

/// What was found below the Node managers' folders in `home`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Found {
    /// Each install's own `etc/npmrc`.
    pub(super) npmrcs: Vec<PathBuf>,
    /// The managers' folders too large to look through whole, which stay hidden.
    pub(super) unsearched: Vec<PathBuf>,
}

/// Each Node install's own `etc/npmrc` under the Node managers' folders in `home`, as found
/// now, and the managers' folders too large to look through. Blocking.
pub(super) fn found(home: &Path) -> Found {
    let mut found = Found::default();
    for manager in NODE_MANAGERS.iter().map(|manager| home.join(manager)) {
        match npmrcs_in(&manager) {
            Some(npmrcs) => found.npmrcs.extend(npmrcs),
            None => found.unsearched.push(manager),
        }
    }
    found.npmrcs.sort();
    found
}

/// The `etc/npmrc` files below `manager`, or none where there are more than [`MOST_ENTRIES`]
/// to look at. Blocking.
fn npmrcs_in(manager: &Path) -> Option<Vec<PathBuf>> {
    let mut npmrcs = Vec::new();
    let mut seen = 0_usize;
    let mut folders = vec![(manager.to_path_buf(), 0_usize)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > MOST_ENTRIES {
                return None;
            }
            let path = entry.path();
            let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            let name = entry.file_name();
            if !is_folder || NEVER_IN.iter().any(|never| name == *never) {
                continue;
            }
            if name == "etc" {
                npmrcs.push(path.join("npmrc"));
            } else if depth < DEPTH {
                folders.push((path, depth.saturating_add(1)));
            }
        }
    }
    Some(npmrcs)
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::TempDir;

    use super::*;

    fn put(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn each_node_install_s_own_npmrc_is_found_there_yet_or_not() {
        let dir = TempDir::new("npmrcs").unwrap();
        let home = dir.path().to_owned();
        put(&home.join(".nvm/versions/node/v20/etc/npmrc"));
        put(&home.join(".volta/tools/image/node/20.0.0/etc/npmrc"));
        std::fs::create_dir_all(home.join(".local/share/pnpm/nodejs/22/etc")).unwrap();
        put(&home.join(".nvm/versions/node/v20/lib/node_modules/x/etc/npmrc"));
        let found = found(&home);
        assert_eq!(
            found.npmrcs,
            vec![
                home.join(".local/share/pnpm/nodejs/22/etc/npmrc"),
                home.join(".nvm/versions/node/v20/etc/npmrc"),
                home.join(".volta/tools/image/node/20.0.0/etc/npmrc"),
            ]
        );
        assert_eq!(found.unsearched, Vec::<PathBuf>::new());
    }

    #[test]
    fn a_manager_too_large_to_look_through_stays_hidden() {
        let dir = TempDir::new("npmrcs-large").unwrap();
        let home = dir.path().to_owned();
        let many = home.join(".nvm/versions");
        std::fs::create_dir_all(&many).unwrap();
        for index in 0..=MOST_ENTRIES {
            std::fs::create_dir(many.join(index.to_string())).unwrap();
        }
        let found = found(&home);
        assert_eq!(found.unsearched, vec![home.join(".nvm")]);
    }
}
