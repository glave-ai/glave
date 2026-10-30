//! A person's own computer (`specs/computers.md`, "A person's computer in the cloud"), reached
//! across the computer protocol: a thread in a project whose folder is there opens with what is
//! read there, through the port (the person's own instructions in their home, then the
//! project's AGENTS.md), never with what the server's host reads. Resumed, it keeps what it
//! opened with, as its log holds it, until its next turn reads them there again.

use std::path::Path;
use std::sync::Arc;

use glave_computer::LocalComputers;
use glave_event::{PrincipalId, StopReason};
use glave_harness::{AllowAll, Cancel, Homes, Place, ProjectFolders};
use glave_ports::computer::{ComputerError, Home};
use glave_ports::{BoxFuture, InMemoryLog, OrgId};

use crate::support::*;
use crate::wire::home_across_the_wire;

/// What the server's own settings and host say, which a thread on a person's computer never
/// gets.
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

#[test]
fn a_project_on_a_person_s_computer_opens_with_its_agents_md_read_there() {
    let server = folder("homes-server");
    let computer = folder("homes-computer");
    let home = computer.join("home");
    let project = home.join("projects/app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(home.join(".glave")).unwrap();
    std::fs::write(home.join(".glave/AGENTS.md"), "Be brief.\n").unwrap();
    std::fs::write(home.join("projects/AGENTS.md"), "Above the repository.\n").unwrap();
    std::fs::write(project.join("AGENTS.md"), "Use tabs.\n").unwrap();
    block_on!(async {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        let computers = Arc::new(LocalComputers::new(computer.join("copies"), environment));
        let (reached, serving) = home_across_the_wire(computers, &home).await;
        let answers = vec![
            answer(Some("Hi."), vec![], StopReason::EndTurn),
            answer(Some("We said hello."), vec![], StopReason::EndTurn),
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
        let opening = first.first().unwrap().clone();
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

        // Resumed and compacted before any turn, so before its computer is reached again, it
        // opens with what its log says it opened with.
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        harness
            .compact(&mut resumed, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let again = texts(&resumed.messages()[0]);
        assert_eq!(again.first(), Some(&opening), "{again:?}");
        // Its next turn reads them there again.
        std::fs::write(project.join("AGENTS.md"), "Use spaces.\n").unwrap();
        turn(&harness, &mut resumed, "back").await;
        harness
            .compact(&mut resumed, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let read = texts(&resumed.messages()[0]);
        assert!(read[0].contains("Use spaces."), "{read:?}");
        serving.abort();
    });
}
