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

    /// The end of the turn before, if one runs, then a turn the person starts with `prompt`.
    fn turn(&mut self, prompt: &str) -> &mut Self {
        self.end();
        let config = r#"{"sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08","bytes":2,"media_type":"application/json"}"#;
        let started = format!(r#"{{"model":{{"provider":"p","name":"m"}},"config":{config}}}"#);
        self.push("turn.started", &started);
        self.message("user", prompt)
    }

    /// The end of the turn that runs, if one does.
    fn end(&mut self) -> &mut Self {
        let running = self.events.iter().rev().find_map(|event| {
            matches!(event.kind.as_str(), "turn.started" | "turn.ended")
                .then(|| event.kind.as_str() == "turn.started")
        });
        if running == Some(true) {
            self.push("turn.ended", r#"{"outcome":"completed"}"#);
        }
        self
    }

    fn message(&mut self, role: &str, text: &str) -> &mut Self {
        let data = format!(r#"{{"role":"{role}","parts":[{{"type":"text","text":"{text}"}}]}}"#);
        self.push("message.added", &data)
    }

    fn reply(&mut self, text: &str) -> &mut Self {
        self.message("assistant", text)
    }

    /// A call of `tool` that finished, with `structured` when it recorded something.
    fn call(
        &mut self,
        tool: &str,
        is_error: bool,
        structured: Option<serde_json::Value>,
    ) -> &mut Self {
        let call_id = format!("call_{}", self.events.len());
        let requested = serde_json::json!({
            "call_id": call_id,
            "tool": tool,
            "arguments": {"json": {}},
        });
        self.push("tool.call.requested", &requested.to_string());
        let mut completed = serde_json::json!({
            "call_id": call_id,
            "is_error": is_error,
            "output": [{"type": "text", "text": "what the model saw"}],
        });
        if let Some(structured) = structured {
            completed["structured"] = structured;
        }
        self.push("tool.call.completed", &completed.to_string())
    }

    /// A `web_fetch` that read the page at `url`, titled `title`.
    fn read(&mut self, url: &str, title: Option<&str>) -> &mut Self {
        let page = page(url, title);
        self.call(WebPage::TOOL, false, Some(page.structured()))
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
        .call("mcp__search__web_search_exa", false, None)
        .read("https://tokio.rs/blog", Some("Tokio blog"))
        .read("https://docs.rs/tokio", None)
        .read("https://tokio.rs/blog", Some("Tokio blog"))
        .reply("Here is what changed.")
        .end();
    let sources = log.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].reply, Seq::new(12).unwrap());
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
    let claimed = page("https://example.com/b", None).structured();
    log.turn("read https://example.com/a")
        .read("https://example.com/a", Some("A"))
        .call(WebPage::TOOL, true, None)
        // Even one that says it read a page, if it failed.
        .call(WebPage::TOOL, true, Some(claimed.clone()))
        // Nor does another tool's record make a page read, whatever it says.
        .call("mcp__docs__fetch", false, Some(claimed))
        .reply("It says hello.")
        .end();
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
        .turn("fourth")
        .end();
    let sources = log.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].reply, Seq::new(5).unwrap());
    assert_eq!(sources[0].pages, [page("https://one.example/", None)]);
}

#[test]
fn a_turn_still_running_and_a_reply_with_no_words_have_none() {
    let mut log = Log::default();
    log.turn("read it")
        .read("https://example.com/", None)
        .reply("  ")
        .turn("and this")
        .read("https://example.com/this", None)
        // Its reply so far, which may not be its last.
        .reply("Looking.");
    assert!(log.sources().is_empty());
}
