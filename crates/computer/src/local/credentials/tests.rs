use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FixedCredentials};
use glave_ports::{
    Command, Computers, CredentialHost, Ending, Environment, Forge, GitCredential, Output, Sandbox,
    SandboxLevel, Sources, block_on,
};

use super::*;
use crate::{LocalComputers, Switch, sandbox_available};

fn host(url: &str, forge: Forge) -> CredentialHost {
    CredentialHost {
        url: url.into(),
        forge,
    }
}

fn value<'a>(settings: &'a [(OsString, OsString)], name: &str) -> Option<&'a str> {
    settings
        .iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| value.to_str())
}

#[test]
fn git_settings_clear_other_helpers_per_host_after_the_commands_own() {
    let hosts = [host("https://github.com", Forge::Github)];
    let own = [("GIT_CONFIG_COUNT".into(), "1".into())];
    let settings = settings(&hosts, Path::new("/tmp/glave x/git-credential-glave"), &own);
    assert_eq!(value(&settings, "GIT_CONFIG_COUNT"), Some("3"));
    assert_eq!(
        value(&settings, "GIT_CONFIG_KEY_1"),
        Some("credential.https://github.com.helper")
    );
    assert_eq!(value(&settings, "GIT_CONFIG_VALUE_1"), Some(""));
    assert_eq!(
        value(&settings, "GIT_CONFIG_VALUE_2"),
        Some("!'/tmp/glave x/git-credential-glave'")
    );
}

#[test]
fn a_wrapper_sets_its_token_alone_and_puts_back_what_could_run_a_program() {
    let gh = wrapper(
        ForgeTool::Gh,
        &host("https://github.com", Forge::Github),
        Path::new("/tmp/w/bin"),
        Path::new("/tmp/w/git-credential-glave"),
    );
    assert!(
        gh.contains("printf 'protocol=%s\\nhost=%s\\n' 'https' 'github.com'"),
        "{gh}"
    );
    assert!(
        gh.contains("then GH_TOKEN=$token; export GH_TOKEN; fi"),
        "{gh}"
    );
    for cleared in [
        "GH_ENTERPRISE_TOKEN",
        "GH_HOST",
        "GH_CONFIG_DIR",
        "BROWSER",
        "EDITOR",
    ] {
        assert!(gh.contains(&format!(" {cleared}")), "{cleared}: {gh}");
    }
    assert!(gh.contains("GH_PAGER='cat'; export GH_PAGER"), "{gh}");
    let tea = wrapper(
        ForgeTool::Tea,
        &host("https://code.example.com", Forge::Gitea),
        Path::new("/tmp/w/bin"),
        Path::new("/tmp/w/git-credential-glave"),
    );
    assert!(
        tea.contains("GITEA_INSTANCE_URL='https://code.example.com'; export GITEA_INSTANCE_URL"),
        "{tea}"
    );
    assert!(tea.contains("then GITEA_TOKEN=$token"), "{tea}");
}

#[test]
fn the_helper_files_hold_no_token_and_go_with_the_command() {
    let credentials = FixedCredentials::new(
        vec![
            host("https://github.com", Forge::Github),
            host("https://x\ntouch pwned", Forge::Other),
        ],
        GitCredential::new("x-access-token", "ghp_never_in_a_file").unwrap(),
    );
    let folder = block_on!(async {
        let offered = offer(&credentials, &[]).await.unwrap().unwrap();
        assert_eq!(offered.hosts.len(), 1, "only a plain origin is offered");
        let folder = offered.folder.clone();
        for name in [HELPER, "bin/gh"] {
            let text = std::fs::read_to_string(folder.join(name)).unwrap();
            assert!(!text.contains("ghp_never_in_a_file"), "{text}");
        }
        let path = value(&offered.environment, "PATH").unwrap();
        assert!(path.ends_with("/bin"), "the wrappers alone: {path}");
        assert!(!folder.join("bin").join(HELPER).exists());
        folder
    });
    assert!(!folder.exists(), "removed when the command ends");
    let none = FixedCredentials::new(Vec::new(), GitCredential::new("a", "b").unwrap());
    assert!(block_on!(offer(&none, &[])).unwrap().is_none());
}

#[derive(Default)]
struct Collect(std::sync::Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

/// A folder outside the system temp folder, which the sandbox lets every command write.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sandbox-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

/// In the sandbox, with the network on, git still reaches the helper on this computer and
/// signs in. Runs only where the sandbox is (macOS, Linux with bubblewrap).
#[test]
fn a_sandboxed_command_signs_in_through_the_helper() {
    if !sandbox_available() {
        return;
    }
    let dir = scratch("credentials");
    let token = "ghp_sandboxed0token";
    let fake = FakeGitHost::start("x-access-token", token).unwrap();
    let credentials = FixedCredentials::new(
        vec![host(&fake.url, Forge::Other)],
        GitCredential::new("x-access-token", token).unwrap(),
    );
    let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
    let computers = LocalComputers::new(dir.join("copies"), environment);
    let output = Collect::default();
    let finished = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let sources = Sources::Folder {
            path: dir.to_string_lossy().into_owned(),
        };
        let here = computers.open(computer.id, &sources).await.unwrap();
        let mut command = Command::new(format!(
            "GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
             git ls-remote {}/repo.git",
            fake.url
        ));
        command.sandbox = Some(Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: std::env::var_os("HOME").map(PathBuf::from),
            glave_home: None,
        });
        computers
            .run_with_credentials(here.id, &command, &output, &Switch::default(), &credentials)
            .await
            .unwrap()
    });
    let printed = output.0.lock().unwrap().clone();
    assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{printed}");
    assert!(printed.contains(FAKE_COMMIT), "{printed}");
    assert!(!printed.contains(token), "{printed}");
}
