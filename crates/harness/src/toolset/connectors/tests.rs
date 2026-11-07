use std::sync::Arc;

use glave_event::CallId;
use glave_ports::{Effect, InMemoryTools, ToolInfo, ToolSource, block_on};

use super::*;
use crate::subagent::SubagentType;

fn info(name: &str, effect: Effect) -> (ToolInfo, ToolOutput) {
    let spec = ToolSpec {
        name: name.to_owned(),
        description: format!("Does {name}. More words about it follow here."),
        parameters: json!({"type": "object", "properties": {
            "number": {"type": "integer", "description": "Its number"}
        }, "required": ["number"]}),
    };
    let info = ToolInfo {
        spec,
        effect,
        parallel: true,
    };
    (info, ToolOutput::text(false, "done"))
}

/// A connected source with `count` tools of `connector`, the first of which only reads.
fn connector(connector: &str, count: usize) -> Arc<dyn ToolSource> {
    let tools = (0..count).map(|n| {
        let effect = if n == 0 { Effect::Reads } else { Effect::Runs };
        info(&format!("mcp__{connector}__tool_{n}"), effect)
    });
    Arc::new(InMemoryTools::new(tools.collect()).as_connected())
}

/// glave's own `read`, then `sources`, gathered and framed for a window of `window` tokens.
fn set_of(sources: Vec<Arc<dyn ToolSource>>, window: u64) -> ToolSet {
    let own = Arc::new(InMemoryTools::new(vec![info("read", Effect::Reads)]));
    let mut all: Vec<Arc<dyn ToolSource>> = vec![own];
    all.extend(sources);
    let mut set = block_on!(ToolSet::gather(&all, "system"));
    set.frame(window);
    set
}

fn names(set: &ToolSet) -> Vec<&str> {
    set.names()
}

fn call(tool: &str, arguments: &Value) -> ToolCall {
    ToolCall {
        call_id: CallId::new("call_1").unwrap(),
        tool: tool.to_owned(),
        arguments: Arguments::Json(arguments.as_object().cloned().unwrap()),
    }
}

fn text(output: &ToolOutput) -> String {
    match output.parts.first() {
        Some(glave_event::Part::Text { text }) => text.clone(),
        _ => String::new(),
    }
}

/// The call `route` runs, by name and arguments.
fn runs(set: &ToolSet, routed: &ToolCall) -> (String, Value) {
    match set.route(routed) {
        Route::Run(call) => {
            let Arguments::Json(map) = call.arguments else {
                panic!("not JSON")
            };
            (call.tool, Value::Object(map))
        }
        other => panic!("not run: {other:?}"),
    }
}

#[test]
fn connected_tools_that_fit_are_listed_in_full_and_calls_run_as_they_are() {
    let set = set_of(vec![connector("github", 3)], 128_000);
    assert_eq!(
        names(&set),
        [
            "read",
            "mcp__github__tool_0",
            "mcp__github__tool_1",
            "mcp__github__tool_2"
        ]
    );
    let direct = call("mcp__github__tool_1", &json!({"tool": "x"}));
    assert_eq!(
        set.route(&direct),
        Route::Run(direct.clone()),
        "no checks either"
    );
}

#[test]
fn past_a_tenth_of_the_window_each_connector_is_one_tool_after_the_search() {
    let set = set_of(vec![connector("github", 30), connector("jira", 2)], 4_000);
    assert_eq!(
        names(&set),
        ["read", TOOL_SEARCH, "mcp__github", "mcp__jira"]
    );
    let github = &set.specs[2].description;
    assert!(
        github.starts_with("Calls a tool of the github connector (30 in all)"),
        "{github}"
    );
    assert!(
        github.contains("\n- tool_0: Does mcp__github__tool_0\n"),
        "{github}"
    );
    assert!(github.contains("- tool_19: "), "{github}");
    assert!(!github.contains("tool_20:"), "twenty listed: {github}");
    assert!(
        github.ends_with("- and 10 more, which tool_search finds"),
        "{github}"
    );
    assert!(
        !set.specs[3].description.contains("more"),
        "{}",
        set.specs[3].description
    );
}

#[test]
fn more_than_a_hundred_tools_are_listed_by_connector_however_small_they_are() {
    let set = set_of(vec![connector("a", 60), connector("b", 60)], 10_000_000);
    assert_eq!(names(&set), ["read", TOOL_SEARCH, "mcp__a", "mcp__b"]);
}

#[test]
fn a_connector_s_own_words_about_itself_open_its_listing() {
    struct Said(Arc<dyn ToolSource>);
    impl ToolSource for Said {
        fn tools(
            &self,
        ) -> glave_ports::BoxFuture<'_, Result<Vec<ToolInfo>, glave_ports::ToolError>> {
            self.0.tools()
        }
        fn connected(&self) -> bool {
            true
        }
        fn instructions(&self) -> Option<String> {
            Some("Issues and pull requests on GitHub. Use it for code review.".into())
        }
        fn call<'a>(
            &'a self,
            name: &'a str,
            arguments: &'a Arguments,
            cancel: &'a dyn glave_ports::Cancellation,
        ) -> glave_ports::BoxFuture<'a, ToolOutput> {
            self.0.call(name, arguments, cancel)
        }
    }
    let set = set_of(vec![Arc::new(Said(connector("github", 200)))], 128_000);
    let github = &set.specs[2].description;
    assert!(
        github.contains("\ngithub: Issues and pull requests on GitHub\n"),
        "{github}"
    );
}

#[test]
fn a_connector_s_tool_calls_the_tool_it_names_with_its_arguments_however_they_come() {
    let set = set_of(vec![connector("github", 200)], 128_000);
    let wanted = ("mcp__github__tool_7".to_owned(), json!({"number": 4120}));
    for given in [
        json!({"tool": "tool_7", "arguments": {"number": 4120}}),
        json!({"tool": "mcp__github__tool_7", "arguments": {"number": 4120}}),
        json!({"tool": "tool_7", "arguments": "{\"number\": 4120}"}),
        json!({"tool": "tool_7", "number": 4120}),
    ] {
        assert_eq!(runs(&set, &call("mcp__github", &given)), wanted, "{given}");
    }
    // Called by its own name, with its own arguments or a connector's call's.
    assert_eq!(
        runs(&set, &call("mcp__github__tool_7", &json!({"number": 4120}))),
        wanted
    );
    let shaped = json!({"tool": "tool_7", "arguments": {"number": 4120}});
    assert_eq!(runs(&set, &call("mcp__github__tool_7", &shaped)), wanted);
    let inside = json!({"arguments": "{\"number\": 4120}"});
    assert_eq!(runs(&set, &call("mcp__github__tool_7", &inside)), wanted);
}

#[test]
fn arguments_that_do_not_fit_run_nothing_and_show_the_parameters() {
    let set = set_of(vec![connector("github", 200)], 128_000);
    for given in [
        call(
            "mcp__github",
            &json!({"tool": "tool_7", "arguments": {"issue": 4120}}),
        ),
        call("mcp__github__tool_7", &json!({"number": "4120"})),
    ] {
        let Route::Refused(output) = set.route(&given) else {
            panic!("ran: {given:?}");
        };
        let said = text(&output);
        assert!(output.is_error && said.starts_with("Nothing ran"), "{said}");
        assert!(
            said.contains("\"number\":{\"description\":\"Its number\""),
            "{said}"
        );
    }
}

#[test]
fn a_search_through_a_connector_s_tool_searches_it_unless_it_has_a_tool_called_so() {
    let mut sources = vec![connector("github", 200)];
    sources.push(Arc::new(
        InMemoryTools::new(vec![info("mcp__notion__search", Effect::Reads)]).as_connected(),
    ));
    let set = set_of(sources, 128_000);
    for tool in ["tool_search", "search", "TOOL_SEARCH"] {
        let given = call(
            "mcp__github",
            &json!({"tool": tool, "arguments": {"query": "tool 7"}}),
        );
        let searched = Route::Search {
            query: Some("tool 7".into()),
            connector: Some("github".into()),
        };
        assert_eq!(set.route(&given), searched, "{tool}");
    }
    // Named as if the search were one of the connector's tools.
    let within = call("mcp__github__tool_search", &json!({"query": "tool 7"}));
    let searched = Route::Search {
        query: Some("tool 7".into()),
        connector: Some("github".into()),
    };
    assert_eq!(set.route(&within), searched);
    let notion = call(
        "mcp__notion",
        &json!({"tool": "search", "arguments": {"number": 1}}),
    );
    assert_eq!(runs(&set, &notion).0, "mcp__notion__search");
}

#[test]
fn a_tool_its_connector_does_not_have_is_told_to_search() {
    let set = set_of(vec![connector("github", 200)], 128_000);
    let Route::Refused(output) = set.route(&call("mcp__github", &json!({"tool": "get_issue"})))
    else {
        panic!("ran");
    };
    let said = text(&output);
    assert!(
        said.contains("github has no tool called \"get_issue\""),
        "{said}"
    );
    assert!(
        said.contains("tool_search (connector \"github\")"),
        "{said}"
    );
}

#[test]
fn a_search_finds_full_definitions_in_one_connector_or_all() {
    let set = set_of(
        vec![connector("github", 200), connector("jira", 200)],
        128_000,
    );
    let found = text(&set.search(Some("tool_7"), None));
    assert!(found.contains("\"connector\":\"mcp__github\""), "{found}");
    assert!(found.contains("\"connector\":\"mcp__jira\""), "{found}");
    assert!(found.contains("\"parameters\":{"), "{found}");
    let scoped = text(&set.search(Some("tool_7"), Some("mcp__jira")));
    assert!(!scoped.contains("mcp__github"), "{scoped}");
    assert!(
        scoped.starts_with("Call one through its connector's tool"),
        "{scoped}"
    );
    let unknown = set.search(Some("x"), Some("linear"));
    assert!(
        text(&unknown).contains("The connectors are github, jira"),
        "{}",
        text(&unknown)
    );
    assert!(set.search(None, None).is_error);
}

#[test]
fn listed_as_a_prefix_lists_them_connected_tools_are_called_through_their_connector() {
    let set = set_of(vec![connector("github", 200)], 128_000);
    let (system, specs) = (set.system.clone(), set.specs.clone());
    // The connection off: its tool is listed still, and refused.
    let off = set_of(Vec::new(), 128_000).listed_as(system.clone(), specs.clone());
    let Route::Refused(output) = off.route(&call("mcp__github", &json!({"tool": "tool_7"}))) else {
        panic!("ran");
    };
    assert!(
        text(&output).contains("is not available now"),
        "{}",
        text(&output)
    );
    assert!(text(&off.search(Some("tool"), Some("github"))).contains("not available"));
    // On again, with one more tool, which the search finds.
    let on = set_of(vec![connector("github", 201)], 128_000).listed_as(system, specs);
    assert!(on.get("mcp__github__tool_200").is_some());
    assert!(text(&on.search(Some("tool_200"), None)).contains("\"tool\":\"tool_200\""));
}

#[test]
fn an_explorer_keeps_the_search_and_the_connectors_with_tools_that_read() {
    let set = set_of(vec![connector("github", 200)], 128_000);
    let child = set.for_child(SubagentType::Explore);
    assert_eq!(names(&child), ["read", TOOL_SEARCH, "mcp__github"]);
    let github = &child.specs[2].description;
    assert!(
        github.starts_with("Calls a tool of the github connector (1 in all)"),
        "{github}"
    );
    let found = text(&child.search(Some("tool"), None));
    assert!(found.contains("\"tool\":\"tool_0\""), "{found}");
    assert!(
        !found.contains("tool_1\""),
        "only the one that reads: {found}"
    );
    let general = set.for_child(SubagentType::General);
    assert_eq!(names(&general), names(&set));
}
