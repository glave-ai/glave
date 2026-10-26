use super::*;

/// A thread's log, written one event after another.
#[derive(Default)]
struct Log {
    events: Vec<RawEvent>,
}

impl Log {
    fn push(&mut self, kind: &str, data: &str) -> &mut Self {
        let seq = self.events.len() + 1;
        let line = format!(
            r#"{{"glave":1,"id":"0192d6a0-0000-7000-8000-{seq:012}","thread":"0192d6a0-0000-7000-8000-000000000000","seq":{seq},"at":"2026-10-25T12:00:00Z","type":"{kind}","v":1,"data":{data}}}"#
        );
        self.events.push(RawEvent::from_json(&line).unwrap());
        self
    }

    fn turn(&mut self, prompt: &str) -> &mut Self {
        let config = r#"{"sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08","bytes":2,"media_type":"application/json"}"#;
        let started = format!(r#"{{"model":{{"provider":"p","name":"m"}},"config":{config}}}"#);
        self.push("turn.started", &started);
        self.message("user", prompt)
    }

    fn message(&mut self, role: &str, text: &str) -> &mut Self {
        let data = format!(r#"{{"role":"{role}","parts":[{{"type":"text","text":"{text}"}}]}}"#);
        self.push("message.added", &data)
    }

    fn reply(&mut self, text: &str) -> &mut Self {
        self.message("assistant", text)
    }

    /// A call that finished, with `structured` when it recorded something.
    fn done(&mut self, is_error: bool, structured: Option<serde_json::Value>) -> &mut Self {
        let mut data = serde_json::json!({
            "call_id": "c",
            "is_error": is_error,
            "output": [{"type": "text", "text": "what the model saw"}],
        });
        if let Some(structured) = structured {
            data["structured"] = structured;
        }
        self.push("tool.call.completed", &data.to_string())
    }

    /// A `web_fetch` that read the page at `url`, titled `title`.
    fn read(&mut self, url: &str, title: Option<&str>) -> &mut Self {
        let page = page(url, title);
        self.done(false, Some(page.structured()))
    }

    fn sources(&self) -> Vec<ReplySources> {
        let mut reading = Reading::default();
        for event in &self.events {
            reading.read(event.clone());
        }
        reading.done()
    }
}

fn page(url: &str, title: Option<&str>) -> WebPage {
    WebPage {
        url: url.into(),
        title: title.map(str::to_owned),
    }
}

#[test]
fn a_reply_s_sources_are_the_pages_its_turn_read_before_it_each_once() {
    let mut log = Log::default();
    log.turn("what changed in tokio?")
        .reply("Let me search.")
        // A search's results the agent only skimmed: no page read.
        .done(false, None)
        .read("https://tokio.rs/blog", Some("Tokio blog"))
        .read("https://docs.rs/tokio", None)
        .read("https://tokio.rs/blog", Some("Tokio blog"))
        .reply("Here is what changed.");
    let sources = log.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].reply, Seq::new(8).unwrap());
    assert_eq!(&*sources[0].sentence, "Sources: 2 pages");
    assert_eq!(
        sources[0].pages,
        [
            page("https://tokio.rs/blog", Some("Tokio blog")),
            page("https://docs.rs/tokio", None),
        ]
    );
}

#[test]
fn a_page_read_with_no_search_is_a_source_and_one_that_failed_is_none() {
    let mut log = Log::default();
    log.turn("read https://example.com/a")
        .read("https://example.com/a", Some("A"))
        .done(true, None)
        // Even one that says it read a page, if it failed.
        .done(true, Some(page("https://example.com/b", None).structured()))
        .reply("It says hello.");
    let sources = log.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].pages, [page("https://example.com/a", Some("A"))]);
    assert_eq!(&*sources[0].sentence, "Sources: 1 page");
}

#[test]
fn each_turn_s_pages_go_under_its_own_last_reply_and_never_one_before_them() {
    let mut log = Log::default();
    log.turn("first")
        .read("https://one.example/", None)
        .reply("One.")
        .turn("second")
        .reply("No reading.")
        .turn("third")
        .reply("Reading now.")
        .read("https://three.example/", None)
        // The turn ended reading, with no reply after: nothing drew on the page.
        .turn("fourth");
    let sources = log.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].reply, Seq::new(4).unwrap());
    assert_eq!(sources[0].pages, [page("https://one.example/", None)]);
}

#[test]
fn a_reply_with_no_words_is_none() {
    let mut log = Log::default();
    log.turn("read it")
        .read("https://example.com/", None)
        .reply("  ");
    assert!(log.sources().is_empty());
}
