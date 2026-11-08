//! Forking a thread (`specs/session.md`, "Forking a thread"): the new thread holds the other's
//! history up to a point between its turns and goes on from there, the other untouched; its
//! model calls open with the other's prefix, byte for byte, so the cache still matches
//! (`specs/invariants.md`, 27); it says plainly that files are not rewound, and where it works;
//! it counts its own receipts; its calls and commands are numbered after its copies', and none
//! of the other's commands are its; what its copies name outlives the other; and a point
//! within a turn is refused.

use glave_event::{Body, ForkFiles, Opened, OriginKind, Part, Permissions, Seq, ThreadId, decode};
use glave_ports::{BlobStore, Completion, InMemoryLog, ModelRequest};
use glave_session::protocol::{ApiError, Meta, Reply, Request, ThreadSummary, code};
use serde_json::json;

use crate::support::*;

fn setup(answers: Vec<Completion>) -> Setup {
    let mut setup = Setup::new(answers);
    setup.settings.titles = false;
    setup
}

/// `thread` forked at `at`, answered with the fork's summary.
async fn fork(
    server: &Tested,
    thread: ThreadId,
    at: Option<Seq>,
) -> Result<ThreadSummary, ApiError> {
    match server.request(Request::ThreadFork { thread, at }).await? {
        Reply::Thread { thread } => Ok(thread),
        other => panic!("a thread, not {other:?}"),
    }
}

/// The `seq` of each event of `kind` in `thread`, in order.
async fn seqs_of(server: &Tested, thread: ThreadId, kind: &str) -> Vec<Seq> {
    events(server, thread)
        .await
        .iter()
        .filter(|event| event.kind.as_str() == kind)
        .map(|event| event.seq)
        .collect()
}

/// What each message of `thread` says, in order: its last text.
async fn said(server: &Tested, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::MessageAdded(added) => added.parts.iter().rev().find_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect()
}

/// How `thread.forked` says the fork stands as to files, and what it tells the person.
async fn forked_record(server: &Tested, thread: ThreadId) -> (ForkFiles, String) {
    let last = events(server, thread).await.pop().unwrap();
    match decode(last).unwrap().body {
        Body::ThreadForked(forked) => (forked.files, forked.told),
        other => panic!("the fork's record last, not {other:?}"),
    }
}

/// A turn saying `prompt` in `thread`, run with `permissions`.
async fn turn_in(server: &Tested, thread: ThreadId, prompt: &str, permissions: Permissions) {
    let started = Request::TurnStart {
        thread,
        prompt: prompt.into(),
        background: false,
        budget: None,
        meta: Meta::default(),
        model: None,
        permissions: Some(permissions),
        unattended: false,
        files: Vec::new(),
        effort: None,
    };
    server.request(started).await.unwrap();
}

/// What a model call opens with, as it was sent: its system prompt and its tools.
fn opening(request: &ModelRequest) -> (String, String) {
    (
        serde_json::to_string(&request.system).unwrap(),
        serde_json::to_string(&request.tools).unwrap(),
    )
}

#[test]
fn a_fork_holds_the_history_up_to_its_point_and_the_original_stays_as_it_was() {
    let dir = folder("forks-history");
    block_on!(async {
        let answers = vec![
            costing(answer(Some("One."), None), 100, 10, 50),
            costing(answer(Some("Two."), None), 100, 10, 50),
            costing(answer(Some("Two, again."), None), 7, 1, 3),
        ];
        let (server, seen) = setup(answers).build(&dir).await;
        let original = start(&server, false).await;
        turn(&server, original, "first").await.unwrap();
        turn(&server, original, "second").await.unwrap();
        let before = events(&server, original).await;
        let first_ended = seqs_of(&server, original, "turn.ended").await[0];

        let forked = fork(&server, original, Some(first_ended)).await.unwrap();
        assert_eq!(forked.origin.kind, OriginKind::Fork);
        assert_eq!(forked.origin.reference, Some(original.to_string()));
        assert_eq!(forked.project, scope().project());
        assert_eq!(forked.title.as_deref(), Some("first"));
        assert_eq!(forked.turns, 1);
        assert_eq!(
            forked.tokens.input, 0,
            "the copies used what the original paid for"
        );
        assert_eq!(said(&server, forked.id).await, ["first", "One."]);
        let kinds = kinds(&server, forked.id).await;
        assert_eq!(kinds.first().map(String::as_str), Some("thread.started"));
        assert_eq!(kinds.last().map(String::as_str), Some("thread.forked"));
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| *kind == "workspace.bound")
                .count(),
            1,
            "where it works is its own"
        );

        turn(&server, forked.id, "second, differently")
            .await
            .unwrap();
        let sent = format!("{:?}", seen.lock().unwrap()[2].messages);
        assert!(
            sent.contains("One.") && sent.contains("second, differently"),
            "{sent}"
        );
        assert!(!sent.contains("Two."), "nothing after its point: {sent}");
        let summary = match server
            .request(Request::ThreadGet { thread: forked.id })
            .await
        {
            Ok(Reply::Thread { thread }) => thread,
            other => panic!("a thread, not {other:?}"),
        };
        assert_eq!(summary.tokens.input, 7, "its own call alone");

        let after = events(&server, original).await;
        let json = |events: &[glave_event::RawEvent]| -> Vec<String> {
            events
                .iter()
                .map(|event| event.to_json().unwrap())
                .collect()
        };
        assert_eq!(json(&after), json(&before), "the original is untouched");
        server.shutdown().await;
    });
}

#[test]
fn a_fork_opens_its_calls_with_the_original_s_prefix_byte_for_byte() {
    let dir = folder("forks-prefix");
    block_on!(async {
        let answers = (0..4).map(|_| answer(Some("Done."), None)).collect();
        let (server, seen) = setup(answers).build(&dir).await;
        // The original starts in Plan, which its prefix tells the model, then goes on in Auto,
        // which a note tells it instead: a prefix made for the fork now would not say Plan.
        let original = start(&server, false).await;
        turn_in(&server, original, "plan the release", Permissions::Plan).await;
        turn_in(&server, original, "go ahead", Permissions::Auto).await;
        let forked = fork(&server, original, None).await.unwrap();
        turn(&server, forked.id, "go ahead, but first the tests")
            .await
            .unwrap();
        let fresh = start(&server, false).await;
        turn_in(&server, fresh, "go ahead", Permissions::Auto).await;

        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 4);
        assert_eq!(opening(&requests[1]), opening(&requests[0]), "invariant 27");
        assert_eq!(
            opening(&requests[2]),
            opening(&requests[0]),
            "the fork's first call opens as the original's did"
        );
        assert_ne!(
            opening(&requests[3]),
            opening(&requests[0]),
            "a prefix made now differs, so the fork's is the original's, kept"
        );
        server.shutdown().await;
    });
}

#[test]
fn a_fork_says_plainly_that_files_are_not_rewound_and_where_it_works() {
    block_on!(async {
        let answers = (0..2).map(|_| answer(Some("Done."), None)).collect();
        let (server, _seen, _repo, _dir) = setup(answers)
            .build_in_repository("forks-files", InMemoryLog::new())
            .await;
        let in_place = start(&server, false).await;
        turn(&server, in_place, "tidy the readme").await.unwrap();
        let forked = fork(&server, in_place, None).await.unwrap();
        assert_eq!(
            opened(&server, forked.id).await,
            opened(&server, in_place).await
        );
        assert_eq!(
            forked_record(&server, forked.id).await,
            (
                ForkFiles::Same,
                "Forked here from an earlier thread. Files aren't rewound: this thread works \
                 in the same folder, with them as they are now."
                    .to_owned()
            )
        );

        // Two threads never share a copy of their own: a worktree's fork gets one of its own.
        let copied = start(&server, true).await;
        turn(&server, copied, "tidy the changelog").await.unwrap();
        let forked = fork(&server, copied, None).await.unwrap();
        let (place, root) = opened(&server, forked.id).await;
        assert!(matches!(place, Opened::Worktree { .. }), "{place:?}");
        assert_ne!(root, opened(&server, copied).await.1);
        assert_eq!(
            forked_record(&server, forked.id).await,
            (
                ForkFiles::Other,
                "Forked here from an earlier thread. Files aren't rewound: this thread works \
                 in another folder, without the earlier thread's changes."
                    .to_owned()
            )
        );
        server.shutdown().await;
    });
}

#[test]
fn what_a_fork_s_copies_name_outlives_the_original() {
    let dir = folder("forks-blobs");
    block_on!(async {
        let setup = setup(vec![answer(Some("One."), None)]);
        let blobs = setup.blobs.clone();
        let (server, _seen) = setup.build(&dir).await;
        let original = start(&server, false).await;
        turn(&server, original, "first").await.unwrap();
        let forked = fork(&server, original, None).await.unwrap();
        server
            .request(Request::ThreadDelete { thread: original })
            .await
            .unwrap();
        blobs.sweep(scope().org()).await.unwrap();
        let config = events(&server, forked.id)
            .await
            .into_iter()
            .find_map(|raw| match decode(raw).unwrap().body {
                Body::TurnStarted(started) => Some(started.config.sha256),
                _ => None,
            })
            .unwrap();
        let kept = blobs.get(&scope(), &config).await.unwrap();
        assert!(
            kept.is_some(),
            "the fork's copy of a turn still finds its configuration"
        );
        server.shutdown().await;
    });
}

#[test]
fn a_fork_point_within_a_turn_or_past_the_end_is_refused_and_nothing_is_made() {
    let dir = folder("forks-refused");
    block_on!(async {
        let (server, _seen) = setup(vec![answer(Some("One."), None)]).build(&dir).await;
        let original = start(&server, false).await;
        turn(&server, original, "first").await.unwrap();
        let within = seqs_of(&server, original, "turn.started").await[0];
        let past = Seq::new(within.get() + 1_000).unwrap();
        for at in [within, past] {
            let refused = fork(&server, original, Some(at)).await.unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS);
            assert_eq!(
                &*refused.message,
                "a thread is forked only between its turns, and that point is within one or \
                 past its end"
            );
        }
        let listed = Request::ThreadList {
            project: Some(scope().project()),
            limit: None,
            show: None,
            cursor: None,
        };
        let Reply::Threads { threads, .. } = server.request(listed).await.unwrap() else {
            panic!("threads");
        };
        assert_eq!(threads.len(), 1, "no fork was made");
        server.shutdown().await;
    });
}

/// The ids of `thread`'s tool calls, in order.
async fn call_ids(server: &Tested, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::ToolCallRequested(requested) => Some(requested.call_id.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_fork_numbers_its_calls_as_the_original_would_have() {
    let dir = folder("forks-calls");
    block_on!(async {
        let look = || answer(None, Some(("glob", json!({"pattern": "*.md"}))));
        let answers = vec![
            look(),
            answer(Some("None."), None),
            look(),
            answer(Some("Still none."), None),
            look(),
            answer(Some("None again."), None),
        ];
        let (server, _seen) = setup(answers).build(&dir).await;
        let original = start(&server, false).await;
        turn(&server, original, "any notes?").await.unwrap();
        turn(&server, original, "look again").await.unwrap();
        let first_ended = seqs_of(&server, original, "turn.ended").await[0];
        let forked = fork(&server, original, Some(first_ended)).await.unwrap();
        turn(&server, forked.id, "look once more").await.unwrap();
        let theirs = call_ids(&server, original).await;
        assert_eq!(theirs.len(), 2);
        assert_ne!(theirs[0], theirs[1]);
        assert_eq!(
            call_ids(&server, forked.id).await,
            theirs,
            "the copy's, then the id the original's next call had"
        );
        server.shutdown().await;
    });
}

/// A command that never ends by itself, so only a stop ends it.
const SERVES: &str = "tail -f /dev/null";

#[test]
fn a_fork_keeps_none_of_the_original_s_commands_and_numbers_its_own_after_them() {
    let dir = folder("forks-commands");
    block_on!(async {
        let serve = || {
            let started = json!({"command": SERVES, "background": true});
            answer(None, Some(("bash", started)))
        };
        let answers = vec![
            serve(),
            answer(Some("It runs."), None),
            serve(),
            answer(Some("It runs here too."), None),
        ];
        let (server, _seen) = setup(answers).build(&dir).await;
        let original = start(&server, false).await;
        turn(&server, original, "start the server").await.unwrap();
        let forked = fork(&server, original, None).await.unwrap();
        assert_eq!(
            listed(&server, forked.id).await,
            [],
            "the original's runs for it alone"
        );
        turn(&server, forked.id, "start it again").await.unwrap();
        let results: Vec<String> = events(&server, forked.id)
            .await
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
                _ => None,
            })
            .collect();
        assert!(
            results[0].contains("Started command 1 in the background"),
            "{results:?}"
        );
        assert!(
            results[1].contains("Started command 2 in the background"),
            "after the copy's command 1: {results:?}"
        );
        for thread in [original, forked.id] {
            let kept = listed(&server, thread).await;
            assert_eq!(kept.len(), 1, "{kept:?}");
            let stop = Request::CommandStop {
                thread,
                command: kept[0].command.id,
            };
            server.request(stop).await.unwrap();
        }
        server.shutdown().await;
    });
}
