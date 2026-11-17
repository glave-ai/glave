//! Every credential store the sandbox names, under the real sandbox: a command reads none of
//! them, whether a store is a file or a folder, in the home folder or where the environment
//! moves it, and a command outside reads them all.

use super::super::secrets::home_secrets;
use super::*;

/// What a command of `computers` prints of `files`, each given a canary: outside the sandbox,
/// then inside `sandbox`.
fn read(
    dir: &Path,
    (computers, sandbox): &(LocalComputers, Sandbox),
    files: &[PathBuf],
) -> (String, String) {
    let mut read = String::new();
    for (index, file) in files.iter().enumerate() {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, format!("canary-{index}\n")).unwrap();
        let _ = writeln!(read, "cat '{}' 2>/dev/null", file.display());
    }
    block_on!(async {
        let workspace = open_app(computers, dir).await;
        let outside = Collect::default();
        computers
            .run(
                workspace,
                &Command::new(read.clone()),
                &outside,
                &CancelSwitch::default(),
            )
            .await
            .unwrap();
        let outside = outside.0.lock().unwrap().clone();
        let (_, inside) = run(computers, workspace, sandbox, &read).await;
        (outside, inside)
    })
}

/// The whole of this platform's list, the common one and its own, each store as the file it
/// is or as a file of the folder it is.
#[test]
fn every_credential_store_in_the_home_folder_is_hidden_from_a_sandboxed_command() {
    if !enforced() {
        return;
    }
    let dir = scratch("stores");
    let made = computers(&dir);
    let home = made.1.home.clone().unwrap();
    let stores: Vec<PathBuf> = home_secrets()
        .enumerate()
        .map(|(index, store)| match index % 2 {
            0 => home.join(store),
            _ => home.join(store).join("kept"),
        })
        .collect();
    let (outside, inside) = read(&dir, &made, &stores);
    assert_eq!(
        outside.matches("canary-").count(),
        stores.len(),
        "each is there to read outside the sandbox: {outside}"
    );
    assert!(
        !inside.contains("canary-"),
        "a sandboxed command read a credential store: {inside}"
    );
}

/// The stores a review found readable (#1508), by where their tools keep them, whatever the
/// list says: git-credential-store's other file and git-credential-cache's socket's folder
/// first.
#[test]
fn stores_once_missed_are_hidden_from_a_sandboxed_command() {
    if !enforced() {
        return;
    }
    let dir = scratch("missed");
    let made = computers(&dir);
    let home = made.1.home.clone().unwrap();
    let stores = [
        ".config/git/credentials",
        ".cache/git/credential/socket",
        ".config/pypoetry/auth.toml",
        ".config/tea/config.yml",
        ".config/doctl/config.yaml",
        ".config/stripe/config.toml",
        ".my.cnf",
        ".claude.json",
        ".claude.json.backup",
    ]
    .map(|store| home.join(store));
    let (outside, inside) = read(&dir, &made, &stores);
    assert_eq!(
        outside.matches("canary-").count(),
        stores.len(),
        "{outside}"
    );
    assert!(
        !inside.contains("canary-"),
        "a sandboxed command read a credential store: {inside}"
    );
}

/// A store the environment moves is hidden where it is: one kept under `~/.config`,
/// `~/.local/share` or `~/.cache` is under `XDG_CONFIG_HOME`, `XDG_DATA_HOME` or
/// `XDG_CACHE_HOME` where the person set it. What else is kept there stays readable.
#[test]
fn a_store_the_environment_moves_is_hidden_from_a_sandboxed_command_where_it_is() {
    if !enforced() {
        return;
    }
    let dir = scratch("moved");
    let (config, data, cache) = (
        dir.join("xdg/config"),
        dir.join("xdg/data"),
        dir.join("xdg/cache"),
    );
    let made = computers_saying(
        &dir,
        &[
            ("XDG_CONFIG_HOME", config.to_str().unwrap()),
            ("XDG_DATA_HOME", data.to_str().unwrap()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
    );
    let stores = [
        config.join("gh/hosts.yml"),
        config.join("git/credentials"),
        data.join("atuin/history.db"),
        cache.join("huggingface/token"),
    ];
    let (outside, inside) = read(&dir, &made, &stores);
    assert_eq!(
        outside.matches("canary-").count(),
        stores.len(),
        "{outside}"
    );
    assert!(
        !inside.contains("canary-"),
        "a sandboxed command read a store where the environment keeps it: {inside}"
    );
    // Not a store: a tool's settings beside them.
    let settings = [config.join("app/settings.json")];
    let (_, inside) = read(&dir, &made, &settings);
    assert!(inside.contains("canary-0"), "{inside}");
}
