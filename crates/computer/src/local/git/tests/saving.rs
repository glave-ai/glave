//! A worker's save and what its commands planted: its repository's config and its home folder are
//! theirs to write, so nothing in them chooses what reads the push's credential, or where it goes.
//! Each way is shown to reach the credential under plain git and shown not to under `save`.

use glave_ports::computer::{Bytes, GitCredential, Save, Saved};
use glave_ports::testing::fake_secret;

use super::*;
use crate::local::tests::asks_for_a_password;

/// What a save pushes to, for the tests to look in.
const BRANCH: &str = "glave/wip/test";

/// A workspace opened as a worker's is, from a clone with a credential.
struct Worker {
    planted: Planted,
    computers: LocalComputers,
    opened: WorkspaceInfo,
    origin: PathBuf,
    secret: String,
    /// Which program, planted, saw which credential.
    told: PathBuf,
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
        planted,
        computers,
        opened,
        origin,
        secret,
        told,
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

/// What a place the commands chose was sent, when `plant` points git there and a save is made.
fn sent_where_planted(plant: impl FnOnce(&Worker, &str)) -> (Worker, Vec<String>, String) {
    let worker = worker();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    plant(&worker, &listener.local_addr().unwrap().to_string());
    std::fs::write(worker.root().join("work.txt"), "work\n").unwrap();
    let sent = block_on!(async {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let server = tokio::spawn(asks_for_a_password(listener));
        let _ = worker.saved().await;
        if server.is_finished() {
            server.await.unwrap()
        } else {
            server.abort();
            Vec::new()
        }
    });
    let basic = worker.basic();
    (worker, sent, basic)
}

#[test]
fn a_host_the_config_names_never_gets_the_credential() {
    let (worker, sent, basic) = sent_where_planted(|worker, listening| {
        worker.point_origin_at(&format!("http://{listening}/r.git"));
    });
    assert!(
        !sent.contains(&basic),
        "the planted host got the credential"
    );
    worker.saves_keeping_the_credential();
}

#[test]
fn a_proxy_the_config_names_never_gets_the_credential() {
    let (worker, sent, basic) = sent_where_planted(|worker, listening| {
        worker.point_origin_at("http://example.invalid/r.git");
        worker.config("http.proxy", &format!("http://{listening}"));
    });
    assert!(
        !sent.contains(&basic),
        "the planted proxy got the credential"
    );
    worker.saves_keeping_the_credential();
}

/// A clone over HTTP needs a real server, so this goes to the push itself: where the test above
/// shows a proxy planted getting the credential under `main`'s git, this shows one left out of a
/// push that really does cross HTTP.
#[test]
fn a_push_over_http_goes_to_its_url_past_a_proxy_the_config_names() {
    let worker = worker();
    let listen = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let at = listener.local_addr().unwrap();
        (listener, at)
    };
    let (real, real_at) = listen();
    let (proxy, proxy_at) = listen();
    worker.config("http.proxy", &format!("http://{proxy_at}"));
    worker.config("http.sslVerify", "false");
    let credential = GitCredential::new("glave-user", &worker.secret).unwrap();
    let (reached, through_proxy) = block_on!(async {
        let opened = worker
            .computers
            .registry
            .workspace(worker.opened.id)
            .unwrap();
        let git = repository(&opened).await.unwrap();
        let hash = git.must(&["rev-parse", "HEAD"]).await.unwrap();
        let real = tokio::spawn(asks_for_a_password(
            tokio::net::TcpListener::from_std(real).unwrap(),
        ));
        let proxy = tokio::spawn(asks_for_a_password(
            tokio::net::TcpListener::from_std(proxy).unwrap(),
        ));
        let refspec = format!("{hash}:refs/heads/{BRANCH}");
        let url = format!("http://{real_at}/r.git");
        let refused = pushing(&opened, &git, &url, Some(&credential), &refspec).await;
        assert!(refused.is_err(), "that server lets no one in");
        let through_proxy = proxy.is_finished();
        proxy.abort();
        (real.await.unwrap(), through_proxy)
    });
    assert!(!through_proxy, "the push went through the planted proxy");
    assert!(
        reached.contains(&worker.basic()),
        "the push never reached its url"
    );
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
