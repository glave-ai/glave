use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::block_on;
use glave_ports::computer::{
    Command, Computers, CredentialHost, Ending, Environment, Forge, GitCredential, Output, RunsFor,
    Sandbox, SandboxLevel, SandboxNetwork, Sources,
};
use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FixedGitCredentials, fake_secret};

use super::*;
use crate::local::sandbox::tests::enforced;
use crate::{CancelSwitch, LocalComputers};

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
    let places = Places {
        bin: Path::new("/tmp/w/bin"),
        helper: Path::new("/tmp/w/git-credential-glave"),
        empty: Path::new("/tmp/w/gh-home"),
    };
    let glaves = [("SSL_CERT_FILE".into(), "/etc/corp.pem".into())];
    let gh = wrapper::wrapper(
        ForgeTool::Gh,
        &host("https://github.com", Forge::Github),
        &places,
        &glaves,
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
        "GH_REPO",
        "BROWSER",
        "EDITOR",
    ] {
        assert!(gh.contains(&format!(" {cleared}")), "{cleared}: {gh}");
    }
    for fixed in [
        "GH_CONFIG_DIR='/tmp/w/gh-home'",
        "XDG_DATA_HOME='/tmp/w/gh-home'",
        "GH_PAGER='cat'",
        "SSL_CERT_FILE='/etc/corp.pem'",
    ] {
        assert!(gh.contains(fixed), "{fixed}: {gh}");
    }
    assert!(gh.contains("unset SSL_CERT_DIR"), "{gh}");
    let tea = wrapper::wrapper(
        ForgeTool::Tea,
        &host("https://code.example.com", Forge::Gitea),
        &places,
        &[],
    );
    assert!(
        tea.contains("GITEA_INSTANCE_URL='https://code.example.com'; export GITEA_INSTANCE_URL"),
        "{tea}"
    );
    assert!(tea.contains(" GITEA_INSTANCE_INSECURE"), "{tea}");
    assert!(tea.contains("then GITEA_TOKEN=$token"), "{tea}");
}

/// The gh wrapper asks for the token only when gh is not pointed at another host.
#[test]
fn gh_pointed_at_another_host_runs_without_the_token() {
    let dir = glave_ports::testing::TempDir::new("wrapper").unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::create_dir_all(dir.join("real")).unwrap();
    let helper = dir.join("git-credential-glave");
    let token = fake_secret("ghp_");
    std::fs::write(&helper, format!("#!/bin/sh\necho password={token}\n")).unwrap();
    let gh = dir.join("real/gh");
    std::fs::write(&gh, "#!/bin/sh\necho \"token=${GH_TOKEN:-none}\"\n").unwrap();
    let places = Places {
        bin: &dir.join("bin"),
        helper: &helper,
        empty: &dir,
    };
    let script = wrapper::wrapper(
        ForgeTool::Gh,
        &host("https://github.com", Forge::Github),
        &places,
        &[],
    );
    std::fs::write(dir.join("bin/gh"), script).unwrap();
    for file in [&helper, &gh, &dir.join("bin/gh")] {
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(file)
            .status()
            .unwrap();
    }
    let run = |args: &[&str]| {
        let output = std::process::Command::new(dir.join("bin/gh"))
            .args(args)
            .env(
                "PATH",
                format!(
                    "{}:{}:/usr/bin:/bin",
                    dir.join("bin").display(),
                    dir.join("real").display()
                ),
            )
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    let given = format!("token={token}");
    assert_eq!(run(&["pr", "list"]), given);
    assert_eq!(run(&["pr", "list", "-R", "acme/app"]), given);
    assert_eq!(
        run(&["pr", "create", "--title", "Fix the localhost redirect"]),
        given,
        "a title is text"
    );
    for elsewhere in [
        &["api", "--hostname", "github.localhost", "user"][..],
        &["api", "http://api.GitHub.LocalHost/user"],
        &["pr", "view", "1", "-cRgithub.localhost/o/r"],
        &["repo", "view", "github.localhost/o/r"],
        &["repo", "clone", "https://acme.ghe.com/o/r"],
        &["pr", "list", "-R", "git@github.localhost:o/r"],
    ] {
        assert_eq!(run(elsewhere), "token=none", "{elsewhere:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_helper_files_hold_no_token_and_go_with_the_command() {
    let token = fake_secret("ghp_");
    let credentials = FixedGitCredentials::new(
        vec![
            host("https://github.com", Forge::Github),
            host("https://x\ntouch pwned", Forge::Other),
        ],
        GitCredential::new("x-access-token", &token).unwrap(),
    );
    let folder = block_on!(async {
        let offered = offer(&credentials, &[], None).await.unwrap().unwrap();
        assert_eq!(offered.hosts.len(), 1, "only a plain origin is offered");
        let folder = offered.folder.clone();
        for name in [HELPER, "bin/gh"] {
            let text = std::fs::read_to_string(folder.join(name)).unwrap();
            assert!(!text.contains(&token), "the token is in {name}");
            assert!(
                !text.contains(&offered.password),
                "the helper's password is in {name}"
            );
        }
        assert_eq!(
            value(&offered.environment, PASSWORD),
            Some(offered.password.as_str()),
            "the password is the command's environment's alone"
        );
        let path = value(&offered.environment, "PATH").unwrap();
        assert!(path.ends_with("/bin"), "the wrappers alone: {path}");
        assert!(!folder.join("bin").join(HELPER).exists());
        folder
    });
    assert!(!folder.exists(), "removed when the command ends");
    let none = FixedGitCredentials::new(Vec::new(), GitCredential::new("a", &token).unwrap());
    assert!(block_on!(offer(&none, &[], None)).unwrap().is_none());
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
fn scratch(name: &str) -> glave_ports::testing::TempDir {
    glave_ports::testing::TempDir::outside_temp(name).unwrap()
}

/// In the sandbox, with the network on, git still reaches the helper on this computer and
/// signs in. Runs only where the sandbox is (macOS, Linux with bubblewrap).
#[test]
fn a_sandboxed_command_signs_in_through_the_helper() {
    if !enforced() {
        return;
    }
    let dir = scratch("credentials");
    let token = fake_secret("ghp_");
    let fake = FakeGitHost::start("x-access-token", &token).unwrap();
    let credentials = FixedGitCredentials::new(
        vec![host(&fake.url, Forge::Other)],
        GitCredential::new("x-access-token", &token).unwrap(),
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
            network: SandboxNetwork::Direct,
            home: std::env::var_os("HOME").map(PathBuf::from),
            glave_home: None,
            runs_for: RunsFor::Owner,
        });
        computers
            .run_with_credentials(
                here.id,
                &command,
                &output,
                &CancelSwitch::default(),
                &credentials,
            )
            .await
            .unwrap()
    });
    let printed = output.0.lock().unwrap().clone();
    assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{printed}");
    assert!(printed.contains(FAKE_COMMIT), "{printed}");
    assert!(!printed.contains(&token), "the token shows: {printed}");
}

/// A computer whose person is signed in to GitHub, glave's folder under `dir`.
struct SignedIn {
    computers: LocalComputers,
    credentials: FixedGitCredentials,
    /// What the helper answers with.
    token: String,
    /// glave's own folder for commands, where helpers are made.
    on_path: PathBuf,
    glave_home: PathBuf,
}

impl SignedIn {
    fn new(dir: &Path) -> Self {
        let token = fake_secret("ghp_");
        let credentials = FixedGitCredentials::new(
            vec![host("https://github.com", Forge::Other)],
            GitCredential::new("x-access-token", &token).unwrap(),
        );
        let glave_home = dir.join(".glave");
        let on_path = glave_home.join("bin");
        std::fs::create_dir_all(&on_path).unwrap();
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH");
        let computers =
            LocalComputers::new(dir.join("copies"), environment).with_glave_on_path(&on_path);
        Self {
            computers,
            credentials,
            token,
            on_path,
            glave_home,
        }
    }

    /// `script` in the sandbox that hides glave's folder, `dir` its home.
    fn sandboxed(&self, dir: &Path, network: SandboxNetwork, script: impl Into<String>) -> Command {
        let mut command = Command::new(script);
        command.sandbox = Some(Sandbox {
            level: SandboxLevel::Project,
            network,
            home: Some(dir.to_path_buf()),
            glave_home: Some(self.glave_home.clone()),
            runs_for: RunsFor::Owner,
        });
        command
    }
}

/// A command's helper is made in glave's own folder for commands, which every sandbox keeps
/// readable and never writable: a sandboxed command cannot change it, nor add to its folder,
/// so no command can change another's. Runs only where the sandbox is.
#[test]
fn a_command_s_helper_is_where_no_sandboxed_command_can_change_it() {
    if !enforced() {
        return;
    }
    let dir = scratch("credentials-kept");
    let signed_in = SignedIn::new(&dir);
    let (computers, credentials, on_path) = (
        &signed_in.computers,
        &signed_in.credentials,
        &signed_in.on_path,
    );
    let output = Collect::default();
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let sources = Sources::Folder {
            path: dir.to_string_lossy().into_owned(),
        };
        let here = computers.open(computer.id, &sources).await.unwrap();
        // The helper's path is git's setting for it, after the cleared one.
        let command = signed_in.sandboxed(
            &dir,
            SandboxNetwork::Off,
            "helper=$(printf '%s' \"$GIT_CONFIG_VALUE_1\" | sed \"s/^!'//; s/'$//\"); \
             echo \"helper=$helper\"; \
             echo changed > \"$helper\" && echo wrote-helper; \
             touch \"$(dirname \"$helper\")/added\" && echo added-file; true",
        );
        let finished = computers
            .run_with_credentials(
                here.id,
                &command,
                &output,
                &CancelSwitch::default(),
                credentials,
            )
            .await
            .unwrap();
        assert_eq!(finished.ending, Ending::Exited { code: 0 });
    });
    let printed = output.0.lock().unwrap().clone();
    let helper = printed
        .lines()
        .find_map(|line| line.strip_prefix("helper="))
        .unwrap_or_else(|| panic!("no helper in {printed}"));
    assert!(
        Path::new(helper).starts_with(on_path),
        "not in glave's folder: {printed}"
    );
    assert!(!printed.contains("wrote-helper"), "{printed}");
    assert!(!printed.contains("added-file"), "{printed}");
}

/// A command's helper answers that command alone. Every sandbox reads the folder helpers are
/// made in, so another command on the computer, someone else's there, finds a running
/// command's helper and runs it as git would: it must be given nothing. Runs only where the
/// sandbox is; named for it, so CI's sandbox jobs, which pick tests by that word, run it
/// under bubblewrap too.
#[test]
fn another_sandboxed_command_that_runs_a_command_s_helper_is_given_nothing() {
    if !enforced() {
        return;
    }
    let dir = scratch("credentials-apart");
    let signed_in = SignedIn::new(&dir);
    let (computers, credentials) = (&signed_in.computers, &signed_in.credentials);
    let sandboxed = |script: String| signed_in.sandboxed(&dir, SandboxNetwork::Direct, script);
    // The command with credentials asks its own helper, then stays until the other has asked.
    let ask = "printf 'protocol=https\\nhost=github.com\\n\\n'";
    let own = sandboxed(format!(
        "helper=$(printf '%s' \"$GIT_CONFIG_VALUE_1\" | sed \"s/^!'//; s/'$//\"); \
         {ask} | \"$helper\" get | sed -n 's/^password=.*/own-helper-answered/p'; \
         i=0; while [ ! -e asked ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i + 1)); done; \
         test -e asked && echo asked-seen"
    ));
    // The other finds that helper in the folder every sandbox reads, and runs it itself; then
    // looks for its password where the first command's processes hold it.
    let other = sandboxed(format!(
        "i=0; helper=; while [ -z \"$helper\" ] && [ $i -lt 400 ]; do \
         helper=$(ls '{bin}'/glave-credentials-*/git-credential-glave 2>/dev/null | head -n 1); \
         sleep 0.05; i=$((i + 1)); done; \
         test -n \"$helper\" && echo helper-found; \
         {ask} | bash \"$helper\" get; \
         {{ ps eww -ax 2>/dev/null; cat /proc/[0-9]*/environ 2>/dev/null | tr '\\0' '\\n'; }} | \
         grep -q '{PASSWORD}=[0-9a-f]' && echo environment-read; touch asked",
        bin = signed_in.on_path.display()
    ));
    let (theirs, others) = (Collect::default(), Collect::default());
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let sources = Sources::Folder {
            path: dir.to_string_lossy().into_owned(),
        };
        let here = computers.open(computer.id, &sources).await.unwrap();
        let cancel = CancelSwitch::default();
        let (owned, asked) = tokio::join!(
            computers.run_with_credentials(here.id, &own, &theirs, &cancel, credentials),
            computers.run(here.id, &other, &others, &cancel),
        );
        assert_eq!(owned.unwrap().ending, Ending::Exited { code: 0 });
        assert_eq!(asked.unwrap().ending, Ending::Exited { code: 0 });
    });
    let (theirs, others) = (
        theirs.0.lock().unwrap().clone(),
        others.0.lock().unwrap().clone(),
    );
    assert!(theirs.contains("own-helper-answered"), "{theirs}");
    assert!(theirs.contains("asked-seen"), "{theirs}");
    assert!(others.contains("helper-found"), "{others}");
    assert!(
        !others.contains("environment-read"),
        "another command read the first one's environment"
    );
    assert!(
        !others.contains(&signed_in.token) && !others.contains("password="),
        "another command was given the credential"
    );
}
