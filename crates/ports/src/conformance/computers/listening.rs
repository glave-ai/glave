//! Commands in the background that listen: where each running one's processes listen, a
//! child's among them, in the sandbox with the network as outside it; one that listens nowhere
//! is not found.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use super::helpers::failed;
use super::{Verdict, ensure};
use crate::computer::{
    Command, ComputerError, Computers, Listening, Sandbox, SandboxLevel, SandboxNetwork,
    WorkspaceId,
};

/// How long the suite gives a command to say where it listens.
const SOON: Duration = Duration::from_secs(20);
/// How long each read of what it printed waits.
const POLL: Duration = Duration::from_millis(100);

/// A command whose child listens on loopback at a port the system picks, and says which.
const LISTENS: &str = "python3 -c \"import socket, time; s = socket.socket(); \
     s.bind(('127.0.0.1', 0)); s.listen(); print('port', s.getsockname()[1], flush=True); \
     time.sleep(300)\" & wait";

/// Runs this part of the suite in `here`: outside the sandbox, then in it with the network,
/// where the computer can run one; none where it cannot tell where its commands listen, which it
/// says.
pub(super) async fn listening<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    match computers.listening(here).await {
        Err(ComputerError::Unsupported(_)) => return Ok(()),
        asked => ensure(
            asked.is_ok_and(|found| found.is_empty()),
            "a workspace with no command in the background is found listening nowhere",
        )?,
    }
    listed_with_its_port(computers, here, None).await?;
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::Direct,
        home: None,
        glave_home: None,
    };
    listed_with_its_port(computers, here, Some(sandbox)).await
}

async fn listed_with_its_port<C: Computers>(
    computers: &C,
    here: WorkspaceId,
    sandbox: Option<Sandbox>,
) -> Verdict {
    let mut command = Command::new(LISTENS);
    command.sandbox = sandbox;
    let listens = match computers.start(here, &command).await {
        Err(ComputerError::Unsupported(_)) if command.sandbox.is_some() => return Ok(()),
        started => started.map_err(failed("start a command that listens"))?,
    };
    let quiet = computers
        .start(here, &Command::new("sleep 30"))
        .await
        .map_err(failed("start a command that listens on nothing"))?;
    let listed = async {
        // It says its port once it listens, so where it listens is found from then on.
        let port = said_port(computers, here, listens).await?;
        let found = computers
            .listening(here)
            .await
            .map_err(failed("look where the commands in the background listen"))?;
        let at = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        ensure(
            found
                == [Listening {
                    command: listens,
                    at: vec![at],
                }],
            &format!(
                "a command whose child listens at {at} is found listening there alone, and one \
                 that listens nowhere is not found, not {found:?}"
            ),
        )
    }
    .await;
    for id in [listens, quiet] {
        computers
            .stop(here, id)
            .await
            .map_err(failed("stop a command in the background"))?;
    }
    listed
}

/// The port the command that listens says it listens on, once it has.
async fn said_port<C: Computers>(
    computers: &C,
    here: WorkspaceId,
    command: crate::computer::CommandId,
) -> Result<u16, String> {
    let mut seen = String::new();
    let began = Instant::now();
    while began.elapsed() < SOON {
        let printed = computers
            .output(here, command, POLL)
            .await
            .map_err(failed("read a command that listens"))?;
        seen.push_str(&printed.text);
        if let Some(port) = seen
            .lines()
            .find_map(|line| line.strip_prefix("port "))
            .and_then(|port| port.trim().parse().ok())
        {
            return Ok(port);
        }
        if printed.ended.is_some() {
            return Err(format!("it ended before it listened: {seen}"));
        }
    }
    Err(format!("it did not say where it listens soon: {seen}"))
}
