//! Commands kept running in the background: started, read, listed and stopped; ended by
//! themselves and read to the end; their end told when another command kills them, whatever
//! they left still holding their output; stopped with everything they started; bounded in number and
//! in the output kept; moved there at a timeout; confined by their sandbox; reached only through
//! their own workspace, and stopped when it closes.

use std::time::{Duration, Instant};

use super::helpers::{failed, run, run_quietly};
use super::{Verdict, ensure};
use crate::computer::{
    Command, CommandId, ComputerError, ComputerId, Computers, Ending, KEPT_OUTPUT, MAX_BACKGROUND,
    OnTimeout, Sandbox, SandboxLevel, Sources, WorkspaceId,
};

/// How long the suite gives a command in the background to do what it must do soon.
const SOON: Duration = Duration::from_secs(20);
/// How long each read waits while the suite looks for output.
const POLL: Duration = Duration::from_millis(100);

/// Runs this part of the suite in `here`, a folder at `scratch` on `computer`, in a folder of
/// its own there, removed at the end.
pub(super) async fn background<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    scratch: &str,
) -> Verdict {
    run_quietly(computers, here, "mkdir -p background/elsewhere").await?;
    read_and_stop(computers, here).await?;
    ends_by_itself(computers, here).await?;
    killed_from_outside(computers, here).await?;
    whole_group(computers, here).await?;
    bounded(computers, here).await?;
    kept_output(computers, here).await?;
    moved_at_timeout(computers, here).await?;
    sandboxed(computers, here).await?;
    closed_with_workspace(computers, computer, here, scratch).await?;
    run_quietly(computers, here, "rm -rf background").await
}

async fn start<C: Computers>(
    computers: &C,
    workspace: WorkspaceId,
    script: &str,
) -> Result<CommandId, String> {
    computers
        .start(workspace, &Command::new(script))
        .await
        .map_err(|error| format!("start {script:?} in the background: {error}"))
}

/// What `command` printed until it printed `marker`, read a little at a time; an error when it
/// ends first or does not print it soon.
async fn until_printed<C: Computers>(
    computers: &C,
    workspace: WorkspaceId,
    command: CommandId,
    marker: &str,
) -> Result<String, String> {
    let mut seen = String::new();
    let began = Instant::now();
    while began.elapsed() < SOON {
        let printed = computers
            .output(workspace, command, POLL)
            .await
            .map_err(failed("read a command in the background"))?;
        seen.push_str(&printed.text);
        if seen.contains(marker) {
            return Ok(seen);
        }
        if printed.ended.is_some() {
            return Err(format!("it ended before it printed {marker:?}: {seen}"));
        }
    }
    Err(format!("it did not print {marker:?} soon: {seen}"))
}

/// Whether the process whose id is in `file` is gone.
async fn gone<C: Computers>(computers: &C, here: WorkspaceId, file: &str) -> Result<bool, String> {
    let (finished, _) = run(
        computers,
        here,
        &Command::new(format!("kill -0 \"$(cat {file})\"")),
    )
    .await?;
    Ok(finished.ending != Ending::Exited { code: 0 })
}

async fn read_and_stop<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let script = "printf 'one\\n'; sleep 30";
    let id = start(computers, here, script).await?;
    until_printed(computers, here, id, "one\n").await?;
    let listed = computers
        .commands(here)
        .await
        .map_err(failed("list the commands in the background"))?;
    ensure(
        listed
            .iter()
            .any(|command| command.id == id && command.script == script && command.ended.is_none()),
        "a listing shows a command in the background with its script, running",
    )?;
    let stopped = computers
        .stop(here, id)
        .await
        .map_err(failed("stop a command in the background"))?;
    ensure(
        stopped
            .ended
            .is_some_and(|ended| ended.ending == Ending::Cancelled),
        "a command stopped in the background ends as cancelled",
    )?;
    ensure(
        matches!(
            computers.output(here, id, Duration::ZERO).await,
            Err(ComputerError::NotFound(_))
        ),
        "a stopped command is forgotten",
    )?;
    let listed = computers
        .commands(here)
        .await
        .map_err(failed("list the commands in the background"))?;
    ensure(
        !listed.iter().any(|command| command.id == id),
        "a stopped command is no longer listed",
    )
}

async fn ends_by_itself<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let id = start(computers, here, "printf 'done\\n'; exit 4").await?;
    let began = Instant::now();
    let printed = computers
        .output(here, id, SOON)
        .await
        .map_err(failed("wait for a command in the background to end"))?;
    ensure(
        began.elapsed() < SOON,
        "a read waiting for a command answers as soon as it ends",
    )?;
    ensure(
        printed.text.contains("done\n")
            && printed
                .ended
                .is_some_and(|ended| ended.ending == Ending::Exited { code: 4 }),
        "a command that ended in the background answers its last output and its exit code",
    )?;
    ensure(
        matches!(
            computers.output(here, id, Duration::ZERO).await,
            Err(ComputerError::NotFound(_))
        ),
        "the read that answers a command's ending is its last",
    )
}

async fn killed_from_outside<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    // The shell is the command's own process, as npm is a dev server's; the child it starts
    // keeps the output open after the shell is killed.
    let script = "sleep 300 & echo $! > background/served.pid; echo $$ > background/shell.pid; \
                  echo ready; wait";
    let id = start(computers, here, script).await?;
    until_printed(computers, here, id, "ready").await?;
    let running = computers
        .ended(here, id, POLL)
        .await
        .map_err(failed("ask how a running command ended"))?;
    ensure(running.is_none(), "a command still running has not ended")?;
    run_quietly(computers, here, "kill \"$(cat background/shell.pid)\"").await?;
    let began = Instant::now();
    let ended = computers
        .ended(here, id, SOON)
        .await
        .map_err(failed("wait for a killed command to end"))?;
    ensure(
        began.elapsed() < SOON
            && ended.is_some_and(|ended| !matches!(ended.ending, Ending::Cancelled)),
        "a command killed by another command is told ended, not stopped",
    )?;
    ensure(
        gone(computers, here, "background/served.pid").await?,
        "what a killed command left running is stopped with it",
    )?;
    let printed = computers
        .output(here, id, Duration::ZERO)
        .await
        .map_err(failed("read a command after its end was told"))?;
    ensure(
        printed.ended.is_some() && printed.ended == ended,
        "telling a command's end is not a read: the next read answers the same ending",
    )
}

async fn whole_group<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let script = "sleep 300 & echo $! > background/child.pid; echo ready; wait";
    let id = start(computers, here, script).await?;
    until_printed(computers, here, id, "ready").await?;
    computers
        .stop(here, id)
        .await
        .map_err(failed("stop a command in the background"))?;
    ensure(
        gone(computers, here, "background/child.pid").await?,
        "stopping a command in the background stops everything it started",
    )
}

async fn bounded<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let mut running = Vec::with_capacity(MAX_BACKGROUND);
    for _ in 0..MAX_BACKGROUND {
        running.push(start(computers, here, "sleep 30").await?);
    }
    let refused = computers.start(here, &Command::new("sleep 30")).await;
    for id in running {
        computers
            .stop(here, id)
            .await
            .map_err(failed("stop a command in the background"))?;
    }
    ensure(
        matches!(refused, Err(ComputerError::Refused(_))),
        "a workspace refuses one command in the background past its limit",
    )
}

async fn kept_output<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let printed = KEPT_OUTPUT.saturating_add(300_000);
    let script = format!(
        "head -c {printed} /dev/zero | tr '\\0' a; printf '\\nend\\n'; \
         touch background/printed; sleep 30"
    );
    let id = start(computers, here, &script).await?;
    let mut wait = Command::new("while [ ! -e background/printed ]; do sleep 0.1; done");
    wait.timeout_secs = Some(SOON.as_secs());
    run(computers, here, &wait).await?;
    let read = computers
        .output(here, id, Duration::ZERO)
        .await
        .map_err(failed("read a command that printed a lot"))?;
    computers
        .stop(here, id)
        .await
        .map_err(failed("stop a command in the background"))?;
    let dropped = usize::try_from(read.dropped).unwrap_or(usize::MAX);
    ensure(
        read.text.len() <= KEPT_OUTPUT
            && read.text.ends_with("end\n")
            && dropped.saturating_add(read.text.len()) == printed.saturating_add(5),
        "a command in the background keeps its most recent output, and says how much it dropped",
    )
}

async fn moved_at_timeout<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let mut slow = Command::new("echo started; sleep 30");
    slow.timeout_secs = Some(1);
    slow.on_timeout = OnTimeout::Background;
    let (finished, output) = run(computers, here, &slow).await?;
    let Ending::Backgrounded {
        command,
        after_secs: 1,
    } = finished.ending
    else {
        return Err(format!(
            "a command past its timeout must move to the background when asked, not {:?}",
            finished.ending
        ));
    };
    ensure(
        output.contains("started"),
        "what a command printed before it moved to the background goes to its run",
    )?;
    let listed = computers
        .commands(here)
        .await
        .map_err(failed("list the commands in the background"))?;
    ensure(
        listed
            .iter()
            .any(|listed| listed.id == command && listed.ended.is_none()),
        "a command moved to the background is listed there, running",
    )?;
    let stopped = computers
        .stop(here, command)
        .await
        .map_err(failed("stop a command moved to the background"))?;
    ensure(
        stopped.ended.is_some(),
        "a command moved to the background can be stopped there",
    )
}

async fn sandboxed<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let mut command = Command::new("echo inside > background/sandboxed.txt");
    command.sandbox = Some(Sandbox {
        level: SandboxLevel::ReadOnly,
        network: false,
        home: None,
        glave_home: None,
    });
    let id = match computers.start(here, &command).await {
        Err(ComputerError::Unsupported(_)) => return Ok(()),
        Err(error) => return Err(format!("start a sandboxed command: {error}")),
        Ok(id) => id,
    };
    let printed = computers
        .output(here, id, SOON)
        .await
        .map_err(failed("wait for a sandboxed command"))?;
    ensure(
        printed.ended.is_some()
            && matches!(
                computers.read(here, "background/sandboxed.txt").await,
                Err(ComputerError::NotFound(_))
            ),
        "a read-only sandbox must stop a command in the background writing to the workspace",
    )
}

async fn closed_with_workspace<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    scratch: &str,
) -> Verdict {
    let there = computers
        .open(
            computer,
            &Sources::Folder {
                path: format!("{scratch}/background/elsewhere"),
            },
        )
        .await
        .map_err(failed("open a second folder"))?;
    let script = "sleep 300 & echo $! > child.pid; echo ready; wait";
    let id = start(computers, there.id, script).await?;
    until_printed(computers, there.id, id, "ready").await?;
    ensure(
        matches!(
            computers.output(here, id, Duration::ZERO).await,
            Err(ComputerError::NotFound(_))
        ),
        "a command in the background is reached only through its own workspace",
    )?;
    computers
        .close(there.id)
        .await
        .map_err(failed("close a workspace with a command in the background"))?;
    ensure(
        gone(computers, here, "background/elsewhere/child.pid").await?,
        "closing a workspace stops its commands in the background, and what they started",
    )
}
