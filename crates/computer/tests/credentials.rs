//! What the person's credential helper is given of the server's environment, seen by a helper
//! of the test's own. A program of its own is the server here, since a test can't change its
//! own environment, and only a new program's shows what `git credential fill` inherits.

use std::os::unix::fs::PermissionsExt as _;
use std::process::Stdio;
use std::time::Duration;

use glave_computer::WorkerCredentials;
use glave_ports::block_on;
use glave_ports::testing::{TempDir, any_secret, fake_secret};
use glave_process::{Processes, Program};

/// Set on the server the first test starts, so the second does nothing when run by anything else.
const AS_THE_SERVER: &str = "GLAVE_TEST_AS_THE_SERVER";

/// What a helper may be relying on, each a place, a setting or a sign-in of the person's for git,
/// none of them anywhere on this computer. In the case each is written, since either is kept.
const NEEDED: &[(&str, &str)] = &[
    ("SSH_AUTH_SOCK", "/nonexistent/agent.sock"),
    ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus"),
    ("XDG_RUNTIME_DIR", "/nonexistent/run"),
    ("XDG_CONFIG_HOME", "/nonexistent/config"),
    ("GH_CONFIG_DIR", "/nonexistent/gh"),
    ("GCM_CREDENTIAL_STORE", "plaintext"),
    ("https_proxy", "http://proxy.invalid"),
];

#[test]
fn a_credential_helper_gets_what_it_needs_and_none_of_the_server_s_own() {
    let dir = TempDir::new("credential-helper").unwrap();
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let helper = dir.join("helper");
    std::fs::write(
        &helper,
        "#!/bin/sh\nenv > \"$HOME/seen\"\ntest \"$1\" = get || exit 0\n\
         printf 'username=someone\\npassword=%s\\n' \"$GH_TOKEN\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        // The empty value first: no helper configured anywhere else on this computer is asked.
        format!(
            "[credential]\n\thelper =\n\thelper = !{}\n",
            helper.display()
        ),
    )
    .unwrap();
    let (key, password) = (any_secret(), any_secret());
    let store = format!("postgres://glave:{password}@db/glave");
    let mut server = Program::new(std::env::current_exe().unwrap());
    server
        .args([
            "--exact",
            "credentials::the_server_asks_for_a_credential",
            "--ignored",
        ])
        .current_dir(&dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", &home)
        .env(AS_THE_SERVER, "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .envs(NEEDED.iter().copied())
        // Named as a secret is, and how `gh auth git-credential` signs in without a file.
        .env("GH_TOKEN", fake_secret("ghp_"))
        .env("GLAVE_API_KEY", &key)
        .env("AWS_SECRET_ACCESS_KEY", &key)
        // Named as nothing a filter by name could catch.
        .env("GLAVE_STORE_URL", &store)
        .env("DATABASE_URL", &store)
        .env("GLAVE_BASE_URL", format!("https://glave:{password}@models"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let processes = Processes::unrecorded();
    let ran = block_on!(processes.output(&mut server, Duration::from_secs(60))).unwrap();
    assert!(
        ran.status.success(),
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    let seen = std::fs::read_to_string(home.join("seen")).unwrap();
    let has = |name: &str| {
        seen.lines()
            .any(|line| line.starts_with(&format!("{name}=")))
    };
    for (name, _) in NEEDED {
        assert!(has(name), "{name} never reached the helper");
    }
    for name in [
        "GLAVE_API_KEY",
        "AWS_SECRET_ACCESS_KEY",
        "GLAVE_STORE_URL",
        "DATABASE_URL",
        "GLAVE_BASE_URL",
        AS_THE_SERVER,
    ] {
        assert!(!has(name), "{name} reached the helper");
    }
    assert!(!seen.contains(&key), "the model's key reached the helper");
    assert!(
        !seen.contains(&password),
        "the store's password reached the helper"
    );
}

/// The server's side of the test above: its git asks, and the helper answers.
#[test]
#[ignore = "run by the test above, as a server with the environment it sets"]
fn the_server_asks_for_a_credential() {
    if std::env::var_os(AS_THE_SERVER).is_none() {
        return;
    }
    let git = WorkerCredentials::HostGit(Processes::unrecorded());
    let credential = block_on!(git.for_url("https://git.invalid/acme/app.git"))
        .unwrap()
        .unwrap();
    assert_eq!(credential.username, "someone");
    let token = std::env::var("GH_TOKEN").unwrap();
    assert!(
        credential.password.expose() == token,
        "the helper's answer is not what it was given to answer with"
    );
}
