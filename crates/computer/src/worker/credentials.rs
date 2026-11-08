//! Where a worker's git credential comes from: the person's own git on the harness side
//! (`git credential fill`, so whatever helper they already use answers), a fixed one, or none.
//! It is asked for when a worker opens a repository, and travels to the worker only inside
//! `workspace/open`, over TLS. And repository URLs as a worker can fetch them: over HTTPS,
//! since SSH keys never leave the person's machine.

use std::ffi::OsStr;
use std::process::Stdio;
use std::time::Duration;

use glave_ports::computer::{ComputerError, GitCredential};
use glave_process::Processes;

/// How long the person's credential helper has to answer.
const FILL_TIMEOUT: Duration = Duration::from_secs(20);

/// All of the server's environment that git and a person's credential helper get, by name
/// ignoring case: what git, the keychains and the helpers people use read. Everything else stays
/// behind, the model's key and the store's address among it, so nothing there has to be named
/// as a secret. `GIT_ASKPASS` is left out too: nothing is asked interactively.
const HELPERS_READ: &[&str] = &[
    // Finding programs, and the person's own files (`~/.gitconfig`, a keychain, a sign-in).
    "HOME",
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "GIT_EXEC_PATH",
    // A keyring on Linux, an ssh agent, gpg and the store `pass` keeps with it.
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "GNUPGHOME",
    "GPG_TTY",
    "PASSWORD_STORE_DIR",
    // `gh auth git-credential` and `glab auth git-credential`, signed in by these or by a file.
    "GH_CONFIG_DIR",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GLAB_CONFIG_DIR",
    "GITLAB_TOKEN",
    // How a helper that renews a sign-in reaches its host.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// The same for families of names: git's own settings (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_KEY_0`)
/// and Git Credential Manager's.
const HELPERS_READ_ALL_OF: &[&str] = &["GIT_CONFIG_", "GCM_"];

/// Where workers' git credentials come from.
#[derive(Debug, Clone, Default)]
pub enum WorkerCredentials {
    /// None: only public repositories can be cloned, and nothing can be pushed.
    #[default]
    None,
    /// The same credential for every repository.
    Fixed(GitCredential),
    /// The person's own git, asked for each repository with `git credential fill`, run as one
    /// of these processes. Nothing is asked interactively: no answer means no credential.
    HostGit(Processes),
}

impl WorkerCredentials {
    /// The credential for the repository at `url`, if there is one. Only HTTPS URLs get one.
    pub async fn for_url(&self, url: &str) -> Result<Option<GitCredential>, ComputerError> {
        let Some(rest) = url.strip_prefix("https://") else {
            return Ok(None);
        };
        match self {
            Self::None => Ok(None),
            Self::Fixed(credential) => Ok(Some(credential.clone())),
            Self::HostGit(processes) => fill(processes, rest).await,
        }
    }
}

/// Asks the person's git for the credential for `https://<rest>`, from no repository: the server
/// may have started in one, whose config a sandboxed command could have written, and git would
/// run a helper it names. What answers is what the person set for their whole account.
async fn fill(processes: &Processes, rest: &str) -> Result<Option<GitCredential>, ComputerError> {
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let asked = format!("protocol=https\nhost={host}\npath={path}\n\n");
    let mut git = glave_process::Program::new("git");
    git.args(["credential", "fill"])
        .current_dir("/")
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| helpers_read(name)))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = match processes
        .output_with(&mut git, asked.as_bytes(), FILL_TIMEOUT)
        .await
    {
        Ok(output) => output,
        // A helper that does not answer in time gives no credential.
        Err(glave_process::ProcessError::TimedOut { .. }) => return Ok(None),
        Err(error) => return Err(ComputerError::Failed(error.to_string())),
    };
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

/// Whether git and a credential helper get the server's variable `name`.
fn helpers_read(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    HELPERS_READ.contains(&name.as_str())
        || HELPERS_READ_ALL_OF
            .iter()
            .any(|family| name.starts_with(family))
}

/// The user name and password in git's answer.
fn parse(answer: &str) -> Option<GitCredential> {
    let field = |key: &str| {
        answer
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_owned)
    };
    GitCredential::new(field("username")?, field("password")?).ok()
}

/// `url` as a worker fetches it: SSH forms (`git@host:owner/repo.git`,
/// `ssh://git@host/owner/repo.git`) become HTTPS; anything else is kept.
pub fn https_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map_or(rest, |(_, after)| after);
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.split_once(':').map_or(host, |(host, _)| host);
        return format!("https://{host}/{path}");
    }
    if !url.contains("://")
        && let Some((user_host, path)) = url.split_once(':')
        && let Some((_, host)) = user_host.split_once('@')
    {
        return format!("https://{host}/{path}");
    }
    url.to_owned()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    use glave_ports::testing::TempDir;

    use super::*;

    /// Set on the server the first test starts, so the second does nothing when run by anything else.
    const AS_THE_SERVER: &str = "GLAVE_TEST_AS_THE_SERVER";

    #[test]
    fn a_helper_named_by_the_repository_the_server_started_in_never_runs() {
        let dir = TempDir::new("credential-from-no-repository").unwrap();
        let (home, repo) = (dir.join("home"), dir.join("repo"));
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(repo.join(".git/objects")).unwrap();
        std::fs::create_dir_all(repo.join(".git/refs")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let script = |name: &str, text: &str| {
            let path = home.join("bin").join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{text}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let ran = dir.join("ran");
        let planted = script("planted", &format!(": > '{}'", ran.display()));
        let theirs = script(
            "theirs",
            "test \"$1\" = get || exit 0\nprintf 'username=someone\\npassword=%s\\n' \"$GH_TOKEN\"",
        );
        let config = |helper: &Path| {
            format!(
                "[credential]\n\thelper =\n\thelper = !{}\n",
                helper.display()
            )
        };
        std::fs::write(home.join(".gitconfig"), config(&theirs)).unwrap();
        std::fs::write(
            repo.join(".git/config"),
            format!(
                "[core]\n\trepositoryformatversion = 0\n{}",
                config(&planted)
            ),
        )
        .unwrap();
        // A program just written starts slowly on a busy Mac, the system checking it first (over
        // 20 seconds, measured): its first start isn't what the limit on the helper is for.
        let mut first_start = glave_process::Program::new(&theirs);
        let started = glave_ports::block_on!(
            Processes::unrecorded().output(&mut first_start, glave_ports::testing::HUNG)
        )
        .unwrap();
        assert!(started.status.success());
        let mut server = glave_process::Program::new(std::env::current_exe().unwrap());
        server
            .args([
                "--exact",
                "worker::credentials::tests::the_server_asks_for_a_credential",
                "--ignored",
            ])
            .current_dir(&repo)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GH_TOKEN", glave_ports::testing::any_secret())
            .env(AS_THE_SERVER, "1");
        let done = glave_ports::block_on!(
            Processes::unrecorded().output(&mut server, Duration::from_secs(60))
        )
        .unwrap();
        let said = String::from_utf8_lossy(&done.stdout);
        assert!(done.status.success(), "{said}");
        assert!(
            said.contains("1 passed"),
            "the server's side never ran: {said}"
        );
        assert!(!ran.exists(), "the repository's helper ran");
    }

    /// The server's side of the test above: its git asks, and the person's helper answers.
    #[test]
    #[ignore = "run by the test above, as a server started in the repository it makes"]
    fn the_server_asks_for_a_credential() {
        if std::env::var_os(AS_THE_SERVER).is_none() {
            return;
        }
        let git = WorkerCredentials::HostGit(Processes::unrecorded());
        let credential = glave_ports::block_on!(git.for_url("https://git.invalid/acme/app.git"))
            .unwrap()
            .unwrap();
        assert_eq!(credential.username, "someone");
    }

    #[test]
    fn ssh_remotes_become_https_and_others_stay() {
        assert_eq!(
            https_url("git@github.com:acme/app.git"),
            "https://github.com/acme/app.git"
        );
        assert_eq!(
            https_url("ssh://git@gitlab.example.com:2222/acme/app.git"),
            "https://gitlab.example.com/acme/app.git"
        );
        let https = "https://github.com/acme/app.git";
        assert_eq!(https_url(https), https);
        assert_eq!(https_url("/srv/repos/app.git"), "/srv/repos/app.git");
    }

    #[test]
    fn git_answers_are_read_and_only_https_gets_a_credential() {
        let credential = parse("protocol=https\nhost=h\nusername=me\npassword=s3cret\n").unwrap();
        assert_eq!(
            (credential.username.as_str(), credential.password.expose()),
            ("me", "s3cret")
        );
        assert!(parse("protocol=https\nhost=h\n").is_none());
        let fixed = WorkerCredentials::Fixed(credential);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert!(
            runtime
                .block_on(fixed.for_url("https://h/r.git"))
                .unwrap()
                .is_some()
        );
        assert!(
            runtime
                .block_on(fixed.for_url("/srv/r.git"))
                .unwrap()
                .is_none()
        );
    }
}
