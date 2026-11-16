//! Every credential store the sandbox names in the home folder, under the real sandbox: a
//! command reads none of them, whether a store is a file or a folder, and a command outside
//! reads them all.

use super::super::secrets::HOME_SECRETS;
use super::*;

/// What a command prints of `stores` in a home folder, each holding a canary: outside the
/// sandbox, then inside it. `None` where no sandbox is enforced.
fn read(name: &str, stores: &[PathBuf]) -> Option<(String, String)> {
    if !enforced() {
        return None;
    }
    let dir = scratch(name);
    let (computers, sandbox) = computers(&dir);
    let home = sandbox.home.clone().unwrap();
    let mut read = String::new();
    for (index, store) in stores.iter().enumerate() {
        let file = home.join(store);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("canary-{index}\n")).unwrap();
        let _ = writeln!(read, "cat '{}' 2>/dev/null", file.display());
    }
    Some(block_on!(async {
        let workspace = open_app(&computers, &dir).await;
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
        let (_, inside) = run(&computers, workspace, &sandbox, &read).await;
        (outside, inside)
    }))
}

#[test]
fn every_credential_store_in_the_home_folder_is_hidden_from_a_sandboxed_command() {
    // Each store as the file it is, or as a file of the folder it is.
    let stores: Vec<PathBuf> = HOME_SECRETS
        .iter()
        .enumerate()
        .map(|(index, store)| match index % 2 {
            0 => PathBuf::from(store),
            _ => Path::new(store).join("kept"),
        })
        .collect();
    let Some((outside, inside)) = read("stores", &stores) else {
        return;
    };
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
    let stores = [
        ".config/git/credentials",
        ".cache/git/credential/socket",
        ".config/pip/pip.conf",
        ".config/pypoetry/auth.toml",
        ".config/tea/config.yml",
        ".config/doctl/config.yaml",
        ".config/stripe/config.toml",
        ".my.cnf",
        ".claude.json",
    ]
    .map(PathBuf::from);
    let Some((outside, inside)) = read("missed", &stores) else {
        return;
    };
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
