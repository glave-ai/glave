//! Trusting a project from an app: a thread in a project that brings what runs only once the
//! person trusts it asks them when it is opened or started, and waits; trusting turns it on
//! from the next turn and asks nothing more; not now keeps it off and stops asking in that
//! thread alone; an ask the project no longer makes trusts nothing; nothing is ever on without
//! an answer; a caller signed in from elsewhere is never asked and cannot answer, and what the
//! person here trusts reaches no one else's threads; and a list's watch hears each ask raised
//! and answered.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_event::{Auth, AuthMethod, ThreadId};
use glave_harness::{Brought, ProjectFolders};
use glave_ports::{
    BoxFuture, Caller, Effect, InMemoryTools, ProjectId, ToolInfo, ToolOutput, ToolSpec,
};
use glave_session::protocol::{
    ApiError, NewFolder, Notification, Reply, Request, Status, ThreadSummary, TrustAsk,
    TrustChoice, TrustItem, code,
};
use glave_session::signin::person;
use glave_session::{Answering, ListWatch, ProjectTrust, Watched};
use serde_json::json;

use crate::support::*;

/// The tool the project's server offers once it is trusted.
const TOOL: &str = "mcp__acme__search";

/// A project whose folder brings a tool server, at a version the test can change, as a
/// repository changes its `.mcp.json`; trusted at most at one version.
#[derive(Default)]
struct Repository {
    version: Mutex<u32>,
    trusted: Mutex<Option<u32>>,
}

impl Repository {
    fn change(&self) {
        *self.version.lock().unwrap() += 1;
    }

    fn version(&self) -> u32 {
        *self.version.lock().unwrap()
    }

    fn is_trusted(&self) -> bool {
        *self.trusted.lock().unwrap() == Some(self.version())
    }
}

impl ProjectTrust for Repository {
    fn ask<'a>(&'a self, _folder: &'a Path) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>> {
        let ask = (!self.is_trusted()).then(|| TrustAsk {
            ask: format!("v{}", self.version()),
            items: vec![TrustItem {
                text: "Runs a tool server".into(),
                detail: Some("npx @acme/mcp".into()),
            }],
        });
        Box::pin(async move { Ok(ask) })
    }

    fn trust<'a>(
        &'a self,
        _folder: &'a Path,
        ask: &'a str,
    ) -> BoxFuture<'a, Result<bool, ApiError>> {
        let now = format!("v{}", self.version());
        let trusted = ask == now;
        if trusted {
            *self.trusted.lock().unwrap() = Some(self.version());
        }
        Box::pin(async move { Ok(trusted) })
    }
}

impl ProjectFolders for Repository {
    fn for_folder<'a>(
        &'a self,
        _root: &'a Path,
        _project: Option<&'a Path>,
        _trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }

    fn brought<'a>(&'a self, _project: &'a Path, _id: ProjectId) -> BoxFuture<'a, Brought> {
        if !self.is_trusted() {
            return Box::pin(async { Brought::default() });
        }
        let info = ToolInfo {
            spec: ToolSpec {
                name: TOOL.into(),
                description: "Searches Acme.".into(),
                parameters: json!({"type": "object"}),
            },
            effect: Effect::Reads,
            parallel: true,
        };
        let tools = InMemoryTools::new(vec![(info, ToolOutput::text(false, "Found."))]);
        let brought = Brought {
            tools: vec![Arc::new(tools)],
            ..Brought::default()
        };
        Box::pin(async move { brought })
    }
}

/// A server whose own project is `repository`, answering `answers`.
async fn serving(
    dir: &Path,
    repository: &Arc<Repository>,
    answers: usize,
    answering: Answering,
) -> (Tested, Seen) {
    let answers = (0..answers).map(|_| answer(Some("Done."), None)).collect();
    let mut setup = Setup::new(answers);
    setup.answering = answering;
    setup.project_folders = Some(Arc::clone(repository) as Arc<dyn ProjectFolders>);
    setup.trust = Some(Arc::clone(repository) as Arc<dyn ProjectTrust>);
    setup.build(dir).await
}

fn thread_start(project: ProjectId) -> Request {
    Request::ThreadStart {
        project: Some(project),
        worktree: None,
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    }
}

fn summary(reply: Reply) -> ThreadSummary {
    match reply {
        Reply::Thread { thread } => thread,
        other => panic!("a thread, not {other:?}"),
    }
}

async fn opened(server: &Tested, caller: &Caller, thread: ThreadId) -> ThreadSummary {
    summary(
        server
            .request_as(caller, Request::ThreadGet { thread })
            .await
            .unwrap(),
    )
}

fn answering(thread: ThreadId, ask: &str, answer: TrustChoice) -> Request {
    Request::TrustAnswer {
        thread,
        ask: ask.into(),
        answer,
    }
}

/// Whether the model was offered the project's tool in request `index`.
fn offered(seen: &Seen, index: usize) -> bool {
    json(seen)[index].contains(TOOL)
}

#[test]
fn a_thread_in_an_untrusted_project_asks_the_person_here_and_waits_until_they_answer() {
    let dir = folder("trust-asks");
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, seen) = serving(&dir, &repository, 2, Answering::Clients).await;
        let started = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        let ask = started.trust.clone().expect("it asks");
        let renamed = server
            .request(Request::ThreadRename {
                thread: started.id,
                title: "Acme".into(),
            })
            .await
            .unwrap();
        assert_eq!(summary(renamed).trust.as_ref(), Some(&ask), "still asking");
        assert_eq!(ask.items[0].detail.as_deref(), Some("npx @acme/mcp"));
        assert_eq!(started.status, Status::Waiting, "so Needs you lists it");
        let listed = server
            .request(Request::ThreadList {
                project: Some(scope().project()),
                limit: None,
                show: None,
                cursor: None,
            })
            .await
            .unwrap();
        let Reply::Threads { threads, .. } = listed else {
            panic!("threads")
        };
        assert_eq!(threads[0].trust.as_ref(), Some(&ask));
        assert_eq!(threads[0].status, Status::Waiting);
        // Nothing is on without an answer, however many turns run.
        turn(&server, started.id, "search acme").await.unwrap();
        assert!(!offered(&seen, 0));
        assert!(!repository.is_trusted());
        assert_eq!(
            opened(&server, &Caller::default(), started.id).await.trust,
            Some(ask.clone()),
            "it still asks"
        );
        // Trusted: asked nothing more, in this thread or a new one, and on from the next turn.
        let answered = server
            .request(answering(started.id, &ask.ask, TrustChoice::Trust))
            .await
            .unwrap();
        let answered = summary(answered);
        assert_eq!((answered.trust, answered.status), (None, Status::Idle));
        assert!(repository.is_trusted());
        turn(&server, started.id, "search acme").await.unwrap();
        assert!(offered(&seen, 1), "the project's tool, from the next turn");
        let another = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        assert_eq!(another.trust, None);
        let again = server
            .request(answering(started.id, &ask.ask, TrustChoice::Trust))
            .await
            .unwrap_err();
        assert_eq!(again.code, code::NO_TRUST, "answered already");
    });
}

#[test]
fn an_ask_the_project_no_longer_makes_trusts_nothing_and_what_changed_asks_again() {
    let dir = folder("trust-changed");
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, _) = serving(&dir, &repository, 0, Answering::Clients).await;
        let started = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        let shown = started.trust.unwrap();
        repository.change();
        let refused = server
            .request(answering(started.id, &shown.ask, TrustChoice::Trust))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NO_TRUST, "{refused}");
        assert!(!repository.is_trusted(), "nothing the person was not shown");
        let asked = opened(&server, &Caller::default(), started.id).await;
        assert_ne!(
            asked.trust.unwrap().ask,
            shown.ask,
            "asked again, about what is there"
        );
    });
}

#[test]
fn not_now_keeps_it_off_and_stops_asking_in_that_thread_alone() {
    let dir = folder("trust-not-now");
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, seen) = serving(&dir, &repository, 1, Answering::Clients).await;
        let first = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        let ask = first.trust.unwrap();
        let declined = server
            .request(answering(first.id, &ask.ask, TrustChoice::NotNow))
            .await
            .unwrap();
        let declined = summary(declined);
        assert_eq!((declined.trust, declined.status), (None, Status::Idle));
        assert_eq!(
            opened(&server, &Caller::default(), first.id).await.trust,
            None
        );
        turn(&server, first.id, "search acme").await.unwrap();
        assert!(!offered(&seen, 0), "still off");
        assert!(!repository.is_trusted(), "and nothing kept");
        let next = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        assert_eq!(next.trust, Some(ask), "a new thread asks again");
    });
}

#[test]
fn with_no_one_to_answer_nothing_is_asked() {
    let dir = folder("trust-no-one");
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, _) = serving(&dir, &repository, 0, Answering::NoOne).await;
        let started = summary(
            server
                .request(thread_start(scope().project()))
                .await
                .unwrap(),
        );
        assert_eq!((started.trust, started.status), (None, Status::Idle));
        let answered = server
            .request(answering(started.id, "v0", TrustChoice::Trust))
            .await
            .unwrap_err();
        assert_eq!(answered.code, code::NO_TRUST);
        assert!(!repository.is_trusted());
    });
}

#[test]
fn a_caller_signed_in_elsewhere_is_never_asked_and_cannot_answer() {
    let dir = folder("trust-elsewhere");
    let other = dir.with_extension("project");
    std::fs::create_dir_all(&other).unwrap();
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, seen) = serving(&dir, &repository, 1, Answering::Clients).await;
        // The person glave works for, signed in from another computer, as a host that knows them
        // by that sign-in calls for them.
        let elsewhere = Caller {
            person: server.person(),
            auth: Some(Auth {
                method: AuthMethod::Oidc,
                issuer: Some("https://id.example.com".into()),
                subject: Some("ada".into()),
                session: None,
            }),
            trace: None,
            profile: None,
        };
        let started = server
            .request_as(&elsewhere, thread_start(scope().project()))
            .await
            .unwrap();
        let started = summary(started);
        assert_eq!(
            (started.trust.clone(), started.status),
            (None, Status::Idle)
        );
        assert_eq!(opened(&server, &elsewhere, started.id).await.trust, None);
        // The ask the person here is shown is not theirs to answer.
        let ask = opened(&server, &Caller::default(), started.id)
            .await
            .trust
            .unwrap();
        for answer in [TrustChoice::Trust, TrustChoice::NotNow] {
            let refused = server
                .request_as(&elsewhere, answering(started.id, &ask.ask, answer))
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
        }
        assert!(!repository.is_trusted());
        // Trusted by the person here, it is theirs alone: another person, as a host embedding the
        // server may call for her, in a project of her own whose folder brings the same, gets
        // none of it.
        let answered = server
            .request(answering(started.id, &ask.ask, TrustChoice::Trust))
            .await;
        assert!(answered.is_ok(), "{answered:?}");
        let grace = Caller {
            person: Some(person("https://id.example.com", "grace").unwrap()),
            ..Caller::default()
        };
        let created = Request::ProjectCreate {
            name: "Hers".into(),
            folder: Some(NewFolder::Path {
                path: other.to_string_lossy().into_owned(),
            }),
        };
        let Reply::Project { project, .. } = server.request_as(&grace, created).await.unwrap()
        else {
            panic!("a project")
        };
        let hers = summary(
            server
                .request_as(&grace, thread_start(project.id))
                .await
                .unwrap(),
        );
        let turn = Request::TurnStart {
            thread: hers.id,
            prompt: "search acme".into(),
            background: false,
            budget: None,
            model: None,
            permissions: None,
            effort: None,
            meta: glave_session::protocol::Meta::default(),
            unattended: false,
            files: Vec::new(),
        };
        server.request_as(&grace, turn).await.unwrap();
        assert!(!offered(&seen, 0), "no project tools for another's thread");
    });
}

/// Reads `watch` until it has sent, of each of `threads`, a summary that `wanted` takes.
async fn heard(
    watch: &mut ListWatch,
    threads: &[ThreadId],
    wanted: impl Fn(&ThreadSummary) -> bool,
) {
    let mut left: Vec<ThreadId> = threads.to_vec();
    while !left.is_empty() {
        let Some(Watched::Note(note)) = watch.next().await else {
            panic!("the list's watch ended");
        };
        if let Notification::Changed { summary, .. } = note.as_ref()
            && wanted(summary)
        {
            left.retain(|thread| *thread != summary.id);
        }
    }
}

#[test]
fn a_list_watch_hears_threads_ask_and_every_one_stop_once_the_project_is_trusted() {
    let dir = folder("trust-list");
    block_on!(async {
        let repository = Arc::new(Repository::default());
        let (server, _) = serving(&dir, &repository, 0, Answering::Clients).await;
        let mut watch = server
            .watch_list(&Caller::default(), None, None)
            .await
            .unwrap();
        let start = || server.request(thread_start(scope().project()));
        let first = summary(start().await.unwrap());
        let second = summary(start().await.unwrap());
        let waits = |s: &ThreadSummary| s.status == Status::Waiting && s.trust.is_some();
        heard(&mut watch, &[first.id, second.id], waits).await;
        let ask = first.trust.expect("it asks").ask;
        server
            .request(answering(first.id, &ask, TrustChoice::Trust))
            .await
            .unwrap();
        // Trusted from one thread, the other asks no more either.
        let asks_nothing = |s: &ThreadSummary| s.status == Status::Idle && s.trust.is_none();
        heard(&mut watch, &[first.id, second.id], asks_nothing).await;
    });
}
