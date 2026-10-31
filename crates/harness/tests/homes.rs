//! A person's own computer (`specs/computers.md`, "A person's computer in the cloud"), reached
//! across the computer protocol: a thread in a project whose folder is there opens with what was
//! read there, through the port (the person's own instructions in their home, then the
//! project's AGENTS.md), never with what the server's host reads, and kept. Starting it and a
//! turn that only talks never reach the computer (decision 6); a turn that runs a command there
//! reads it again as it ends, for the next thread to open with. Resumed, a thread keeps what it
//! opened with, as its log holds it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use glave_computer::{LocalComputers, Wake, Waking};
use glave_event::{HookPoint, Permissions, PrincipalId, StopReason};
use glave_harness::{AllowAll, Cancel, Decision, Homes, Place, ProjectFolders, Workspace};
use glave_ports::computer::{ComputerError, Home, Sandbox, SandboxLevel};
use glave_ports::settings::{Hook, Hooks};
use glave_ports::stream::Reserved;
use glave_ports::{BoxFuture, Caller, EventLog, InMemoryLog, OrgId};
use serde_json::json;

use crate::outside_sandbox::{Person, sandboxed};
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

    fn sandbox(&self) -> Option<Sandbox> {
        None
    }
}

/// Everyone's computer the same one reached across the protocol, behind a [`Waking`] that wakes
/// it only for the calls that need it, as a server just started holds it: each time a call asks
/// for it is counted.
struct Sleepy {
    planned: Home,
    asked: Arc<AtomicUsize>,
    sandbox: Option<Sandbox>,
}

impl Sleepy {
    /// `reached` the computer each call that needs it finds.
    fn new(reached: &Home, scratch: &Path) -> Self {
        Self::finding(reached, scratch, Ok(reached.clone()))
    }

    /// A computer that cannot be reached, as when it fails to wake.
    fn unreachable(reached: &Home, scratch: &Path) -> Self {
        let failed = ComputerError::Unreachable("it could not wake".into());
        Self::finding(reached, scratch, Err(failed))
    }

    /// `reached` planned, and what each call that needs it finds.
    fn finding(reached: &Home, scratch: &Path, found: Result<Home, ComputerError>) -> Self {
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        let wake: Wake = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(found.clone()))
        });
        let scratch = scratch.to_string_lossy();
        let planned = Waking::home(wake, None, reached.home.clone(), &scratch);
        Self {
            planned,
            asked,
            sandbox: None,
        }
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// `reached`, its home at `home`, its commands in glave's sandbox.
    fn in_sandbox(reached: &Home, scratch: &Path, home: &Path) -> Self {
        let mut sleepy = Self::new(reached, scratch);
        sleepy.sandbox = Some(Sandbox {
            level: SandboxLevel::Project,
            network: false,
            home: Some(home.to_owned()),
            glave_home: Some(home.join(".glave")),
        });
        sleepy
    }
}

impl Homes for Sleepy {
    fn home(&self, _org: OrgId, _person: PrincipalId) -> Result<Home, ComputerError> {
        Ok(self.planned.clone())
    }

    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(()))
    }

    fn sandbox(&self) -> Option<Sandbox> {
        self.sandbox.clone()
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
        ];
        let mut settings = settings();
        settings.instructions = Some(SERVERS.to_owned());
        let (harness, seen) =
            harness_seeing(&server, answers, AllowAll, InMemoryLog::new(), 1, settings).await;
        let harness = harness
            .with_project_folders(Some(Arc::new(OnTheServer)))
            .with_homes(Some(Arc::new(Reached(reached.clone()))));
        // Read there as the project's folder is made, and kept.
        let path = project.to_string_lossy().into_owned();
        let at_home = Workspace::new(reached.computers, reached.home);
        harness.keep_instructions(scope(), &at_home, &path).await;
        let mut thread = thread_in(&harness, Place::Folder(path)).await;
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
        serving.abort();
    });
}

#[test]
fn a_project_s_thread_opens_with_its_last_read_and_reaches_the_computer_only_for_a_command() {
    let server = folder("asleep-server");
    let computer = folder("asleep-computer");
    let home = computer.join("home");
    let project = home.join("projects/app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(project.join("AGENTS.md"), "Use tabs.\n").unwrap();
    block_on!(async {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        let computers = Arc::new(LocalComputers::new(computer.join("copies"), environment));
        let (reached, serving) = home_across_the_wire(computers, &home).await;
        let change = r"printf 'Use spaces.\n' > AGENTS.md";
        let answers = vec![
            answer(Some("Hi."), vec![], StopReason::EndTurn),
            answer(
                None,
                vec![("bash", json!({ "command": change }))],
                StopReason::ToolUse,
            ),
            answer(Some("Changed."), vec![], StopReason::EndTurn),
            answer(Some("Hey."), vec![], StopReason::EndTurn),
            answer(Some("Back."), vec![], StopReason::EndTurn),
            answer(
                None,
                vec![("bash", json!({ "command": "true" }))],
                StopReason::ToolUse,
            ),
            answer(Some("Nothing changed."), vec![], StopReason::EndTurn),
            answer(
                None,
                vec![("bash", json!({ "command": change }))],
                StopReason::ToolUse,
            ),
            answer(Some("It could not."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let scratch = computer.join("scratch");
        let first = Arc::new(Sleepy::new(&reached, &scratch));
        let harness = harness.with_homes(Some(Arc::clone(&first) as Arc<dyn Homes>));
        let place = || Place::Folder(project.to_string_lossy().into_owned());
        let opening = |request: usize| texts(&seen.lock().unwrap()[request].messages[0])[0].clone();

        // Started, and talked to, it asks nothing of the computer, and nothing was kept yet.
        let mut changing = thread_in(&harness, place()).await;
        turn(&harness, &mut changing, "hello").await;
        assert_eq!(first.asked(), 0, "a thread that only talks asks nothing");
        assert_eq!(opening(0), "hello", "nothing was read there yet");

        // A command wakes it, and what it changed is read there as the turn ends, and kept.
        turn(&harness, &mut changing, "use spaces").await;
        assert!(first.asked() > 0, "the command asks for it");

        // A server that has not reached the computer since: a new thread opens with what was
        // kept, and neither it nor the first one resumed asks anything of the computer.
        let later = Arc::new(Sleepy::new(&reached, &scratch));
        let harness = harness.with_homes(Some(Arc::clone(&later) as Arc<dyn Homes>));
        let mut next = thread_in(&harness, place()).await;
        turn(&harness, &mut next, "hey").await;
        assert!(opening(3).contains("Use spaces."), "{}", opening(3));
        assert!(!opening(3).contains("Use tabs."), "{}", opening(3));
        let mut resumed = harness.resume_thread(scope(), changing.id).await.unwrap();
        turn(&harness, &mut resumed, "back").await;
        assert_eq!(
            later.asked(),
            0,
            "a resumed thread that only talks asks nothing"
        );

        // A command that changes nothing there keeps nothing new, and one on a computer that
        // cannot be reached keeps nothing, so a read it could not finish is never kept.
        turn(&harness, &mut next, "check").await;
        let gone = Arc::new(Sleepy::unreachable(&reached, &scratch));
        let harness = harness.with_homes(Some(Arc::clone(&gone) as Arc<dyn Homes>));
        let mut failing = thread_in(&harness, place()).await;
        turn(&harness, &mut failing, "use spaces again").await;
        assert!(gone.asked() > 0, "the command asked for it");
        let (scope, ledger) = (scope(), Reserved::Instructions.id());
        let kept = harness.log().read(&scope, ledger, None, usize::MAX);
        assert_eq!(kept.await.unwrap().len(), 1, "one read kept, not more");
        serving.abort();
    });
}

/// glave's sandbox on a person's computer, as on this one, across the protocol: a thread started
/// and talked to asks nothing of the computer, no sandbox wakes it, and at each level a command
/// is held there with no one asked: in Auto to the project, in Ask first to nothing it can write.
#[test]
fn commands_on_a_person_s_computer_run_in_glave_s_sandbox_and_nothing_is_asked() {
    let Some(base) = sandboxed("home") else {
        return;
    };
    let server = folder("walled-server");
    let home = base.join("home");
    let project = home.join("app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    block_on!(async {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        let computers = Arc::new(LocalComputers::new(base.join("copies"), environment));
        let (reached, serving) = home_across_the_wire(computers, &home).await;
        let run = |command: &str| {
            answer(
                None,
                vec![("bash", json!({ "command": command }))],
                StopReason::ToolUse,
            )
        };
        let answers = vec![
            answer(Some("Hi."), vec![], StopReason::EndTurn),
            run("echo kept > kept; echo lost > ../lost"),
            answer(Some("Done."), vec![], StopReason::EndTurn),
            run("echo asked > asked"),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let person = Person::answering(Decision::Deny);
        let (harness, _) = harness_seeing(
            &server,
            answers,
            person.clone(),
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        )
        .await;
        let sleepy = Arc::new(Sleepy::in_sandbox(&reached, &base.join("scratch"), &home));
        let harness = harness.with_homes(Some(Arc::clone(&sleepy) as Arc<dyn Homes>));
        let place = || Place::Folder(project.to_string_lossy().into_owned());

        let mut auto = thread_in(&harness, place()).await;
        turn(&harness, &mut auto, "hello").await;
        assert_eq!(sleepy.asked(), 0, "a thread that only talks wakes nothing");
        turn(&harness, &mut auto, "write").await;
        assert!(sleepy.asked() > 0, "the command wakes it");
        assert!(project.join("kept").exists(), "the project is writable");
        assert!(!home.join("lost").exists(), "what is beside it is not");

        let mut asking = thread_in(&harness, place()).await;
        harness
            .choose_permissions(&mut asking, &Caller::default(), Permissions::Ask)
            .await
            .unwrap();
        turn(&harness, &mut asking, "write").await;
        assert!(!project.join("asked").exists(), "Ask first's sandbox holds");
        assert!(person.asked().is_empty(), "the sandbox holds: no one asked");
        serving.abort();
    });
}

/// What the host says of a project there: its trusted hook, whatever its `config.toml` holds.
struct HookThere(Hook);

impl ProjectFolders for HookThere {
    fn for_folder<'a>(
        &'a self,
        _root: &'a Path,
        _project: Option<&'a Path>,
        _trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(std::future::ready(None))
    }

    fn hooks_at_home<'a>(
        &'a self,
        _scope: glave_ports::Scope,
        _project: &'a Path,
        _config: &'a str,
    ) -> BoxFuture<'a, Hooks> {
        let hooks = Hooks::new(vec![self.0.clone()]).unwrap();
        Box::pin(std::future::ready(hooks))
    }
}

/// A trusted hook of a project there runs on that computer through the port, in its sandbox, and
/// a turn that only talks, which runs no such hook, still wakes nothing.
#[test]
fn a_trusted_hook_runs_on_a_person_s_computer_in_its_sandbox_and_wakes_nothing_by_itself() {
    let Some(base) = sandboxed("home-hook") else {
        return;
    };
    let server = folder("hook-server");
    let home = base.join("home");
    let project = home.join("app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    std::fs::write(
        project.join(".glave/config.toml"),
        "[[hooks.PostToolUse]]\n",
    )
    .unwrap();
    block_on!(async {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        let computers = Arc::new(LocalComputers::new(base.join("copies"), environment));
        let (reached, serving) = home_across_the_wire(computers, &home).await;
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({ "command": "true" }))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
            answer(Some("Hello."), vec![], StopReason::EndTurn),
        ];
        let (harness, _) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        )
        .await;
        let hook = Hook {
            point: HookPoint::PostToolUse,
            matcher: None,
            command: "echo ran > hook-ran; echo lost > ../hook-lost".to_owned(),
            timeout_secs: None,
        };
        let sleepy = Arc::new(Sleepy::in_sandbox(&reached, &base.join("scratch"), &home));
        let harness = harness
            .with_project_folders(Some(Arc::new(HookThere(hook))))
            .with_homes(Some(Arc::clone(&sleepy) as Arc<dyn Homes>));
        let path = project.to_string_lossy().into_owned();
        let at_home = Workspace::new(reached.computers, reached.home);
        harness.keep_instructions(scope(), &at_home, &path).await;

        // Started and talked to, with a hook only after tools, it reaches nothing.
        let mut talking = thread_in(&harness, Place::Folder(path.clone())).await;
        let mut working = thread_in(&harness, Place::Folder(path)).await;
        turn(&harness, &mut working, "run it").await;
        assert!(project.join("hook-ran").exists(), "it ran on that computer");
        assert!(!home.join("hook-lost").exists(), "in its sandbox");
        let woken = sleepy.asked();
        turn(&harness, &mut talking, "hello").await;
        assert_eq!(sleepy.asked(), woken, "a turn that only talks asks nothing");
        serving.abort();
    });
}

#[test]
fn a_chat_on_a_person_s_computer_opens_with_their_own_instructions_as_last_read_there() {
    let server = folder("chat-server");
    let computer = folder("chat-computer");
    let home = computer.join("home");
    let scratch = home.join(".glave/scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    block_on!(async {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        let computers = LocalComputers::new(computer.join("copies"), environment);
        let computers = Arc::new(computers.with_scratch(&scratch));
        let (reached, serving) = home_across_the_wire(computers, &home).await;
        let own = home.join(".glave/AGENTS.md");
        let write = format!("printf 'Be brief.\\n' > '{}'", own.display());
        let helper = json!({ "description": "look", "prompt": "Look around." });
        let answers = vec![
            answer(Some("Hi."), vec![], StopReason::EndTurn),
            answer(None, vec![("agent", helper)], StopReason::ToolUse),
            answer(Some("Nothing to see."), vec![], StopReason::EndTurn),
            answer(Some("It saw nothing."), vec![], StopReason::EndTurn),
            answer(
                None,
                vec![("bash", json!({ "command": write }))],
                StopReason::ToolUse,
            ),
            answer(Some("Noted."), vec![], StopReason::EndTurn),
            answer(Some("Hey."), vec![], StopReason::EndTurn),
        ];
        let mut settings = settings();
        settings.subagents = true;
        let (harness, seen) =
            harness_seeing(&server, answers, AllowAll, InMemoryLog::new(), 1, settings).await;
        let first = Arc::new(Sleepy::new(&reached, &scratch));
        let harness = harness.with_homes(Some(Arc::clone(&first) as Arc<dyn Homes>));
        let opening = |request: usize| texts(&seen.lock().unwrap()[request].messages[0])[0].clone();

        // A chat that only talks asks nothing of the computer, and nothing was kept yet.
        let mut chat = thread_in(&harness, Place::Scratch).await;
        turn(&harness, &mut chat, "hello").await;
        assert_eq!(opening(0), "hello");
        turn(&harness, &mut chat, "ask a helper").await;
        assert_eq!(
            first.asked(),
            0,
            "a chat that only talks, or whose subagent only talks, asks nothing"
        );

        // A command that writes the person's own instructions, read there as its turn ends.
        turn(&harness, &mut chat, "be brief from now on").await;
        assert!(first.asked() > 0, "the command asks for it");

        // The next chat, on a server that has not reached the computer since, opens with them,
        // for its own folder, and asks nothing.
        let later = Arc::new(Sleepy::new(&reached, &scratch));
        let harness = harness.with_homes(Some(Arc::clone(&later) as Arc<dyn Homes>));
        let mut next = thread_in(&harness, Place::Scratch).await;
        turn(&harness, &mut next, "hey").await;
        assert_eq!(
            later.asked(),
            0,
            "a chat started and talked to asks nothing"
        );
        let opening = opening(6);
        let root = scratch.join(next.id.to_string());
        let heading = format!("# AGENTS.md instructions for {}", root.display());
        assert!(opening.starts_with(&heading), "{opening}");
        assert!(opening.contains("Be brief."), "{opening}");
        serving.abort();
    });
}
