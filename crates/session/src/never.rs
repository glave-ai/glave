//! The folders that are never a project (`specs/one-server.md`, "A folder's project"): the home
//! folder, a folder above it, and the temporary folder. What the person does there is a chat in
//! their Chats. One rule, which the CLI asks of this computer's folders and the server of the
//! folders on its computer, each with the folders as that computer resolves them.

use std::path::Path;

/// Why a folder is never a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeverAProject {
    /// The home folder, which holds the person's Chats.
    Home,
    /// A folder the home folder is in.
    AboveHome,
    /// The temporary folder.
    Temporary,
}

/// Why `folder` is never a project, if it is not one: given the `home` folder, if there is one,
/// and the `temporary` one, all three with links followed.
pub fn never_a_project(
    folder: &Path,
    home: Option<&Path>,
    temporary: &Path,
) -> Option<NeverAProject> {
    match home {
        Some(home) if home == folder => Some(NeverAProject::Home),
        Some(home) if home.starts_with(folder) => Some(NeverAProject::AboveHome),
        _ if folder == temporary => Some(NeverAProject::Temporary),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_folder_those_above_it_and_the_temporary_one_are_never_projects() {
        let home = Some(Path::new("/home/ada"));
        let temporary = Path::new("/tmp");
        let never = |folder: &str| never_a_project(Path::new(folder), home, temporary);
        assert_eq!(never("/home/ada"), Some(NeverAProject::Home));
        assert_eq!(never("/home/ada/"), Some(NeverAProject::Home));
        assert_eq!(never("/home"), Some(NeverAProject::AboveHome));
        assert_eq!(never("/"), Some(NeverAProject::AboveHome));
        assert_eq!(never("/tmp"), Some(NeverAProject::Temporary));
        for folder in ["/home/ada/code/app", "/home/adam", "/tmp/app", "/srv"] {
            assert_eq!(never(folder), None, "{folder}");
        }
        assert_eq!(
            never_a_project(Path::new("/home/ada"), None, temporary),
            None,
            "no home folder, none above it"
        );
    }
}
