use super::*;

/// An event of `kind` at `at`, whose payload is `data`.
fn event(seq: u64, at: &str, kind: &str, data: &str) -> RawEvent {
    let line = format!(
        r#"{{"glave":1,"id":"0192d6a0-0000-7000-8000-{seq:012}","thread":"0192d6a0-0000-7000-8000-000000000000","seq":{seq},"at":"{at}","type":"{kind}","v":1,"data":{data}}}"#
    );
    RawEvent::from_json(&line).unwrap()
}

fn at(text: &str) -> At {
    At::from(text.parse::<Timestamp>().unwrap())
}

#[test]
fn this_month_begins_on_its_first_day_in_utc() {
    let now = "2026-10-25T17:55:00Z".parse().unwrap();
    assert_eq!(month_start(now).unwrap(), at("2026-10-01T00:00:00Z"));
    let first = "2026-11-01T00:00:00Z".parse().unwrap();
    assert_eq!(month_start(first).unwrap(), at("2026-11-01T00:00:00Z"));
}

#[test]
fn only_this_month_s_calls_to_a_connection_s_tools_count_each_to_its_own_server() {
    let call =
        |tool: &str| format!(r#"{{"call_id":"c","tool":"{tool}","arguments":{{"json":{{}}}}}}"#);
    let events = [
        event(
            1,
            "2026-09-30T23:59:59Z",
            ToolCallRequested::TYPE,
            &call("mcp__search__web_search_exa"),
        ),
        event(
            2,
            "2026-10-01T00:00:00Z",
            ToolCallRequested::TYPE,
            &call("mcp__search__web_search_exa"),
        ),
        event(
            3,
            "2026-10-02T09:00:00Z",
            ToolCallRequested::TYPE,
            &call("mcp__search__web_search_exa"),
        ),
        event(
            4,
            "2026-10-02T09:00:01Z",
            "tool.call.completed",
            &call("mcp__search__x"),
        ),
        // A server whose name holds another's: the longer name is the one it names.
        event(
            5,
            "2026-10-02T09:00:02Z",
            ToolCallRequested::TYPE,
            &call("mcp__search__v2__find"),
        ),
        event(
            6,
            "2026-10-02T09:00:03Z",
            ToolCallRequested::TYPE,
            &call("web_fetch"),
        ),
        event(
            7,
            "2026-10-02T09:00:04Z",
            ToolCallRequested::TYPE,
            &call("mcp__docs__read"),
        ),
        // Called through its connector's one tool, where tools are listed by connector.
        event(
            8,
            "2026-10-02T09:00:05Z",
            ToolCallRequested::TYPE,
            &call("mcp__search"),
        ),
    ];
    let prefixes = [
        (glave_mcp::exposed("search", ""), "search"),
        (glave_mcp::exposed("search__v2", ""), "search__v2"),
    ];
    let mut calls = BTreeMap::new();
    tally(&events, at("2026-10-01T00:00:00Z"), &prefixes, &mut calls);
    assert_eq!(
        calls,
        BTreeMap::from([("search".to_owned(), 3), ("search__v2".to_owned(), 1)])
    );
}

#[test]
fn a_fork_counts_its_own_calls_and_not_its_copies() {
    let call = r#"{"call_id":"c","tool":"mcp__search__find","arguments":{"json":{}}}"#;
    let forked = r#"{"from":{"thread":"0192d6a0-0000-7000-8000-000000000001","seq":2},"files":"same","told":"Forked here."}"#;
    let prefixes = [(glave_mcp::exposed("search", ""), "search")];
    let since = at("2026-10-01T00:00:00Z");
    let copies = [event(
        2,
        "2026-10-02T09:00:00Z",
        ToolCallRequested::TYPE,
        call,
    )];
    let own = [
        event(3, "2026-10-03T09:00:00Z", ThreadForked::TYPE, forked),
        event(4, "2026-10-03T09:00:01Z", ToolCallRequested::TYPE, call),
    ];
    let mut calls = BTreeMap::new();
    tally(&copies, since, &prefixes, &mut calls);
    tally(&own, since, &prefixes, &mut calls);
    assert_eq!(calls, BTreeMap::from([("search".to_owned(), 1)]));
}
