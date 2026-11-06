//! A worker's save and what its commands planted: its repository's config and its home folder are
//! theirs to write, so nothing in them chooses what reads the push's credential, or where it goes.
//! Each way is shown to reach it under plain git and shown not to under `save`.

use glave_ports::computer::{Bytes, GitCredential, Save, Saved};
use glave_ports::testing::fake_secret;

use super::*;
use crate::local::tests::asks_for_a_password;

/// What a save pushes to, for the tests to look in.
const BRANCH: &str = "glave/wip/test";

/// A workspace opened as a worker's is, from a clone with a credential.
struct Worker {
    computers: LocalComputers,
    opened: WorkspaceInfo,
    origin: PathBuf,
    secret: String,
    /// Which program, planted, saw which credential.
    told: PathBuf,
    /// Last, so what the computers started stops before its folder goes.
    planted: Planted,
}

fn worker() -> Worker {
    let planted = planted();
    let origin = planted.dir.join("origin.git");
    must(
        &planted,
        &["clone", "-q", "--bare", ".", &origin.to_string_lossy()],
    );
    let told = planted.dir.join("told");
    executable(
        &planted.program,
        &format!(
            "#!/bin/sh\necho \"ran $GLAVE_GIT_PASSWORD\" >> '{}'\nexit 1\n",
            told.display()
        ),
    );
    let secret = fake_secret("");
    let credential = GitCredential::new("glave-user", &secret).unwrap();
    let computers = LocalComputers::new(planted.dir.join("copies"), environment(&planted.dir));
    let sources = Sources::Git {
        url: origin.to_string_lossy().into_owned(),
        at: None,
    };
    let opened = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        computers
            .open_with_credential(computer.id, &sources, &credential)
            .await
            .unwrap()
    });
    Worker {
        computers,
        opened,
        origin,
        secret,
        told,
        planted,
    }
}

impl Worker {
    fn root(&self) -> &Path {
        Path::new(&self.opened.root)
    }

    /// Writes this in the repository's config, as a command there can.
    fn config(&self, key: &str, value: &str) {
        let done = plain(&self.planted, self.root(), &["config", key, value]);
        assert!(done.status.success(), "{key}");
    }

    /// Plain git, handed the credential as `save` hands it, reaches what was planted with `args`.
    fn plain_git_hands_it_over(&self, args: &[&str]) {
        let mut git = Program::new("git");
        git.args(args)
            .current_dir(self.root())
            .env_clear()
            .envs(environment(&self.planted.dir))
            .env("GLAVE_GIT_PASSWORD", &self.secret)
            .no_input()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        block_on!(Processes::unrecorded().output(&mut git, Duration::from_secs(60))).unwrap();
        assert!(
            self.read_told().contains(&self.secret),
            "plain git never reached the planted program"
        );
        std::fs::remove_file(&self.told).unwrap();
    }

    fn read_told(&self) -> String {
        std::fs::read_to_string(&self.told).unwrap_or_default()
    }

    async fn saved(&self) -> Result<Saved, ComputerError> {
        let save = Save {
            branch: BRANCH.into(),
            message: "work".into(),
        };
        self.computers.save(self.opened.id, &save).await
    }

    /// `save` handed nothing to what was planted, and the work reached where the workspace came
    /// from.
    fn saves_keeping_the_credential(&self) {
        std::fs::write(self.root().join("work.txt"), "work\n").unwrap();
        let saved = block_on!(self.saved());
        assert!(
            !self.read_told().contains(&self.secret),
            "something the commands planted read the credential"
        );
        assert!(saved.unwrap().pushed);
        let at = format!("refs/heads/{BRANCH}:work.txt");
        let pushed = plain(&self.planted, &self.origin, &["show", &at]);
        assert!(pushed.status.success(), "the work never reached the origin");
    }

    /// The credential as git sends it over HTTP.
    fn basic(&self) -> String {
        let credential = format!("glave-user:{}", self.secret);
        let basic = serde_json::to_value(Bytes::new(credential)).unwrap();
        format!("Basic {}", basic.as_str().unwrap())
    }

    /// What was planted in the repository's `remote.origin` sends the push there.
    fn point_origin_at(&self, url: &str) {
        self.config("remote.origin.url", url);
    }

    fn program(&self) -> String {
        self.planted.program.to_string_lossy().into_owned()
    }
}

/// Plain git's push.
const PUSH: [&str; 3] = ["push", "origin", "HEAD:refs/heads/plain"];

#[test]
fn a_signing_program_the_config_names_never_sees_the_credential() {
    let worker = worker();
    worker.config("commit.gpgSign", "true");
    worker.config("gpg.program", &worker.program());
    worker.plain_git_hands_it_over(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--allow-empty",
        "-qm",
        "signed",
    ]);
    worker.saves_keeping_the_credential();
}

#[test]
fn a_receiving_program_the_config_names_never_sees_the_credential() {
    let worker = worker();
    worker.config("remote.origin.receivepack", &worker.program());
    worker.plain_git_hands_it_over(&PUSH);
    worker.saves_keeping_the_credential();
}

#[test]
fn an_ssh_command_the_config_names_never_sees_the_credential() {
    let worker = worker();
    worker.config("core.sshCommand", &worker.program());
    worker.point_origin_at("ssh://example.invalid/r.git");
    worker.plain_git_hands_it_over(&PUSH);
    worker.saves_keeping_the_credential();
}

#[test]
fn a_transport_the_config_allows_never_sees_the_credential() {
    let worker = worker();
    worker.config("protocol.ext.allow", "always");
    worker.point_origin_at(&format!("ext::{}", worker.program()));
    worker.plain_git_hands_it_over(&PUSH);
    worker.saves_keeping_the_credential();
}

#[test]
fn what_the_home_folder_names_never_sees_the_credential() {
    let worker = worker();
    let origin = worker.origin.to_string_lossy();
    std::fs::write(
        worker.planted.dir.join("home/.gitconfig"),
        format!(
            "[protocol \"ext\"]\n\tallow = always\n[url \"ext::{}\"]\n\tinsteadOf = {origin}\n",
            worker.program(),
        ),
    )
    .unwrap();
    worker.plain_git_hands_it_over(&PUSH);
    worker.saves_keeping_the_credential();
}

fn listening() -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let at = listener.local_addr().unwrap().to_string();
    (listener, at)
}

/// Answers on `listener`, recording what it is sent, until stopped.
fn serving(listener: std::net::TcpListener) -> tokio::task::JoinHandle<Vec<String>> {
    tokio::spawn(asks_for_a_password(
        tokio::net::TcpListener::from_std(listener).unwrap(),
    ))
}

/// What `server` was sent, if git got as far as sending a password; it is stopped otherwise.
async fn sent(server: tokio::task::JoinHandle<Vec<String>>) -> Vec<String> {
    if server.is_finished() {
        server.await.unwrap()
    } else {
        server.abort();
        Vec::new()
    }
}

impl Worker {
    /// Plain git pushing to `to`, with a helper that hands it the credential: what `save` does.
    async fn plain_push(&self, to: &str) {
        let helper = format!(
            "credential.helper=!f() {{ echo username=glave-user; echo password={}; }}; f",
            self.secret
        );
        let mut git = Program::new("git");
        git.args(["-c", &helper, "push", to, "HEAD:refs/heads/plain"])
            .current_dir(self.root())
            .env_clear()
            .envs(environment(&self.planted.dir))
            .env("GIT_TERMINAL_PROMPT", "0")
            .no_input()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Processes::unrecorded()
            .output(&mut git, Duration::from_secs(20))
            .await
            .unwrap();
    }
}

#[test]
fn a_host_the_config_names_never_gets_the_credential() {
    let worker = worker();
    let (control, control_at) = listening();
    let (listener, listening_at) = listening();
    worker.point_origin_at(&format!("http://{control_at}/r.git"));
    let reached_under_plain_git = block_on!(async {
        let server = serving(control);
        worker.plain_push("origin").await;
        sent(server).await
    });
    assert!(reached_under_plain_git.contains(&worker.basic()));

    worker.point_origin_at(&format!("http://{listening_at}/r.git"));
    std::fs::write(worker.root().join("work.txt"), "work\n").unwrap();
    let reached = block_on!(async {
        let server = serving(listener);
        let _ = worker.saved().await;
        sent(server).await
    });
    assert_eq!(
        reached,
        Vec::<String>::new(),
        "the planted host was reached"
    );
    worker.saves_keeping_the_credential();
}

/// A clone over HTTP needs a real server, so these go to the push itself, which does cross HTTP:
/// to its own URL, whichever way the config names a proxy, each shown to work on plain git.
#[test]
fn a_push_over_http_goes_to_its_url_past_a_proxy_the_config_names() {
    for only_this_url in [false, true] {
        let worker = worker();
        let (real, real_at) = listening();
        let (proxy, proxy_at) = listening();
        let key = if only_this_url {
            format!("http.http://{real_at}.proxy")
        } else {
            "http.proxy".to_owned()
        };
        let (control, control_at) = listening();
        worker.config(&key, &format!("http://{control_at}"));
        let url = format!("http://{real_at}/r.git");
        let reached_under_plain_git = block_on!(async {
            let server = serving(control);
            worker.plain_push(&url).await;
            sent(server).await
        });
        assert!(
            reached_under_plain_git.contains(&worker.basic()),
            "plain git passed {key} by"
        );

        worker.config(&key, &format!("http://{proxy_at}"));
        let pushes = Pushes {
            url: url.clone(),
            credential: GitCredential::new("glave-user", &worker.secret).unwrap(),
        };
        let (reached, through_proxy) = block_on!(async {
            let opened = worker
                .computers
                .registry
                .workspace(worker.opened.id)
                .unwrap();
            let git = repository(&opened).await.unwrap();
            let hash = git.must(&["rev-parse", "HEAD"]).await.unwrap();
            let (real, proxy) = (serving(real), serving(proxy));
            let refspec = format!("{hash}:refs/heads/{BRANCH}");
            let refused = push_with_credential(&opened, &git, &pushes, &refspec).await;
            assert!(refused.is_err(), "that server lets no one in");
            (sent(real).await, sent(proxy).await)
        });
        assert_eq!(through_proxy, Vec::<String>::new(), "{key}");
        assert!(reached.contains(&worker.basic()), "{key}: it never arrived");
    }
}

#[test]
fn a_url_holding_a_password_is_never_said_in_an_error() {
    let worker = worker();
    let (listener, at) = listening();
    let pushes = Pushes {
        url: format!("http://glave-user:{}@{at}/r.git", worker.secret),
        credential: GitCredential::new("glave-user", &worker.secret).unwrap(),
    };
    let said = block_on!(async {
        let opened = worker
            .computers
            .registry
            .workspace(worker.opened.id)
            .unwrap();
        let git = repository(&opened).await.unwrap();
        let hash = git.must(&["rev-parse", "HEAD"]).await.unwrap();
        let server = serving(listener);
        let refspec = format!("{hash}:refs/heads/{BRANCH}");
        let refused = push_with_credential(&opened, &git, &pushes, &refspec).await;
        let _ = sent(server).await;
        refused.unwrap_err().to_string()
    });
    assert!(
        !said.contains(&worker.secret),
        "the error held the password"
    );
}

#[test]
fn a_save_leaves_nothing_to_save_again() {
    let worker = worker();
    worker.saves_keeping_the_credential();
    let left = plain(
        &worker.planted,
        worker.root(),
        &["rev-list", "HEAD", "--not", "--remotes"],
    );
    assert_eq!(String::from_utf8_lossy(&left.stdout).trim(), "");
}

#[test]
fn a_repository_of_either_hash_is_pushed() {
    let planted = planted();
    let origin = planted.dir.join("sha256.git");
    let origin_text = origin.to_string_lossy();
    must(
        &planted,
        &[
            "init",
            "-q",
            "--bare",
            "--object-format=sha256",
            &origin_text,
        ],
    );
    let secret = fake_secret("");
    let credential = GitCredential::new("glave-user", &secret).unwrap();
    let computers = LocalComputers::new(planted.dir.join("copies"), environment(&planted.dir));
    let sources = Sources::Git {
        url: origin_text.clone().into_owned(),
        at: None,
    };
    let save = Save {
        branch: BRANCH.into(),
        message: "work".into(),
    };
    let saved = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let opened = computers
            .open_with_credential(computer.id, &sources, &credential)
            .await
            .unwrap();
        computers.save(opened.id, &save).await.unwrap()
    });
    assert!(saved.pushed);
    assert_eq!(saved.commit.len(), 64);
}

#[test]
fn work_reaches_where_it_came_from_whatever_became_of_origin() {
    let worker = worker();
    let removed = plain(
        &worker.planted,
        worker.root(),
        &["remote", "remove", "origin"],
    );
    assert!(removed.status.success());
    worker.saves_keeping_the_credential();
}

#[test]
fn a_folder_saved_pushes_to_its_origin_and_without_one_keeps_the_commit() {
    let planted = planted();
    let computers = LocalComputers::new(planted.dir.join("copies"), environment(&planted.dir));
    let sources = Sources::Folder {
        path: planted.repo.to_string_lossy().into_owned(),
    };
    let save = Save {
        branch: BRANCH.into(),
        message: "work".into(),
    };
    let saved = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let opened = computers.open(computer.id, &sources).await.unwrap();
        computers.save(opened.id, &save).await.unwrap()
    });
    assert!(!saved.pushed, "no origin, so nothing to push to");
    assert_eq!(must(&planted, &["rev-parse", "HEAD"]), saved.commit);

    let origin = planted.dir.join("elsewhere.git");
    let origin = origin.to_string_lossy();
    must(&planted, &["init", "-q", "--bare", &origin]);
    must(&planted, &["remote", "add", "origin", &origin]);
    let saved = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let opened = computers.open(computer.id, &sources).await.unwrap();
        computers.save(opened.id, &save).await.unwrap()
    });
    assert!(saved.pushed);
    let at = format!("refs/heads/{BRANCH}");
    let pushed = plain(&planted, Path::new(&*origin), &["rev-parse", &at]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        saved.commit,
        "its push reached the origin"
    );
}

#[test]
fn nothing_is_left_behind_in_the_repository_by_a_push() {
    let worker = worker();
    worker.saves_keeping_the_credential();
    let git_folder: Vec<_> = std::fs::read_dir(worker.root().join(".git"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with("glave"))
        .collect();
    assert_eq!(git_folder, Vec::<OsString>::new());
}
