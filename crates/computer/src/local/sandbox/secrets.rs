//! What the sandbox hides from commands: credential stores, by where they are. Under the home
//! folder, one list for every platform and each platform's own beside it; and wherever the
//! environment moves those (`XDG_CONFIG_HOME` and the like). The project's own files, `.env`
//! among them, stay readable, so a test or a dev server that loads them works. Each platform's
//! sandbox turns these into its own rules.

use std::ffi::OsString;
use std::path::PathBuf;

/// Under the home folder, on every platform: where credentials live. Hidden whole, whether
/// they exist yet or not (on Linux, from the next command once they do).
///
/// A few files a secret can be in are left out on purpose, since their tools stop or go wrong
/// without them: `specs/sandbox.md`, *Reads*, names each and why.
pub(super) const HOME_SECRETS: &[&str] = &[
    // Keys and cloud credentials.
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".minikube",
    ".docker/config.json",
    ".docker/contexts",
    ".config/containers/auth.json",
    ".config/helm/registry/config.json",
    ".oci",
    ".boto",
    ".s3cfg",
    ".mc/config.json",
    ".config/doctl",
    ".config/hcloud",
    ".config/linode-cli",
    ".config/scw",
    ".aliyun",
    ".databrickscfg",
    ".snowflake",
    ".snowsql",
    ".kaggle",
    ".modal.toml",
    ".bluemix",
    ".config/openstack",
    ".vultr-cli.yaml",
    ".cloudflared",
    // Package registries and forges.
    ".netrc",
    ".authinfo",
    ".npmrc",
    ".pypirc",
    ".config/pypoetry/auth.toml",
    ".local/share/uv/credentials",
    ".pgpass",
    ".my.cnf",
    ".mylogin.cnf",
    // git-credential-store's two files, and where git-credential-cache's daemon listens.
    ".git-credentials",
    ".config/git/credentials",
    ".cache/git/credential",
    ".git-credential-cache",
    ".subversion/auth",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gem/credentials",
    ".local/share/gem/credentials",
    ".config/dart/pub-credentials.json",
    ".m2/settings.xml",
    ".m2/settings-security.xml",
    ".sbt/.credentials",
    ".ivy2/.credentials",
    ".gradle/gradle.properties",
    ".bundle/config",
    ".composer/auth.json",
    ".config/composer/auth.json",
    ".config/gh",
    ".config/hub",
    ".config/glab-cli",
    ".config/tea",
    // Infrastructure and secret managers.
    ".terraform.d",
    ".terraformrc",
    ".vault-token",
    ".config/op",
    ".op",
    ".password-store",
    ".config/sops",
    ".pulumi",
    ".config/pulumi",
    ".config/configstore",
    ".config/rclone",
    ".config/ngrok",
    ".ngrok2",
    ".rclone.conf",
    ".wrangler",
    ".config/.wrangler",
    ".vercel",
    ".local/share/com.vercel.cli",
    ".railway",
    ".supabase",
    ".config/stripe",
    ".sentryclirc",
    ".doppler",
    ".config/Bitwarden CLI",
    ".appstoreconnect",
    ".fastlane",
    ".expo/state.json",
    ".ansible/galaxy_token",
    ".config/netlify",
    ".fly",
    ".cache/huggingface/token",
    ".huggingface/token",
    // Other agents' and model tools' sign-ins.
    ".claude/.credentials.json",
    ".claude.json",
    ".claude.json.backup",
    ".cursor/mcp.json",
    ".config/github-copilot",
    ".local/share/opencode/auth.json",
    ".config/io.datasette.llm/keys.json",
    ".codex/auth.json",
    ".gemini",
    ".ollama/id_ed25519",
    // Shell and REPL histories, where pasted tokens end up.
    ".bash_history",
    ".zsh_history",
    ".zhistory",
    ".zsh_sessions",
    ".local/share/atuin",
    ".local/share/fish/fish_history",
    ".python_history",
    ".node_repl_history",
    ".psql_history",
    ".mysql_history",
    ".sqlite_history",
    ".rediscli_history",
    ".irb_history",
    // A running notebook server's token: whoever reads it runs code there.
    ".local/share/jupyter/runtime",
];

/// Under the home folder, on macOS besides the common list: the keychains, browser profiles
/// (cookies are credentials), and where tools that keep to macOS's own folders hold what the
/// common list names. Hiding the keychains is what keeps a command from them
/// (`specs/invariants.md`, 24): a file keychain is read from disk by the process that opens it,
/// and with the network on the security server stays reachable, since TLS roots are read
/// through it; the data-protection keychain's service (`com.apple.securityd.xpc`) never is.
#[cfg(any(target_os = "macos", test))]
pub(super) const MACOS_HOME_SECRETS: &[&str] = &[
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Firefox",
    "Library/Application Support/ngrok",
    // Where tools that keep to macOS's own folders hold what the common list names.
    "Library/Application Support/pypoetry/auth.toml",
    "Library/Application Support/com.vercel.cli",
    "Library/Application Support/Bitwarden CLI",
    "Library/Application Support/io.datasette.llm/keys.json",
    "Library/Preferences/.wrangler",
    "Library/Preferences/helm/registry/config.json",
    "Library/Application Support/doctl",
    "Library/Application Support/tea",
    "Library/Application Support/dart/pub-credentials.json",
    "Library/Application Support/Claude/claude_desktop_config.json",
    "Library/Jupyter/runtime",
];

/// Under the home folder, on Linux besides the common list: the X server's cookie, keyrings,
/// where GNU screen's sessions, Docker Desktop and the connections Ansible has ssh share listen
/// there (on macOS no command reaches a socket in a file), and browser profiles (cookies are
/// credentials), a snap's and a Flatpak's among them, as Ubuntu's own Firefox is.
#[cfg(target_os = "linux")]
const LINUX_HOME_SECRETS: &[&str] = &[
    // The X server's cookie: with it, a command that reaches the server could read and type
    // into every window.
    ".Xauthority",
    ".local/share/keyrings",
    ".local/share/kwalletd",
    ".screen",
    ".docker/desktop",
    ".ansible/cp",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
    "snap/firefox/common/.mozilla",
    "snap/chromium/common/chromium",
    ".var/app/org.mozilla.firefox/.mozilla",
    ".var/app/com.google.Chrome/config/google-chrome",
    ".var/app/org.chromium.Chromium/config/chromium",
    ".var/app/com.brave.Browser/config/BraveSoftware",
];

/// This platform's own list.
#[cfg(target_os = "macos")]
const OWN: &[&str] = MACOS_HOME_SECRETS;
#[cfg(target_os = "linux")]
const OWN: &[&str] = LINUX_HOME_SECRETS;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const OWN: &[&str] = &[];

/// Every credential store under the home folder on this platform, each as written from it:
/// the common list, then the platform's own.
pub(super) fn home_secrets() -> impl Iterator<Item = &'static str> {
    HOME_SECRETS.iter().chain(OWN).copied()
}

/// The folders of the home folder that the XDG base directory variables move, as the stores
/// above are written, each with its variable.
const MOVED: &[(&str, &str)] = &[
    (".config/", "XDG_CONFIG_HOME"),
    (".local/share/", "XDG_DATA_HOME"),
    (".cache/", "XDG_CACHE_HOME"),
];

/// The variable that moves the X server's cookie from `~/.Xauthority`, as display managers do
/// (to `/tmp` or the runtime folder): with it, a command that reaches the X server could read
/// and type into every window.
const X_COOKIE: &str = "XAUTHORITY";

/// Where `environment` moves the stores of `home_secrets`: one kept under `~/.config`,
/// `~/.local/share` or `~/.cache` is under `XDG_CONFIG_HOME`, `XDG_DATA_HOME` or
/// `XDG_CACHE_HOME` instead where that is set, and so is hidden there too. The last value of
/// each, as a command takes it; one that is not a full path is no folder to the tools either.
pub(crate) fn moved_stores(environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    let set = |variable: &str| set(environment, variable);
    MOVED
        .iter()
        .filter_map(|&(default, variable)| set(variable).map(|folder| (default, folder)))
        .flat_map(|(default, folder)| {
            home_secrets()
                .filter_map(move |store| store.strip_prefix(default))
                .map(move |store| folder.join(store))
        })
        .collect()
}

/// The full path the last `variable` in `environment` holds, as a command takes it.
fn set(environment: &[(OsString, OsString)], variable: &str) -> Option<PathBuf> {
    environment
        .iter()
        .rev()
        .find(|(name, _)| name == variable)
        .map(|(_, place)| PathBuf::from(place))
        .filter(|place| place.is_absolute())
}

/// The X server's cookie where `XAUTHORITY` in `environment` names a file: a folder named there
/// is no cookie, and hiding it could hide what commands need.
pub(crate) async fn x_cookie(environment: &[(OsString, OsString)]) -> Option<PathBuf> {
    let cookie = set(environment, X_COOKIE)?;
    let meta = tokio::fs::metadata(&cookie).await.ok()?;
    meta.is_file().then_some(cookie)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn environment(set: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        set.iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn a_store_is_hidden_where_the_environment_moves_it_too() {
        let moved = moved_stores(&environment(&[
            ("XDG_CONFIG_HOME", "/first"),
            ("XDG_CONFIG_HOME", "/x/config"),
            ("XDG_DATA_HOME", "/x/data"),
            ("XDG_CACHE_HOME", "/x/cache"),
        ]));
        for store in [
            "/x/config/gh",
            "/x/config/git/credentials",
            "/x/data/atuin",
            "/x/cache/huggingface/token",
        ] {
            assert!(moved.contains(&PathBuf::from(store)), "{store}: {moved:?}");
        }
        // Only what is kept under the folder a variable moves, and only where the last says.
        let elsewhere = |path: &PathBuf| !path.starts_with("/x") || path.ends_with(".ssh");
        assert!(!moved.iter().any(elsewhere), "{moved:?}");
    }

    #[test]
    fn the_x_server_s_cookie_is_hidden_where_the_environment_names_a_file() {
        let dir = glave_ports::testing::TempDir::new("x-cookie").unwrap();
        let cookie = dir.join("xauth_abc");
        std::fs::write(&cookie, "").unwrap();
        let named = |place: &Path| environment(&[("XAUTHORITY", place.to_str().unwrap())]);
        glave_ports::block_on!(async {
            assert_eq!(x_cookie(&named(&cookie)).await, Some(cookie.clone()));
            assert_eq!(x_cookie(&named(dir.path())).await, None, "a folder");
            assert_eq!(x_cookie(&named(&dir.join("gone"))).await, None);
            assert_eq!(x_cookie(&named(Path::new("xauth_abc"))).await, None);
        });
    }

    #[test]
    fn nothing_moves_without_a_variable_or_with_one_that_is_no_full_path() {
        let unset = moved_stores(&[]);
        assert!(unset.is_empty(), "{unset:?}");
        let relative = moved_stores(&environment(&[("XDG_CONFIG_HOME", "config")]));
        assert!(relative.is_empty(), "{relative:?}");
    }
}
