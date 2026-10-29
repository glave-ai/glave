//! Why a computer refuses, typed where glave's own words say it (`specs/session.md`,
//! "Languages"): a [`Refusal`], whose English the model and the log read as it always was, and
//! which the session writes again in the caller's language. A refusal in words a computer got
//! from elsewhere, or not yet typed, is its text alone; both are a [`Reason`].

use std::fmt;
use std::ops::Deref;

use glave_event::{Worded, worded};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

worded! {
    /// A refusal of glave's own, with the values it is written with.
    Refusal {
        // Folders and projects.
        NotAFolder { path } = "computer-not-a-folder" "{ $path } is not a folder",
        NamesTaken { path, tries } = "computer-names-taken"
            "{ $path } and the { $tries } names after it are taken; give the project another name",
        ProjectFolderName { name, most } = "computer-project-folder-name"
            "{ $name } is not a project folder's name: 1 to { $most } letters, digits, -, _ and ., not starting with . or -",
        ScratchName { name, most } = "computer-scratch-name"
            "{ $name } is not a scratch folder's name: 1 to { $most } letters, digits and -",
        TooManyComputers { most } = "computer-too-many-computers"
            "this machine already holds { $most } computers; destroy one first",
        TooManyWorkspaces { most } = "computer-too-many-workspaces"
            "{ $most } workspaces are already open; close one first",

        // Worktrees and git.
        NotInRepository { path } = "computer-not-in-repository"
            "{ $path } is not in a git repository, so it cannot have a worktree. Turn the worktree option off to work in the folder itself, or make it a repository with a commit.",
        NoCommitsYet { top } = "computer-no-commits-yet"
            "the repository at { $top } has no commits yet, and a worktree starts from one. Commit first, or turn the worktree option off.",
        NotABranchName { branch } = "computer-not-a-branch-name"
            "{ $branch } is not a valid branch name",
        NotOurWorktree { path, branch } = "computer-not-our-worktree"
            "{ $path } already exists and is not this repository's worktree on { $branch }; move it away, or remove it with `glave worktrees prune`",
        NewerRecord { path, version } = "computer-newer-record"
            "{ $path } was written by a newer version of glave (record version { $version }); update glave to use it",
        TooManyRepositories { most } = "computer-too-many-repositories"
            "worktrees are being made in { $most } repositories at once on this computer; try again when some are done",
        RepositoryNotValid { value } = "computer-repository-not-valid"
            "the repository { $value } is not valid",
        RevisionNotValid { value } = "computer-revision-not-valid"
            "the revision { $value } is not valid",
        BranchNotValid { value } = "computer-branch-not-valid" "the branch { $value } is not valid",
        NoCommitMessage = "computer-no-commit-message" "the commit message is empty",
        NotARepository = "computer-not-a-repository"
            "this folder is not a git repository; glave saves work only in repositories",
        CredentialLines = "computer-credential-lines"
            "a git credential needs a user name and a password, each one line",

        // Commands.
        EmptyCommand = "computer-empty-command" "the command is empty",
        TooManyHere { most } = "computer-too-many-here"
            "{ $most } commands are already running in the background here; stop one first",
        TooManyRunning { most } = "computer-too-many-running"
            "this computer already keeps { $most } commands running in the background; stop one first",
        NoRoom = "computer-no-room" "there is no room for it",

        // Workers.
        NeedsIsolation = "computer-needs-isolation"
            "policy requires workers isolated in a sandbox or virtual machine of their own ([workers] isolation = \"vm\"), and this backend has none: run workers on Kubernetes with [workers] runtime_class set (gVisor or Kata)",
        NoNetworkToClone = "computer-no-network-to-clone"
            "this worker has no network, so it cannot clone a repository; give it the network, or open a folder on it",
        HostNotImage = "computer-host-not-image"
            "a worker runs an image; this environment names the host. Name an image in devcontainer.json or .glave/environment.toml",
        NotPinned { image } = "computer-not-pinned"
            "workers run images pinned by digest, so the same bytes run every time: write the image as name@sha256:<digest>, not { $image }",
        TooManyWorkers { running, most } = "computer-too-many-workers"
            "you already have { $running } workers, and policy allows { $most } at once ([workers] per_person); release one first",
        TooManyCpus { asked, most } = "computer-too-many-cpus"
            "the environment asks for { $asked } CPUs, and policy allows at most { $most } ([workers] cpu)",
        TooMuchMemory { asked, most } = "computer-too-much-memory"
            "the environment asks for { $asked } GB of memory, and policy allows at most { $most } ([workers] memory_gb)",
        NoNetworkAllowed = "computer-no-network-allowed"
            "the environment asks for the network, and policy allows workers none ([workers] network = \"off\", or [sandbox] network = false)",
        SecretNamed { name } = "computer-secret-named"
            "{ $name } is named like a secret; a worker's variables are never secrets",
    }
}

/// Why a computer refused: its English as the model and the log read it, and the [`Refusal`]
/// it was made from when glave's own words said it. It crosses the wire as its text, or as the
/// refusal when it is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    english: String,
    refusal: Option<Refusal>,
}

impl Reason {
    /// The refusal it was made from, when glave's own words said it.
    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }
}

impl From<Refusal> for Reason {
    fn from(refusal: Refusal) -> Self {
        Self {
            english: refusal.english(),
            refusal: Some(refusal),
        }
    }
}

/// Words a computer got from elsewhere, or not yet typed, as they are.
impl From<String> for Reason {
    fn from(english: String) -> Self {
        Self {
            english,
            refusal: None,
        }
    }
}

impl From<&str> for Reason {
    fn from(english: &str) -> Self {
        Self::from(english.to_owned())
    }
}

impl Deref for Reason {
    type Target = str;

    fn deref(&self) -> &str {
        &self.english
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.english)
    }
}

/// How a reason crosses the wire: its text, or the refusal it was made from.
#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    Text(String),
    Refusal(Refusal),
}

impl Serialize for Reason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.refusal {
            Some(refusal) => Refusal::serialize(refusal, serializer),
            None => serializer.serialize_str(&self.english),
        }
    }
}

impl<'de> Deserialize<'de> for Reason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Text(text) => Self::from(text),
            Wire::Refusal(refusal) => Self::from(refusal),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_crosses_the_wire_typed_and_text_as_text() {
        let typed = Reason::from(Refusal::NotAFolder {
            path: "/srv/x".into(),
        });
        let json = serde_json::to_string(&typed).unwrap();
        assert_eq!(json, r#"{"says":"computer-not-a-folder","path":"/srv/x"}"#);
        let read: Reason = serde_json::from_str(&json).unwrap();
        assert_eq!(read, typed);
        assert_eq!(&*read, "/srv/x is not a folder");
        let text = Reason::from("the pattern could not be searched");
        let json = serde_json::to_string(&text).unwrap();
        assert_eq!(json, r#""the pattern could not be searched""#);
        assert_eq!(serde_json::from_str::<Reason>(&json).unwrap(), text);
    }

    /// Each refusal's English, as the computer wrote it before it was typed: what the model
    /// and the log read stays the same, byte for byte.
    #[test]
    fn each_refusal_reads_as_the_computer_wrote_it_before() {
        let before = [
            "<path> is not a folder",
            "<path> and the <tries> names after it are taken; give the project another name",
            "<name> is not a project folder's name: 1 to <most> letters, digits, -, _ and ., not starting with . or -",
            "<name> is not a scratch folder's name: 1 to <most> letters, digits and -",
            "this machine already holds <most> computers; destroy one first",
            "<most> workspaces are already open; close one first",
            "<path> is not in a git repository, so it cannot have a worktree. Turn the worktree option off to work in the folder itself, or make it a repository with a commit.",
            "the repository at <top> has no commits yet, and a worktree starts from one. Commit first, or turn the worktree option off.",
            "<branch> is not a valid branch name",
            "<path> already exists and is not this repository's worktree on <branch>; move it away, or remove it with `glave worktrees prune`",
            "<path> was written by a newer version of glave (record version <version>); update glave to use it",
            "worktrees are being made in <most> repositories at once on this computer; try again when some are done",
            "the repository <value> is not valid",
            "the revision <value> is not valid",
            "the branch <value> is not valid",
            "the commit message is empty",
            "this folder is not a git repository; glave saves work only in repositories",
            "a git credential needs a user name and a password, each one line",
            "the command is empty",
            "<most> commands are already running in the background here; stop one first",
            "this computer already keeps <most> commands running in the background; stop one first",
            "there is no room for it",
            "policy requires workers isolated in a sandbox or virtual machine of their own ([workers] isolation = \"vm\"), and this backend has none: run workers on Kubernetes with [workers] runtime_class set (gVisor or Kata)",
            "this worker has no network, so it cannot clone a repository; give it the network, or open a folder on it",
            "a worker runs an image; this environment names the host. Name an image in devcontainer.json or .glave/environment.toml",
            "workers run images pinned by digest, so the same bytes run every time: write the image as name@sha256:<digest>, not <image>",
            "you already have <running> workers, and policy allows <most> at once ([workers] per_person); release one first",
            "the environment asks for <asked> CPUs, and policy allows at most <most> ([workers] cpu)",
            "the environment asks for <asked> GB of memory, and policy allows at most <most> ([workers] memory_gb)",
            "the environment asks for the network, and policy allows workers none ([workers] network = \"off\", or [sandbox] network = false)",
            "<name> is named like a secret; a worker's variables are never secrets",
        ];
        let every = Refusal::every();
        assert_eq!(every.len(), before.len());
        for (refusal, before) in every.iter().zip(before) {
            assert_eq!(refusal.english(), before, "{}", refusal.id());
        }
    }
}
