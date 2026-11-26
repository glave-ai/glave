//! A computer's token in a folder of glave's own, hidden as `glave computer serve` hides the
//! folder of its token, under the real sandbox: a command that read it, or put its own in its
//! place, could connect and open a terminal outside every sandbox (#1502). It does neither; the
//! same commands outside the sandbox do, so the test cannot pass by being broken.

use super::*;

/// The folder in glave's own that holds the token.
const FOLDER: &str = "computer-token";

/// `computers` as `glave computer serve` makes them, hiding the folder of its token too.
pub(in crate::local::sandbox) fn serving(
    computers: LocalComputers,
    glave_home: &Path,
) -> LocalComputers {
    computers.with_hidden([glave_home.join(FOLDER)])
}

/// What a command runs to take the token from `folder`, in glave's folder `home`: reads it,
/// then replaces it by writing it, by moving it aside, by moving its folder aside for another,
/// and by moving glave's folder aside.
fn takes(folder: &Path, home: &Path) -> String {
    format!(
        "cat '{folder}/token'; printf forged > '{folder}/token'; \
         mv '{folder}/token' '{folder}/old'; printf forged > '{folder}/token'; \
         mkdir -p aside && mv '{folder}' aside/folder; mkdir -p '{folder}' && printf forged > '{folder}/token'; \
         mv '{home}' aside/home; mkdir -p '{folder}' && printf forged > '{folder}/token'; echo tried",
        folder = folder.display(),
        home = home.display(),
    )
}

/// In `workspace`, under `sandbox`, a command tries to take the token kept in glave's folder as
/// [`serving`] keeps it: it does not read it, and it stays as it was. Outside the sandbox the
/// same read sees it, and the first way to replace it works.
pub(in crate::local::sandbox) async fn kept_from_a_command(
    computers: &LocalComputers,
    workspace: WorkspaceId,
    sandbox: &Sandbox,
) {
    let home = sandbox.glave_home.clone().unwrap();
    let folder = home.join(FOLDER);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("token"), "canary-token\n").unwrap();
    let (ending, said) = run(computers, workspace, sandbox, &takes(&folder, &home)).await;
    assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
    assert!(said.contains("tried"), "{said}");
    assert!(!said.contains("canary"), "{said}");
    let kept = |name: &str| std::fs::read_to_string(folder.join(name)).unwrap_or_default();
    assert_eq!(kept("token"), "canary-token\n", "{said}");
    // Outside, the reads and a write, leaving glave's folder in place.
    let outside = Command::new(format!(
        "cat '{0}/token'; printf forged > '{0}/token'",
        folder.display()
    ));
    let output = Collect::default();
    let finished = computers
        .run(workspace, &outside, &output, &CancelSwitch::default())
        .await
        .unwrap();
    let said = output.0.lock().unwrap().clone();
    assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{said}");
    assert!(said.contains("canary-token"), "{said}");
    assert_eq!(kept("token"), "forged");
}

/// glave's folder inside the project, where the command writes everything else, at the
/// project level with the network, as a person's commands run.
#[test]
fn a_computer_s_token_in_glave_s_folder_is_neither_read_nor_replaced() {
    if !enforced() {
        return;
    }
    let dir = scratch("token");
    let glave_home = dir.join("app/glave");
    let (computers, mut sandbox) = computers(&dir);
    let computers = serving(computers, &glave_home);
    sandbox.network = SandboxNetwork::Direct;
    sandbox.glave_home = Some(glave_home);
    block_on!(async {
        let here = open_app(&computers, &dir).await;
        kept_from_a_command(&computers, here, &sandbox).await;
    });
}
