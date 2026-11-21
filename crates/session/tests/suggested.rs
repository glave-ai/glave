//! A connector of glave's catalog that a search finds for a person who may connect it and has
//! not (`specs/session.md`, "Connectors"): the thread records it as a fact a client draws, its
//! sentence the server's own, recorded in English and sent in the caller's language.

use glave_harness::OwnTools;
use glave_session::protocol::Request;
use glave_session::{Catalog, Connector};
use serde_json::{Value, json};

use crate::support::web::read_in_two;
use crate::support::*;

/// What glave tells the person of Linear, in English.
const TOLD: &str = "Linear isn't connected. Connect it to let me use it for you. You sign in on Linear's own page.";

/// Each suggestion among `messages`, each a `thread/event` notification or an event: the
/// connector, what it was written from, and the sentence.
fn suggested(messages: &[Value]) -> Vec<(String, Value, String)> {
    messages
        .iter()
        .map(|message| {
            message
                .get("params")
                .map_or(message, |params| &params["event"])
        })
        .filter(|event| event["type"] == "connector.suggested")
        .map(|event| {
            let data = &event["data"];
            let connector = data["connector"].as_str().unwrap().to_owned();
            let told = data["told"].as_str().unwrap().to_owned();
            (connector, data["suggests"].clone(), told)
        })
        .collect()
}

#[test]
fn a_connector_a_search_suggests_is_recorded_in_english_and_told_in_the_callers_language() {
    block_on!(async {
        let dir = folder("suggested");
        // Nothing connected: every connector of the catalog is one to connect.
        let catalog = Catalog::built_in().unwrap().connectors;
        let connectable = catalog.iter().map(Connector::connectable).collect();
        let searching = vec![
            answer(
                None,
                Some(("tool_search", json!({"query": "create linear issue"}))),
            ),
            answer(Some("Connect Linear and I'll make it."), None),
        ];
        let setup = Setup {
            own: Box::new(OwnTools::default().with_connectable(connectable)),
            ..Setup::new(searching)
        };
        let (served, mut reading, mut plain, thread) = read_in_two(setup.build(&dir).await.0).await;
        let (message, _) = reading.message(&turn_start(thread, "make a Linear issue", None));
        let streamed = suggested(&reading.post(&message).await.messages);
        let [(connector, suggests, told)] = streamed.as_slice() else {
            panic!("one suggestion: {streamed:?}")
        };
        assert_eq!(connector, "linear");
        assert_eq!(
            *suggests,
            json!({"says": "suggest-sign-in", "name": "Linear"})
        );
        assert!(
            told.starts_with("[Linear ") && told != TOLD,
            "in the reader's language, the vendor's name as it is: {told}"
        );
        // What was recorded is English, as a client reading English is sent it.
        let history = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let page = plain.call(&history).await;
        let english = suggested(page["result"]["events"].as_array().unwrap());
        assert_eq!(english.len(), 1, "{english:?}");
        assert_eq!(english[0].2, TOLD);
        served.close().await;
    });
}
