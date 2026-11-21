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
    let found = text(&set.search(Some("tool_7"), None).output);
    assert!(found.contains("\"connector\":\"mcp__github\""), "{found}");
    assert!(found.contains("\"connector\":\"mcp__jira\""), "{found}");
    assert!(found.contains("\"parameters\":{"), "{found}");
    let scoped = text(&set.search(Some("tool_7"), Some("mcp__jira")).output);
    assert!(!scoped.contains("mcp__github"), "{scoped}");
    assert!(
        scoped.starts_with("Call one through its connector's tool"),
        "{scoped}"
    );
    let unknown = set.search(Some("x"), Some("linear")).output;
    assert!(
        text(&unknown).contains("The connectors are github, jira"),
        "{}",
        text(&unknown)
    );
    assert!(set.search(None, None).output.is_error);
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
    assert!(text(&off.search(Some("tool"), Some("github")).output).contains("not available"));
    // On again, with one more tool, which the search finds.
    let on = set_of(vec![connector("github", 201)], 128_000).listed_as(system, specs);
    assert!(on.get("mcp__github__tool_200").is_some());
    assert!(text(&on.search(Some("tool_200"), None).output).contains("\"tool\":\"tool_200\""));
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
    let found = text(&child.search(Some("tool"), None).output);
    assert!(found.contains("\"tool\":\"tool_0\""), "{found}");
    assert!(
        !found.contains("tool_1\""),
        "only the one that reads: {found}"
    );
    let general = set.for_child(SubagentType::General);
    assert_eq!(names(&general), names(&set));
}

#[test]
fn a_subagent_never_gets_schedule() {
    let mut set = set_of(Vec::new(), 128_000);
    set.offer_schedule();
    assert_eq!(names(&set), ["read", "schedule"]);
    let child = set.for_child(SubagentType::General);
    assert_eq!(
        names(&child),
        ["read"],
        "only a chat sets up repeating work"
    );
}

/// A connector of glave's catalog the person may connect, `name`, reaching `description`;
/// DeepWiki asks no one to sign in, and Linear's, Make's, Close's and Railway's names are plain
/// words.
fn connectable(name: &str, description: &str, tag: &str) -> Connectable {
    Connectable {
        id: name.to_ascii_lowercase(),
        name: name.to_owned(),
        description: description.to_owned(),
        tags: vec![tag.to_owned()],
        signs_in: name != "DeepWiki",
        plain_word: ["Linear", "Make", "Close", "Railway"].contains(&name),
    }
}

/// `sources` gathered as [`set_of`] does, with what the person may connect, then framed for a
/// window of `window` tokens.
fn suggesting(sources: Vec<Arc<dyn ToolSource>>, window: u64) -> ToolSet {
    // Listed in full while no catalog is offered.
    let mut set = set_of(sources, u64::MAX);
    set.offered = Some(vec![
        connectable("Linear", "Issues, projects and cycles.", "Issues and bugs"),
        connectable("Stripe", "Payments and subscriptions.", "Payments"),
        connectable(
            "DeepWiki",
            "Documentation of public repositories.",
            "Documentation",
        ),
        connectable("Make", "Your scenarios, run when you ask.", "Automation"),
        connectable("Close", "Leads, opportunities and calls.", "Sales and CRM"),
        connectable("Attio", "Records, lists and notes.", "Sales and CRM"),
        connectable("Railway", "Projects, services and deploys.", "Hosting"),
        connectable("Notion", "Pages, databases and search.", "Notes"),
        connectable("Guru", "Cards of what your team knows.", "Team knowledge"),
    ]);
    set.frame(window);
    set
}

fn ids(searched: &Searched) -> Vec<&str> {
    (searched.suggested.iter())
        .map(|it| it.id.as_str())
        .collect()
}

#[test]
fn services_the_person_may_connect_are_found_where_no_connected_tool_is() {
    let set = suggesting(vec![connector("github", 3)], 128_000);
    // Listed in full, the search last, to find them.
    assert_eq!(names(&set).first(), Some(&"read"));
    assert_eq!(names(&set).last(), Some(&TOOL_SEARCH));
    let asked = call(TOOL_SEARCH, &json!({"query": "track issues"}));
    let Route::Search { query, connector } = set.route(&asked) else {
        panic!("not a search");
    };
    let found = set.search(query.as_deref(), connector.as_deref());
    assert_eq!(ids(&found), ["linear"]);
    let said = text(&found.output);
    assert!(
        said.starts_with("No connected tool matches \"track issues\".\n"),
        "{said}"
    );
    assert!(
        said.contains(
            "Linear isn't connected; the person can connect it. It reaches: Issues, projects and \
             cycles."
        ),
        "{said}"
    );
    assert!(!found.output.is_error);
    // A connected tool found suggests none the search does not name; one named by `connector`
    // is suggested whatever the words.
    assert_eq!(set.search(Some("tool_1"), None).suggested, []);
    assert_eq!(ids(&set.search(Some("x"), Some("stripe"))), ["stripe"]);
    assert_eq!(set.search(Some("kubernetes"), None).suggested, []);
    // A connected tool listed in full still runs as it is, unchecked.
    let direct = call("mcp__github__tool_1", &json!({"tool": "x"}));
    assert_eq!(set.route(&direct), Route::Run(direct.clone()));
    // A subagent suggests nothing, and has no search where the tools fit.
    let child = set.for_child(SubagentType::General);
    assert!(!names(&child).contains(&TOOL_SEARCH));
}

#[test]
fn where_each_connector_is_one_tool_the_search_says_it_finds_services_to_connect_too() {
    let set = suggesting(vec![connector("github", 30)], 4_000);
    assert_eq!(names(&set), ["read", TOOL_SEARCH, "mcp__github"]);
    let search = &set.specs[1].description;
    assert!(search.ends_with(suggest::ALSO), "{search}");
    let plain = set_of(vec![connector("github", 30)], 4_000);
    assert!(!plain.specs[1].description.contains(suggest::ALSO));
    let found = set.search(Some("documentation of repositories"), None);
    assert_eq!(ids(&found), ["deepwiki"]);
}

#[test]
fn a_suggestion_tells_the_person_how_it_is_connected_in_one_plain_sentence() {
    let call = CallId::new("call_3").unwrap();
    let signs_in = suggestion(&call, &connectable("Linear", "Issues.", "Issues and bugs"));
    assert_eq!(signs_in.connector, "linear");
    assert_eq!(
        signs_in.told,
        "Linear isn't connected. Connect it to let me use it for you. You sign in on Linear's \
         own page."
    );
    let at_once = suggestion(&call, &connectable("DeepWiki", "Docs.", "Documentation"));
    assert_eq!(
        at_once.told,
        "DeepWiki isn't connected. Connect it to let me use it for you."
    );
}

#[test]
fn a_name_is_one_word_and_a_short_word_finds_no_service() {
    let catalog = [
        connectable(
            "PostHog",
            "Product analytics and feature flags.",
            "Analytics",
        ),
        connectable("Todoist", "Tasks and projects.", "To-do lists"),
        connectable("Stripe", "Payments, refunds and subscriptions.", "Payments"),
    ];
    let found = |query: &str| -> Vec<String> {
        let found = suggest::matching(&catalog, query, false);
        found.iter().map(|it| it.id.clone()).collect()
    };
    assert_eq!(found("post to slack"), Vec::<String>::new());
    assert_eq!(found("refund a payment"), ["stripe"]);
    assert_eq!(found("PostHog"), ["posthog"]);
}

/// A connected source of some of GitHub's tools: two about issues, one that searches, one
/// that runs a workflow.
fn github() -> Arc<dyn ToolSource> {
    let tools = [
        "issue_write",
        "issue_read",
        "pull_request_read",
        "branch_list",
        "file_read",
        "repository_search",
        "workflow_run",
    ];
    let tools = tools.map(|tool| info(&format!("mcp__github__{tool}"), Effect::Runs));
    Arc::new(InMemoryTools::new(tools.into()).as_connected())
}

#[test]
fn a_service_the_search_names_is_suggested_whatever_connected_tools_it_finds() {
    let set = suggesting(vec![github()], 128_000);
    let found = set.search(Some("make a Linear issue"), None);
    let said = text(&found.output);
    assert!(said.contains("\"tool\":\"issue_write\""), "{said}");
    assert_eq!(
        ids(&found),
        ["linear"],
        "GitHub's issues, and Linear named, not Make"
    );
    assert!(
        said.contains("Linear isn't connected; the person can connect it."),
        "{said}"
    );
    let named = |query: &str| ids(&set.search(Some(query), None)).join(" ");
    assert_eq!(named("use stripe to charge a card"), "stripe");
    // A plain word written as a plain word names nothing.
    assert_eq!(named("make the tests run"), "");
    assert_eq!(named("linear search over the array"), "");
}

#[test]
fn a_name_that_is_a_plain_word_names_its_service_only_as_the_catalog_spells_it() {
    let set = suggesting(Vec::new(), 128_000);
    let named = |query: &str| ids(&set.search(Some(query), None)).join(" ");
    assert_eq!(named("Make a scenario that posts to my sheet"), "make");
    assert_eq!(named("deploy to Railway"), "railway");
    assert_eq!(named("update a lead in Close"), "close");
    assert_eq!(named("move leads from Close to Attio"), "close attio");
    // A service named is the one meant: none ranked beside it, though Guru has cards.
    assert_eq!(named("use stripe to charge a card"), "stripe");
    assert_eq!(named("save this to notion"), "notion");
    // Ranked, a plain word's name counts for nothing: `make` and `close` are verbs.
    assert_eq!(named("make the deal close"), "");
    assert_eq!(named("find the team's cards"), "guru");
}

#[test]
fn the_search_is_listed_where_the_catalog_is_offered_with_none_left_to_connect() {
    let mut set = set_of(vec![github()], u64::MAX);
    set.offered = Some(Vec::new());
    set.frame(128_000);
    assert_eq!(names(&set).last(), Some(&TOOL_SEARCH));
    assert_eq!(set.search(Some("Linear"), None).suggested, []);
    // Offered none: not listed.
    assert!(!names(&set_of(vec![github()], 128_000)).contains(&TOOL_SEARCH));
}
