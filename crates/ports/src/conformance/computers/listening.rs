//! Commands in the background that listen: each running one is listed with the TCP ports its
//! processes listen on, a child's among them, in the sandbox with the network as outside it,
//! and one that listens on nothing with none.

use std::time::{Duration, Instant};

use super::helpers::failed;
use super::{Verdict, ensure};
use crate::computer::{Command, ComputerError, Computers, Sandbox, SandboxLevel, WorkspaceId};

/// How long the suite gives a command to listen and the listing to say so.
const SOON: Duration = Duration::from_secs(20);
/// How long each wait between two listings is.
const POLL: Duration = Duration::from_millis(100);

/// A command whose child listens on loopback at a port the system picks, and says which.
const LISTENS: &str = "python3 -c \"import socket, time; s = socket.socket(); \
     s.bind(('127.0.0.1', 0)); s.listen(); print('port', s.getsockname()[1], flush=True); \
     time.sleep(300)\" & wait";

/// Runs this part of the suite in `here`: outside the sandbox, then in it with the network,
/// where the computer can run one.
pub(super) async fn listening<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    listed_with_its_port(computers, here, None).await?;
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: true,
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
        let port = said_port(computers, here, listens).await?;
        let began = Instant::now();
        loop {
            let listed = computers
                .commands(here)
                .await
                .map_err(failed("list the commands in the background"))?;
            let ports = |id| {
                listed
                    .iter()
                    .find(|command| command.id == id)
                    .map(|command| &command.ports)
            };
            if ports(listens).is_some_and(|ports| ports.contains(&port)) {
                return ensure(
                    ports(quiet).is_some_and(Vec::is_empty),
                    "a command in the background that listens on nothing is listed with no port",
                );
            }
            if began.elapsed() > SOON {
                return Err(format!(
                    "a command in the background whose child listens on {port} is listed with it, \
                     not {:?}",
                    ports(listens)
                ));
            }
            // A short wait between listings, which ends early should the command end.
            computers
                .ended(here, listens, POLL)
                .await
                .map_err(failed("wait on a command that listens"))?;
        }
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
