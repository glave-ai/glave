//! Everyone's schedules, whoever made them: a run acts as its maker, with what they may do and
//! spend; a server started since fires them though their maker asked it nothing, from the
//! store's index of where schedules are; a maker removed from the organization meanwhile has
//! their run refused; and schedules are offered only where a server fires them.

use glave_event::{
    At, Body, EventId, Header, PersonRemoved, Principal, ScheduleId, Seq, Skipped, ThreadId, When,
    decode, encode,
};
use glave_ports::stream::Reserved;
use glave_ports::testing::ManualClock;
use std::sync::Arc;

use glave_ports::{
    Caller, EventLog as _, InMemoryLog, InMemoryProjects, ProjectId, Projects, Scope,
};
use glave_session::protocol::{Reply, Request, ScheduleInfo, code};
use jiff::Timestamp;

use crate::support::*;

fn set(now: &ManualClock, text: &str) {
    now.set_millis(text.parse::<Timestamp>().unwrap().as_millisecond());
}

/// A server with plenty of short answers, its clock at 10:03 until the test moves it.
fn setup() -> Setup {
    let mut setup = Setup::new((0..10).map(|_| answer(Some("Done."), None)).collect());
    setup.clock = ManualClock::at_millis(
        "2026-10-10T10:03:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond(),
    );
    setup
}

/// A schedule `caller` makes in their Chats, every day at 11:00.
async fn daily<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    caller: &Caller,
) -> Result<ScheduleInfo, i64> {
    let request = Request::ScheduleCreate {
        project: None,
        name: None,
        when: When::Cron("0 11 * * *".into()),
        zone: None,
        prompt: "check the build".into(),
        budget: 5_000,
    };
    match server.request_as(caller, request).await {
        Ok(Reply::Schedule { schedule }) => Ok(schedule),
        Ok(other) => panic!("a schedule, not {other:?}"),
        Err(error) => Err(error.code),
    }
}

/// What `project`'s task ledger records.
async fn ledger_of(log: &Shared, project: ProjectId) -> Vec<Body> {
    let ledger = Scope::glaves_own(scope().org(), project);
    let events = log
        .read(&ledger, Reserved::Tasks.id(), None, usize::MAX)
        .await
        .unwrap();
    (events.into_iter())
        .map(|raw| decode(raw).unwrap().body)
        .collect()
}

/// Who each task started in `project`'s ledger ran as, and for which schedule.
async fn runs_in(log: &Shared, project: ProjectId) -> Vec<(Option<Principal>, ScheduleId)> {
    (ledger_of(log, project).await.into_iter())
        .filter_map(|body| match body {
            Body::TaskStarted(started) => {
                Some((started.by, started.schedule.map(|fired| fired.schedule)?))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_server_started_since_fires_everyone_s_schedules_each_as_whoever_made_it() {
    block_on!(async {
        let log = Shared::default();
        // One store: its log, and its projects.
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        let mut first = setup();
        first.projects = Some(Arc::clone(&projects));
        let now = first.clock.clone();
        let (server, _, repo, _dir) = first.build_in_repository("people-fire", log.clone()).await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        let mine = daily(&server, &Caller::default()).await.unwrap();
        server.shutdown().await;
        drop(server);
        // Neither has asked the new server anything.
        let mut later = setup();
        later.clock = now.clone();
        later.projects = Some(projects);
        let (restarted, _) = later.build_on(&repo, log.clone()).await;
        set(&now, "2026-10-10T11:00:00Z");
        let mut fired = restarted.tick().await.unwrap();
        settle(&restarted).await;
        fired.sort();
        let mut chats: Vec<ThreadId> = [&theirs, &mine]
            .iter()
            .filter_map(|made| made.thread)
            .collect();
        chats.sort();
        assert_eq!(fired, chats, "both, though neither asked it anything");
        let grace_id = grace.person.unwrap();
        assert_eq!(
            runs_in(&log, theirs.project).await,
            vec![(Some(Principal::person(grace_id)), theirs.schedule)],
            "as her"
        );
        assert_eq!(
            runs_in(&log, mine.project).await,
            vec![(None, mine.schedule)],
            "as the person on this computer"
        );
    });
}

#[test]
fn a_run_of_someone_removed_meanwhile_is_refused() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("people-removed", log.clone())
            .await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        // Removed on another server, which has yet to delete her schedules.
        let people = Scope::of_organization(scope().org());
        let ledger = Reserved::People.id();
        let before = log.read(&people, ledger, None, usize::MAX).await.unwrap();
        let next = u64::try_from(before.len()).unwrap() + 1;
        let at = At::try_from("2026-10-10T10:30:00.000Z".to_owned()).unwrap();
        let header = Header {
            id: EventId::new(at.timestamp(), [7; 10]).unwrap(),
            thread: ledger,
            seq: Seq::new(next).unwrap(),
            at,
        };
        let removed = PersonRemoved {
            person: grace.person.unwrap(),
            by: Principal::person(id("ada")),
        };
        log.append(&people, &[encode(header, &removed).unwrap()])
            .await
            .unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(runs_in(&log, theirs.project).await, Vec::new());
        let refused =
            (ledger_of(&log, theirs.project).await.into_iter()).find_map(|body| match body {
                Body::ScheduleSkipped(skipped) => Some(skipped.why),
                _ => None,
            });
        assert!(
            matches!(refused, Some(Skipped::Refused { .. })),
            "{refused:?}"
        );
    });
}

#[test]
fn schedules_are_offered_only_where_a_server_fires_them() {
    block_on!(async {
        let mut setup = setup();
        setup.schedules = false;
        let (server, _, _, _dir) = setup
            .build_in_repository("people-none", InMemoryLog::new())
            .await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert!(!info.schedules);
        assert_eq!(
            daily(&server, &Caller::default()).await,
            Err(code::NOT_AVAILABLE)
        );
        let (here, _, _, _dir) = self::setup()
            .build_in_repository("people-here", InMemoryLog::new())
            .await;
        let Reply::Server(info) = here.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert!(info.schedules);
    });
}
