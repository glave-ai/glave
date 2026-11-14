//! Web search through the session API (`specs/session.md`, "Web search"): where the server
//! searches, it is on until the person on this computer turns it off, says which service
//! searches and how often it did this month, and their choice is kept in their settings, so a
//! server started later on the same store offers no search either. Never anyone else's to
//! choose, nor anyone's where the server searches nothing.

use glave_ports::testing::ScriptedModel;
use glave_ports::{Hit, Searched};
use glave_session::protocol::{Reply, Request, SearchInfo, code};
use serde_json::json;

use crate::support::*;

fn found() -> Searched {
    Searched {
        hits: vec![Hit {
            title: "Tokio".into(),
            url: "https://tokio.rs/".into(),
            snippet: "A runtime.".into(),
        }],
        cost: None,
    }
}

/// Web search as the server answers `request`.
async fn search<L: glave_ports::EventLog + 'static>(
    tested: &Tested<L>,
    request: Request,
) -> SearchInfo {
    match tested.request(request).await.unwrap() {
        Reply::Search { search } => search,
        other => panic!("search, not {other:?}"),
    }
}

/// Whether `search` is on, and how many searches it says it made this month, if it says.
fn on_and_counted(search: &SearchInfo) -> (bool, Option<u64>) {
    match search {
        SearchInfo::Chosen { on, usage, .. } => (*on, usage.as_ref().map(|usage| usage.calls)),
        SearchInfo::Never { .. } => panic!("the person's to choose, not {search:?}"),
    }
}

#[test]
fn web_search_is_on_until_the_person_turns_it_off_and_says_how_often_it_searched() {
    let dir = folder("search-switch");
    block_on!(async {
        let searched = || answer(None, Some(("web_search", json!({"query": "tokio"}))));
        let answers = vec![searched(), searched(), answer(Some("done"), None)];
        let log = Shared::default();
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(ScriptedModel::new(answers).searching(Ok(found())));
        let (tested, _) = setup.build_on(&dir, log.clone()).await;

        let on = search(&tested, Request::SearchGet { usage: true }).await;
        let SearchInfo::Chosen { on, detail, usage } = on else {
            panic!("the person's to choose, not {on:?}");
        };
        assert_eq!(
            (on, &*detail),
            (true, "Uses Brave Search. Your searches go to Brave Search."),
            "on by default"
        );
        let usage = usage.unwrap();
        assert_eq!(
            (usage.calls, &*usage.sentence),
            (0, "No searches this month")
        );

        let thread = start(&tested, false).await;
        turn(&tested, thread, "search twice").await.unwrap();
        let counted = search(&tested, Request::SearchGet { usage: true }).await;
        assert_eq!(on_and_counted(&counted), (true, Some(2)));
        // A client that only shows whether it is on reads no month of threads.
        let shown = search(&tested, Request::SearchGet { usage: false }).await;
        assert_eq!(on_and_counted(&shown), (true, None));

        let refused = tested
            .request_as(&elsewhere(), Request::SearchSet { on: false })
            .await;
        assert_eq!(refused.err().map(|e| e.code), Some(code::NOT_ALLOWED));
        let theirs = tested
            .request_as(&elsewhere(), Request::SearchGet { usage: false })
            .await;
        assert!(
            matches!(
                theirs,
                Ok(Reply::Search {
                    search: SearchInfo::Never { .. }
                })
            ),
            "not theirs to choose: {theirs:?}"
        );

        let off = search(&tested, Request::SearchSet { on: false }).await;
        assert_eq!(on_and_counted(&off), (false, None));

        // Kept in the person's settings: a server started later on the same store reads it, and
        // offers its threads no search.
        let script = ScriptedModel::new(vec![answer(Some("Hi."), None)]).searching(Ok(found()));
        let seen = script.seen();
        let mut next = Setup::new(Vec::new());
        next.model = Some(script);
        next.seed = 2;
        next.settings.person = tested.person();
        next.projects = Some(std::sync::Arc::clone(tested.projects()));
        let (again, _) = next.build_on(&dir, log).await;
        let thread = start(&again, false).await;
        turn(&again, thread, "hi").await.unwrap();
        let offered = seen.lock().unwrap()[0]
            .tools
            .iter()
            .any(|tool| tool.name == "web_search");
        assert!(!offered, "the person turned it off");
        let still = search(&again, Request::SearchGet { usage: false }).await;
        assert_eq!(on_and_counted(&still), (false, None));
    });
}

#[test]
fn where_the_server_searches_nothing_web_search_is_no_one_s_to_choose() {
    let dir = folder("search-none");
    block_on!(async {
        let (tested, _) = Setup::new(Vec::new()).build(&dir).await;
        let never = search(&tested, Request::SearchGet { usage: true }).await;
        let SearchInfo::Never { reason } = never else {
            panic!("not shown: {never:?}");
        };
        assert_eq!(
            &*reason,
            "web search is set up by whoever runs this server, not from here"
        );
        let refused = tested.request(Request::SearchSet { on: true }).await;
        assert_eq!(refused.err().map(|e| e.code), Some(code::NOT_AVAILABLE));
    });
}
