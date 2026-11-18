//! A computer's token in glave's folder, where `glave computer serve --tls` keeps it with its
//! key (`computer-tls`), under the real sandbox: a command that read it, or put its own in its
//! place, could connect and open a terminal outside every sandbox (#1502). It does neither, nor
//! read the key; the same commands outside the sandbox do, so the test cannot pass by being
//! broken.

use super::*;

/// The folder in glave's own that `glave computer serve --tls` serves from.
const TLS: &str = "computer-tls";

/// `computers` as `glave computer serve` makes them, hiding the folder of its token too.
pub(in crate::local::sandbox) fn serving(
    computers: LocalComputers,
    glave_home: &Path,
) -> LocalComputers {
    computers.with_hidden([glave_home.join(TLS)])
}

/// What a command runs to take the token from `tls`, in glave's folder `home`: reads it and the
/// key, then replaces the token by writing it, by moving it aside, by moving its folder aside
/// for another, and by moving glave's folder aside.
fn takes(tls: &Path, home: &Path) -> String {
    format!(
        "cat '{tls}/token' '{tls}/key.pem'; printf forged > '{tls}/token'; \
         mv '{tls}/token' '{tls}/old'; printf forged > '{tls}/token'; \
         mkdir -p aside && mv '{tls}' aside/tls; mkdir -p '{tls}' && printf forged > '{tls}/token'; \
         mv '{home}' aside/home; mkdir -p '{tls}' && printf forged > '{tls}/token'; echo tried",
        tls = tls.display(),
        home = home.display(),
    )
}

/// In `workspace`, under `sandbox`, a command tries to take the token and key kept in glave's
/// folder as [`serving`] keeps them: it reads neither, and both stay as they were. Outside the
/// sandbox the same reads see both, and the first way to replace the token works.
pub(in crate::local::sandbox) async fn kept_from_a_command(
    computers: &LocalComputers,
    workspace: WorkspaceId,
    sandbox: &Sandbox,
) {
    let home = sandbox.glave_home.clone().unwrap();
    let tls = home.join(TLS);
    std::fs::create_dir_all(&tls).unwrap();
    std::fs::write(tls.join("token"), "canary-token\n").unwrap();
    std::fs::write(tls.join("key.pem"), "canary-key\n").unwrap();
    let (ending, said) = run(computers, workspace, sandbox, &takes(&tls, &home)).await;
    assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
    assert!(said.contains("tried"), "{said}");
    assert!(!said.contains("canary"), "{said}");
    let kept = |name: &str| std::fs::read_to_string(tls.join(name)).unwrap_or_default();
    assert_eq!(kept("token"), "canary-token\n", "{said}");
    assert_eq!(kept("key.pem"), "canary-key\n", "{said}");
    // Outside, the reads and a write, leaving glave's folder in place.
    let outside = Command::new(format!(
        "cat '{0}/token' '{0}/key.pem'; printf forged > '{0}/token'",
        tls.display()
    ));
    let output = Collect::default();
    let finished = computers
        .run(workspace, &outside, &output, &CancelSwitch::default())
        .await
        .unwrap();
    let said = output.0.lock().unwrap().clone();
    assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{said}");
    assert!(
        said.contains("canary-token") && said.contains("canary-key"),
        "{said}"
    );
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
