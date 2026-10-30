//! A person's own computer (`specs/computers.md`, "A person's computer in the cloud"), reached
//! across the computer protocol: a thread in a project whose folder is there opens with what is
//! read there, through the port (the person's own instructions in their home, then the
//! project's AGENTS.md), never with what is on the server's disk, and reads it there again
//! when it is resumed.

use std::path::Path;
use std::sync::Arc;

use glave_computer::{LocalComputers, RemoteComputers, serve};
use glave_event::{PrincipalId, StopReason};
use glave_harness::{AllowAll, Cancel, Homes, Place, ProjectFolders};
use glave_ports::computer::{ComputerError, Computers, Environment, Home, Sources};
use glave_ports::{BoxFuture, InMemoryLog, Message, OrgId};
use tokio::io::BufReader;

use crate::support::*;

/// What the server's own disk and host say, which a thread on a person's computer never gets.
const SERVERS: &str = "The server's rules.";

/// The server's host, which reads the server's disk.
struct OnTheServer;

impl ProjectFolders for OnTheServer {
    fn for_folder<'a>(
        &'a self,
        _root: &'a Path,
        _project: Option<&'a Path>,
        _trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(std::future::ready(Some(SERVERS.to_owned())))
    }
}

/// Everyone's computer the same one, its home reached across the protocol.
struct Reached(Home);

impl Homes for Reached {
    fn home(&self, _org: OrgId, _person: PrincipalId) -> Result<Home, ComputerError> {
        Ok(self.0.clone())
    }

    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(()))
    }
}

/// The computer at `dir`, served over an in-memory pipe, and its home at `home` opened there
/// by a client of it.
async fn across_the_protocol(dir: &Path, home: &Path) -> Home {
    let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
    let computers = Arc::new(LocalComputers::new(dir.join("copies"), environment));
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server);
    tokio::spawn(serve(
        computers,
        BufReader::new(server_read),
        server_write,
        None,
    ));
    let (client_read, client_write) = tokio::io::split(client);
    let remote = RemoteComputers::connect(client_read, client_write);
    let made = remote.create(&Environment::host()).await.unwrap();
    let path = home.to_string_lossy().into_owned();
    let info = remote
        .open(made.id, &Sources::Folder { path })
        .await
        .unwrap();
    Home {
        computers: Arc::new(remote),
        home: info,
    }
}

/// The text parts of `message`.
fn texts(message: &Message) -> Vec<String> {
    let Message::User { parts } = message else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part {
            glave_event::Part::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_project_on_a_person_s_computer_opens_with_its_agents_md_read_there() {
    let server = folder("homes-server");
    std::fs::write(server.join("AGENTS.md"), SERVERS).unwrap();
    let computer = folder("homes-computer");
    let home = computer.join("home");
    let project = home.join("projects/app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(home.join(".glave")).unwrap();
    std::fs::write(home.join(".glave/AGENTS.md"), "Be brief.\n").unwrap();
    std::fs::write(home.join("projects/AGENTS.md"), "Above the repository.\n").unwrap();
    std::fs::write(project.join("AGENTS.md"), "Use tabs.\n").unwrap();
    block_on!(async {
        let reached = across_the_protocol(&computer, &home).await;
        let answers = vec![
            answer(Some("Hi."), vec![], StopReason::EndTurn),
            answer(Some("Back."), vec![], StopReason::EndTurn),
            answer(
                Some("We said hello, then back."),
                vec![],
                StopReason::EndTurn,
            ),
        ];
        let mut settings = settings();
        settings.instructions = Some(SERVERS.to_owned());
        let (harness, seen) =
            harness_seeing(&server, answers, AllowAll, InMemoryLog::new(), 1, settings).await;
        let harness = harness
            .with_project_folders(Some(Arc::new(OnTheServer)))
            .with_homes(Some(Arc::new(Reached(reached))));
        let place = Place::Folder(project.to_string_lossy().into_owned());
        let mut thread = thread_in(&harness, place).await;
        turn(&harness, &mut thread, "hello").await;

        let first = texts(&seen.lock().unwrap()[0].messages[0]);
        let opening = first.first().unwrap();
        let order = ["Be brief.", "Use tabs."].map(|line| opening.find(line));
        assert!(
            order.iter().all(Option::is_some) && order[0] < order[1],
            "the person's own, then the project's: {opening}"
        );
        assert!(
            !opening.contains("Above the repository."),
            "nothing above the repository's root: {opening}"
        );
        assert!(
            !first.iter().any(|text| text.contains(SERVERS)),
            "nothing of the server's: {first:?}"
        );
        assert_eq!(first.last().map(String::as_str), Some("hello"));

        // Resumed, its next turn opens the folder there again and reads them again, which a
        // compaction opens the conversation with.
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut resumed, "back").await;
        harness
            .compact(&mut resumed, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let again = texts(&resumed.messages()[0]);
        assert_eq!(again.first(), Some(opening), "{again:?}");
    });
}
