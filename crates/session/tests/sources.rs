//! Sources (`specs/session.md`, "Sources"): what a reply drew on from the web, read from the
//! thread's log. Each page its turn read with glave's own `web_fetch` is one, with the title the
//! page gives itself; a page that failed to load is none, and a turn that read nothing has none.

use std::collections::HashMap;
use std::sync::Arc;

use glave_event::{Body, Role, WebPage, decode};
use glave_ports::settings::{Domain, WebPolicy};
use glave_session::protocol::{Reply, Request};
use glave_web::pages::{Pages, Reply as Served};
use glave_web::{Web, WebSettings};
use serde_json::json;

use crate::support::*;

/// `web_fetch`, reaching only this computer's pages.
fn web() -> Web {
    let policy = WebPolicy {
        allow: vec![Domain::parse("127.0.0.1").unwrap()],
        ..WebPolicy::default()
    };
    Web::new(WebSettings {
        policy,
        ..WebSettings::default()
    })
    .unwrap()
}

#[test]
fn a_reply_s_sources_are_the_pages_its_turn_read_and_never_one_that_failed() {
    let dir = folder("sources");
    block_on!(async {
        let page = "<title>Release notes</title><p>The codename is Juniper.</p>";
        let replies = HashMap::from([("/notes".to_owned(), Served::html(page))]);
        let pages = Pages::serve(replies).await.unwrap();
        let notes = format!("{}/notes", pages.base());
        let gone = format!("{}/gone", pages.base());
        let mut setup = Setup::new(vec![
            // A page that is not there, then one that is: no search came first.
            answer(None, Some(("web_fetch", json!({ "url": gone })))),
            answer(None, Some(("web_fetch", json!({ "url": notes })))),
            answer(Some("The codename is Juniper."), None),
            answer(Some("Nothing else to read."), None),
        ]);
        setup.settings.titles = false;
        setup.tools = vec![Arc::new(web())];
        let (server, _seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "What is the codename?")
            .await
            .unwrap();
        turn(&server, thread, "Anything else?").await.unwrap();
        let sourced = server.request(Request::ThreadSources { thread }).await;
        let answers: Vec<_> = events(&server, thread)
            .await
            .into_iter()
            .filter_map(|raw| {
                let seq = raw.seq;
                match decode(raw).unwrap().body {
                    Body::MessageAdded(added) if added.role == Role::Assistant => Some(seq),
                    _ => None,
                }
            })
            .collect();
        server.shutdown().await;
        let Ok(Reply::Sources { sources }) = sourced else {
            panic!("sources, not {sourced:?}")
        };
        assert_eq!(sources.len(), 1, "{sources:?}");
        assert_eq!(sources[0].reply, answers[0], "under the first turn's reply");
        assert_eq!(&*sources[0].sentence, "Sources: 1 page");
        let read = WebPage {
            url: notes,
            title: Some("Release notes".into()),
        };
        assert_eq!(sources[0].pages, [read]);
    });
}
