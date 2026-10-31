//! Taking someone out of an organization (`specs/model.md`, "Identity"): one admin action ends,
//! at once, the watches they hold there and refuses their next request; signing in again
//! through the company login does not bring them back, an invite does, on every server on the
//! store. Only admins remove, only an Owner removes an Owner, and someone not there is no one
//! to remove. What they have running ends with them, on every server on the store (and only in
//! the organization they were removed from): a watch, a turn, a task and a command in the
//! background; and their schedules are deleted.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{
    Body, CommandEnding, OrgRole, Outcome, PrincipalKind, RawEvent, Secret, ThreadId, When, decode,
};
use glave_ports::stream::Reserved;
use glave_ports::testing::HUNG;
use glave_ports::{Caller, EventLog as _, OrgId, Scope};
use glave_session::protocol::{ApiError, Notification, Reply, Request, Status, code};
use glave_session::{Sent, TurnItem, TurnStream, Watched};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::*;

/// A server whose people sign in through a company login.
async fn organization(dir: &std::path::Path) -> Tested {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.build(dir).await.0
}

/// The `seed`th server of one organization on `log`.
async fn on(dir: &std::path::Path, log: &Shared, seed: u64) -> Tested<Shared> {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.seed = seed;
    setup.build_on(dir, log.clone()).await.0
}

/// A server on `log` whose model calls wait for ever, so a turn runs until it is stopped, and
/// which answers each of `answers` before that.
async fn waiting_on(
    dir: &std::path::Path,
    log: &Shared,
    seed: u64,
    answers: Vec<glave_ports::Completion>,
) -> Tested<Shared> {
    let mut setup = Setup::new(answers);
    setup.identity_provider = true;
    setup.seed = seed;
    setup.gate = Some(Arc::new(Notify::new()));
    setup.build_on(dir, log.clone()).await.0
}

/// A new thread in `caller`'s Chats on `server`, and where it is.
async fn chat(server: &Tested<Shared>, caller: &Caller, background: bool) -> (ThreadId, Scope) {
    let started = Request::ThreadStart {
        project: None,
        background,
        worktree: None,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => {
            let org = caller.org.unwrap_or_else(|| scope().org());
            (thread.id, Scope::glaves_own(org, thread.project))
        }
        other => panic!("a thread, not {other:?}"),
    }
}

/// Starts a turn for `caller`, followed until the model is called.
async fn running_turn(server: &Tested<Shared>, caller: &Caller, thread: ThreadId) -> TurnStream {
    let started = server
        .start_streaming(caller, turn_start(thread, "go on", None))
        .await;
    let Ok(Sent::Started(mut stream)) = started else {
        panic!("a turn of its own");
    };
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Event { event, .. } = note.as_ref()
            && event.kind.as_str() == "turn.started"
        {
            return stream;
        }
    }
    panic!("the turn never started");
}

/// What `find` first finds among the events of `stream` the store holds, once it holds it:
/// work told to stop ends in its own time.
async fn recorded<T>(
    log: &Shared,
    at: &Scope,
    stream: ThreadId,
    find: impl Fn(RawEvent) -> Option<T>,
) -> T {
    let found = tokio::time::timeout(HUNG, async {
        loop {
            let events = log.read(at, stream, None, usize::MAX).await.unwrap();
            if let Some(found) = events.into_iter().find_map(&find) {
                return found;
            }
            tokio::task::yield_now().await;
        }
    });
    found.await.expect("it was recorded")
}

async fn outcome_of(log: &Shared, at: &Scope, thread: ThreadId) -> Outcome {
    recorded(log, at, thread, |raw| match decode(raw).unwrap().body {
        Body::TurnEnded(ended) => Some(ended.outcome),
        _ => None,
    })
    .await
}

async fn remove<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    caller: &Caller,
    subject: &str,
) -> Result<Reply, ApiError> {
    let person = id(subject);
    server
        .request_as(caller, Request::PersonRemove { person })
        .await
}

fn list() -> Request {
    Request::ThreadList {
        project: None,
        limit: None,
        show: None,
        cursor: None,
    }
}

#[test]
fn someone_removed_has_their_watches_end_at_once_and_comes_back_only_by_an_invite() {
    block_on!(async {
        let dir = folder("remove-person");
        let server = organization(&dir).await;
        let ada = signed_in("ada");
        server.request_as(&ada, list()).await.unwrap();
        let mut watching = server.watch_list(&ada, None, None).await.unwrap();
        let owner = Caller::default();
        let (came, removed) = tokio::join!(watching.next(), remove(&server, &owner, "ada"));
        removed.unwrap();
        let Some(Watched::Ended(Err(ended))) = came else {
            panic!("the watch ended, not {came:?}");
        };
        assert_eq!(ended.code, code::NO_SIGN_IN);
        assert!(watching.next().await.is_none(), "nothing more after");

        // Signing in again through the company login lets her in to nothing.
        let refused = server.request_as(&ada, list()).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert!(
            refused.message.to_string().contains("invite"),
            "{refused:?}"
        );
        let people = Request::PeopleList {
            after: None,
            limit: None,
        };
        let Ok(Reply::People { people }) = server.request(people).await else {
            panic!("people");
        };
        assert!(people.iter().all(|person| person.id != id("ada")));

        // An invite brings her back, with its role.
        let invite = Request::InviteCreate {
            role: OrgRole::Admin,
            email: None,
        };
        let Ok(Reply::Invited { code, .. }) = server.request(invite).await else {
            panic!("an invite");
        };
        let accept = Request::InviteAccept {
            code: Secret::new(code.expose()),
        };
        let Ok(Reply::Person { person }) = server.request_as(&ada, accept).await else {
            panic!("back");
        };
        assert_eq!(person.role, OrgRole::Admin);
        server.request_as(&ada, list()).await.unwrap();
    });
}

#[test]
fn only_admins_remove_and_only_owners_remove_owners() {
    block_on!(async {
        let dir = folder("remove-who");
        let server = organization(&dir).await;
        let (ada, alan, grace) = (signed_in("ada"), signed_in("alan"), signed_in("grace"));
        for caller in [&ada, &alan, &grace] {
            server.request_as(caller, list()).await.unwrap();
        }
        let owner = Caller::default();
        for (person, role) in [("ada", OrgRole::Admin), ("grace", OrgRole::Owner)] {
            let set = Request::RoleSet {
                person: id(person),
                role,
            };
            server.request_as(&owner, set).await.unwrap();
        }
        let refused = remove(&server, &alan, "ada").await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "a Member removes no one");
        let refused = remove(&server, &ada, "grace").await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "an Admin removes no Owner");
        let nobody = remove(&server, &ada, "linus").await.unwrap_err();
        assert_eq!(nobody.code, code::NO_PERSON);
        remove(&server, &ada, "alan").await.unwrap();
        remove(&server, &grace, "ada").await.unwrap();
        // Asked again, it is done again.
        remove(&server, &grace, "ada").await.unwrap();
    });
}

#[test]
fn another_server_on_the_store_refuses_them_at_their_next_request() {
    block_on!(async {
        let (first, second) = (folder("remove-first"), folder("remove-second"));
        let log = Shared::default();
        let one = on(&first, &log, 1).await;
        let two = on(&second, &log, 2).await;
        let ada = signed_in("ada");
        two.request_as(&ada, list()).await.unwrap();
        remove(&one, &Caller::default(), "ada").await.unwrap();
        let refused = two.request_as(&ada, list()).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
    });
}

#[test]
fn a_watch_on_another_server_on_the_store_ends_within_seconds_of_their_removal() {
    block_on!(async {
        let (first, second) = (folder("removal-first"), folder("removal-second"));
        let log = Shared::default();
        let one = on(&first, &log, 1).await;
        let two = on(&second, &log, 2).await;
        let ada = signed_in("ada");
        one.request_as(&ada, list()).await.unwrap();
        two.request_as(&ada, list()).await.unwrap();
        let mut watching = two.watch_list(&ada, None, None).await.unwrap();

        // Time stands still but for what waits, so this says how long it took, however busy
        // the machine is.
        tokio::time::pause();
        let began = tokio::time::Instant::now();
        let ended = async {
            tokio::select! {
                came = watching.next() => came,
                () = two.follow_removals() => unreachable!("it follows until dropped"),
            }
        };
        let owner = Caller::default();
        let (came, removed) = tokio::join!(ended, remove(&one, &owner, "ada"));
        removed.unwrap();
        let Some(Watched::Ended(Err(ended))) = came else {
            panic!("the watch ended, not {came:?}");
        };
        assert_eq!(ended.code, code::NO_SIGN_IN);
        // One look again, and the timer's last millisecond.
        assert!(
            began.elapsed() < Duration::from_secs(6),
            "{:?}",
            began.elapsed()
        );
    });
}

#[test]
fn someone_removed_has_their_turn_end_on_any_server_and_only_theirs() {
    block_on!(async {
        let (first, second) = (folder("removal-turn-1"), folder("removal-turn-2"));
        let log = Shared::default();
        let one = on(&first, &log, 1).await;
        let two = waiting_on(&second, &log, 2, Vec::new()).await;
        let (ada, alan) = (signed_in("ada"), signed_in("alan"));
        // Known to the organization, and another's turn beside hers.
        one.request_as(&ada, list()).await.unwrap();
        let (hers, at) = chat(&two, &ada, false).await;
        let (his, _) = chat(&two, &alan, false).await;
        let _hers = running_turn(&two, &ada, hers).await;
        let _his = running_turn(&two, &alan, his).await;

        // Nothing yet of a removal no one made.
        two.look_for_removals().await;
        remove(&one, &Caller::default(), "ada").await.unwrap();
        two.look_for_removals().await;
        assert_eq!(outcome_of(&log, &at, hers).await, Outcome::Interrupted);

        let running = two
            .request_as(&alan, Request::ThreadGet { thread: his })
            .await;
        let Ok(Reply::Thread { thread }) = running else {
            panic!("his thread, not {running:?}");
        };
        assert_eq!(thread.status, Status::Running);
        two.shutdown().await;
    });
}

#[test]
fn someone_removed_from_one_organization_keeps_their_turn_in_another() {
    block_on!(async {
        let dir = folder("removal-orgs");
        let log = Shared::default();
        let server = waiting_on(&dir, &log, 1, Vec::new()).await;
        let other = OrgId::from_uuid(uuid::Uuid::from_u128(3));
        let ada = signed_in("ada");
        let ada_there = Caller {
            org: Some(other),
            ..signed_in("ada")
        };
        server.request_as(&ada, list()).await.unwrap();
        let (there, _) = chat(&server, &ada_there, false).await;
        let _turn = running_turn(&server, &ada_there, there).await;

        remove(&server, &Caller::default(), "ada").await.unwrap();
        server.look_for_removals().await;

        let running = server
            .request_as(&ada_there, Request::ThreadGet { thread: there })
            .await;
        let Ok(Reply::Thread { thread }) = running else {
            panic!("her thread there, not {running:?}");
        };
        assert_eq!(thread.status, Status::Running);
        server.shutdown().await;
    });
}

#[test]
fn someone_removed_has_their_task_stopped_as_by_whoever_removed_them() {
    block_on!(async {
        let dir = folder("removal-task");
        let log = Shared::default();
        let server = waiting_on(&dir, &log, 1, Vec::new()).await;
        let ada = signed_in("ada");
        let (thread, at) = chat(&server, &ada, true).await;
        let mut started = turn_start(thread, "tidy up", None);
        let Request::TurnStart { background, .. } = &mut started else {
            panic!("a turn");
        };
        *background = true;
        server.request_as(&ada, started).await.unwrap();

        remove(&server, &Caller::default(), "ada").await.unwrap();

        let ended = recorded(&log, &at, Reserved::Tasks.id(), |raw| {
            match decode(raw).unwrap().body {
                Body::TaskEnded(ended) => Some(ended),
                _ => None,
            }
        })
        .await;
        assert_eq!(ended.thread, thread);
        assert_eq!(ended.outcome, Outcome::Interrupted);
        assert!(ended.by.is_some(), "stopped by whoever removed her");
    });
}

#[test]
fn someone_removed_has_their_commands_in_the_background_stopped() {
    block_on!(async {
        let dir = folder("removal-commands");
        let log = Shared::default();
        let answers = vec![
            answer(
                None,
                Some((
                    "bash",
                    json!({"command": "tail -f /dev/null", "background": true}),
                )),
            ),
            answer(Some("It is running."), None),
        ];
        let mut setup = Setup::new(answers);
        setup.identity_provider = true;
        let server = setup.build_on(&dir, log.clone()).await.0;
        let ada = signed_in("ada");
        let (thread, at) = chat(&server, &ada, false).await;
        let ran = server
            .request_as(&ada, turn_start(thread, "start the server", None))
            .await;
        assert!(ran.is_ok(), "{ran:?}");

        remove(&server, &Caller::default(), "ada").await.unwrap();

        let ending = recorded(&log, &at, thread, |raw| match decode(raw).unwrap().body {
            Body::CommandEnded(ended) => Some(ended.ending),
            _ => None,
        })
        .await;
        let CommandEnding::Stopped { by: Some(by) } = ending else {
            panic!("stopped by whoever removed her, not {ending:?}");
        };
        assert_eq!(by.kind, PrincipalKind::Person);
    });
}

#[test]
fn someone_removed_has_their_schedules_deleted() {
    block_on!(async {
        let dir = folder("removal-schedules");
        let server = organization(&dir).await;
        let ada = signed_in("ada");
        let schedule = Request::ScheduleCreate {
            project: None,
            when: When::Cron("0 9 * * *".into()),
            prompt: "check the build".into(),
            goal: false,
            budget: 5_000,
        };
        server.request_as(&ada, schedule).await.unwrap();

        remove(&server, &Caller::default(), "ada").await.unwrap();

        // Brought back by an invite, nothing of hers fires.
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let Ok(Reply::Invited { code, .. }) = server.request(invite).await else {
            panic!("an invite");
        };
        let accept = Request::InviteAccept {
            code: Secret::new(code.expose()),
        };
        server.request_as(&ada, accept).await.unwrap();
        let listed = server
            .request_as(&ada, Request::ScheduleList {})
            .await
            .unwrap();
        let Reply::Schedules { schedules } = listed else {
            panic!("her schedules, not {listed:?}");
        };
        assert_eq!(schedules, []);
    });
}
