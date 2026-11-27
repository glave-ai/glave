//! What the crate's own tests share.

use glave_ports::NeverCancelled;
use glave_ports::computer::{Command, Computers, Ending, WorkspaceId};
use glave_ports::testing::Captured;

/// What `script` printed, run in `workspace` on `computers` with no sandbox of its own; the test
/// fails unless it exits 0.
pub(crate) async fn ran<C: Computers + ?Sized>(
    computers: &C,
    workspace: WorkspaceId,
    script: &str,
) -> String {
    let output = Captured::default();
    let finished = computers
        .run(workspace, &Command::new(script), &output, &NeverCancelled)
        .await
        .unwrap();
    let said = output.text();
    assert_eq!(
        finished.ending,
        Ending::Exited { code: 0 },
        "{script}: {said}"
    );
    said
}
