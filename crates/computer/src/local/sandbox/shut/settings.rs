//! Files in the person's home folder that someone else's commands meet through what stays
//! readable there: those git's own settings name, which git refuses to go on without once it
//! may not read them (it stops at a file it is refused, as it does not at one that is not
//! there); and each Node install's own `npmrc`, which can hold a registry's token.

use std::path::{Path, PathBuf};

use glave_ports::computer::git_settings;

/// The most settings files read, includes and all.
const MOST_FILES: usize = 64;
/// The most of one settings file read.
const MOST_BYTES: u64 = 1024 * 1024;

/// The settings git reads at every start that name another file: what they include, the
/// hooks, the commit message template, and the files of ignores and attributes.
const NAMING: &[(&str, &str)] = &[
    ("include", "path"),
    ("includeif", "path"),
    ("core", "hookspath"),
    ("core", "excludesfile"),
    ("core", "attributesfile"),
    ("commit", "template"),
];

/// Where Node's version managers keep what they install, each Node in it with its own
/// `etc/npmrc`.
const NODE_MANAGERS: &[&str] = &[
    ".nvm",
    ".fnm",
    ".local/share/fnm",
    "Library/Application Support/fnm",
    ".volta",
    ".local/share/mise",
    ".asdf",
];

/// How deep below a Node manager's folder an install's `etc/npmrc` is looked for: Volta's is
/// at `tools/image/node/<version>/etc/npmrc`.
const NPMRC_DEPTH: usize = 7;
/// The most entries looked at below the Node managers' folders.
const MOST_ENTRIES: usize = 20_000;

/// The files git's settings in `home` name, read as git reads them, includes followed: as
/// named, `~/` read as `home` and a relative include from the file that names it. Blocking.
pub(super) fn named_by_git(home: &Path) -> Vec<PathBuf> {
    let mut queue: Vec<PathBuf> = git_settings(home)
        .map(|path| {
            if path.ends_with(".config/git") {
                path.join("config")
            } else {
                path
            }
        })
        .collect();
    let mut read = Vec::new();
    let mut named = Vec::new();
    while let Some(file) = queue.pop() {
        if read.len() >= MOST_FILES || read.contains(&file) {
            continue;
        }
        read.push(file.clone());
        let Some(text) = text_of(&file) else {
            continue;
        };
        for (section, value) in naming(&text) {
            let Some(path) = expanded(&value, home, &file) else {
                continue;
            };
            if section.starts_with("include") {
                queue.push(path.clone());
            }
            named.push(path);
        }
    }
    named
}

/// The text of the settings file `file`, at most [`MOST_BYTES`] of it.
fn text_of(file: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(file)
        .ok()?
        .take(MOST_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// Each setting of [`NAMING`] in `text`, a git settings file, with its section and value.
fn naming(text: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut found = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let mut name = header.split(|c: char| c == ']' || c == '.' || c.is_whitespace());
            section = name.next().unwrap_or_default().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let wanted = NAMING
            .iter()
            .any(|(named, setting)| *named == section && *setting == key);
        if wanted && let Some(value) = plain(value) {
            found.push((section.clone(), value));
        }
    }
    found
}

/// A setting's value: quoted, what is between the quotes; else up to a comment.
fn plain(value: &str) -> Option<String> {
    let value = value.trim();
    let value = match value.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => value.split(['#', ';']).next()?.trim(),
    };
    (!value.is_empty()).then(|| value.to_owned())
}

/// `value`, a path in the settings file `file`: `~/` from `home`, a relative one from `file`'s
/// folder. None for one git fills in itself (`%(prefix)`).
fn expanded(value: &str, home: &Path, file: &Path) -> Option<PathBuf> {
    if value.starts_with("%(") {
        return None;
    }
    if value == "~" {
        return Some(home.to_owned());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    file.parent().map(|folder| folder.join(path))
}

/// Each Node install's own `etc/npmrc` under the Node managers' folders in `home`, as found
/// now. Blocking.
pub(super) fn npmrcs(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen = 0_usize;
    let mut folders: Vec<(PathBuf, usize)> = NODE_MANAGERS
        .iter()
        .map(|manager| (home.join(manager), 0))
        .collect();
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > MOST_ENTRIES {
                return found;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < NPMRC_DEPTH {
                folders.push((path, depth.saturating_add(1)));
            } else if path.ends_with("etc/npmrc") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::TempDir;

    use super::*;

    #[test]
    fn git_s_settings_name_their_includes_hooks_and_template_followed() {
        let dir = TempDir::new("git-named").unwrap();
        let home = dir.path().to_owned();
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = t\n[include]\n\tpath = ~/.gitconfig.local ; mine\n\
             [includeIf \"gitdir:~/work/\"]\n\tpath = \"work/.gitconfig\"\n\
             [core]\n\thooksPath = /opt/hooks\n\texcludesFile = ~/.ignore\n\
             [commit]\n\ttemplate = ~/.gitmessage\n[url \"x\"]\n\tpath = ~/not-this\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".gitconfig.local"),
            "[include]\n\tpath = deeper\n",
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".config/git")).unwrap();
        std::fs::write(
            home.join(".config/git/config"),
            "[include]\npath = %(prefix)/etc/x\n",
        )
        .unwrap();
        let mut named = named_by_git(&home);
        named.sort();
        let mut wanted = [
            ".gitconfig.local",
            "work/.gitconfig",
            ".ignore",
            ".gitmessage",
            "deeper",
        ]
        .map(|path| home.join(path))
        .to_vec();
        wanted.push(PathBuf::from("/opt/hooks"));
        wanted.sort();
        assert_eq!(named, wanted);
    }

    #[test]
    fn each_node_install_s_own_npmrc_is_found() {
        let dir = TempDir::new("npmrcs").unwrap();
        let home = dir.path().to_owned();
        for path in [
            ".nvm/versions/node/v20/etc/npmrc",
            ".volta/tools/image/node/20.0.0/etc/npmrc",
            ".nvm/versions/node/v20/lib/npmrc",
        ] {
            std::fs::create_dir_all(home.join(path).parent().unwrap()).unwrap();
            std::fs::write(home.join(path), "").unwrap();
        }
        assert_eq!(
            npmrcs(&home),
            vec![
                home.join(".nvm/versions/node/v20/etc/npmrc"),
                home.join(".volta/tools/image/node/20.0.0/etc/npmrc"),
            ]
        );
    }
}
