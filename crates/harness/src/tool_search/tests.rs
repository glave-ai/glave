use super::*;

fn tool(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters: json!({"type": "object", "properties": {
            "text": {"type": "string"}
        }}),
    }
}

fn found(tools: &[ToolSpec], query: &str) -> Vec<String> {
    let tools: Vec<&ToolSpec> = tools.iter().collect();
    let found = matching(&tools, query);
    found.iter().map(|tool| tool.name.clone()).collect()
}

fn catalog() -> Vec<ToolSpec> {
    vec![
        tool("mcp__github__list_issues", "List a repository's issues."),
        tool("mcp__github__create_pull_request", "Open a pull request."),
        tool("mcp__slack__post_message", "Post a message to a channel."),
        tool(
            "mcp__pager__whoIsOnCall",
            "Find who is on call for a service.",
        ),
        tool("mcp__wiki__search", "Search pages. Mentions issues rarely."),
    ]
}

#[test]
fn the_best_match_comes_first_and_only_what_matches_comes_at_all() {
    let tools = catalog();
    assert_eq!(
        found(&tools, "post to a slack channel")[0],
        "mcp__slack__post_message"
    );
    assert_eq!(found(&tools, "kubernetes"), Vec::<String>::new());
}

#[test]
fn a_name_counts_for_more_than_a_passing_mention() {
    let tools = catalog();
    assert_eq!(found(&tools, "issues")[0], "mcp__github__list_issues");
}

#[test]
fn a_sentence_finds_what_its_rare_words_find_and_nothing_for_its_common_ones() {
    let mut tools: Vec<ToolSpec> = (0..8)
        .map(|n| {
            tool(
                &format!("mcp__a__report_{n}"),
                "It is for a team, on a page.",
            )
        })
        .collect();
    tools.push(tool(
        "mcp__pager__whoIsOnCall",
        "Find who is on call for a service.",
    ));
    assert_eq!(
        found(&tools, "who is on call for payments"),
        ["mcp__pager__whoIsOnCall"]
    );
    assert_eq!(found(&tools, "kubernetes for a team"), Vec::<String>::new());
}

#[test]
fn a_schema_s_own_words_find_nothing_but_its_parameters_names_do() {
    let tools = catalog();
    assert_eq!(found(&tools, "object"), Vec::<String>::new());
    assert_eq!(found(&tools, "text").len(), MOST_FOUND);
}

#[test]
fn words_match_across_plurals_and_ways_of_writing_a_name() {
    let tools = catalog();
    assert_eq!(found(&tools, "ISSUE")[0], "mcp__github__list_issues");
    assert_eq!(found(&tools, "on call")[0], "mcp__pager__whoIsOnCall");
    assert_eq!(words("listOpenIssues"), ["list", "open", "issue"]);
    assert_eq!(words("status class"), ["statu", "class"]);
}

#[test]
fn exact_names_find_exactly_those_in_the_order_asked() {
    let tools = catalog();
    assert_eq!(
        found(&tools, "mcp__wiki__search, MCP__GITHUB__LIST_ISSUES"),
        ["mcp__wiki__search", "mcp__github__list_issues"]
    );
}

#[test]
fn one_search_finds_only_so_many() {
    let tools: Vec<ToolSpec> = (0..20)
        .map(|n| tool(&format!("mcp__a__report_{n}"), "A report."))
        .collect();
    assert_eq!(found(&tools, "report").len(), MOST_FOUND);
}

#[test]
fn a_line_is_the_first_sentence_cut_short() {
    assert_eq!(one_line("Open it. Then close it."), "Open it");
    assert_eq!(one_line("One\nTwo"), "One");
    let long = "é".repeat(LINE + 1);
    assert_eq!(one_line(&long).chars().count(), LINE + 1);
}

#[test]
fn the_listing_stops_at_its_room_and_says_how_many_are_left() {
    let tools = catalog();
    let refs: Vec<&ToolSpec> = tools.iter().collect();
    let all = spec(&refs, usize::MAX).description;
    assert!(all.contains("mcp__wiki__search: Search pages\n"), "{all}");
    assert!(!all.contains("more, which"), "{all}");
    let some = spec(&refs, 60).description;
    assert!(some.contains("mcp__github__list_issues"), "{some}");
    assert!(!some.contains("mcp__wiki__search"), "{some}");
    assert!(
        some.contains("- and 4 more, which a search finds too"),
        "{some}"
    );
}

#[test]
fn a_tool_a_turn_adds_goes_before_the_loaded_ones_so_they_stay_last() {
    let connected = glave_ports::InMemoryTools::new(vec![(
        glave_ports::ToolInfo {
            spec: tool("mcp__a__one", &"word ".repeat(800)),
            effect: glave_ports::Effect::Reads,
            parallel: true,
        },
        ToolOutput::text(false, "one"),
    )])
    .as_connected();
    let sources: Vec<std::sync::Arc<dyn glave_ports::ToolSource>> =
        vec![std::sync::Arc::new(connected)];
    let mut set = glave_ports::block_on!(ToolSet::gather(&sources, "system"));
    let loaded = ["mcp__a__one".to_owned()];
    set.defer_if_crowded(1_000, &loaded);
    assert_eq!(set.names(), ["tool_search", "mcp__a__one"]);
    set.offer_goal();
    assert_eq!(set.names(), ["tool_search", "update_goal", "mcp__a__one"]);
    assert_eq!(set.load(&loaded), Vec::<String>::new(), "loaded once");
    assert!(!set.is_held_back("mcp__a__one"));

    // Below the line again, a thread that loaded some keeps its list as it was.
    let mut set = glave_ports::block_on!(ToolSet::gather(&sources, "system"));
    set.defer_if_crowded(1_000_000, &loaded);
    assert_eq!(set.names(), ["tool_search", "mcp__a__one"]);
    let mut fresh = glave_ports::block_on!(ToolSet::gather(&sources, "system"));
    fresh.defer_if_crowded(1_000_000, &[]);
    assert_eq!(fresh.names(), ["mcp__a__one"]);
}

#[test]
fn crowding_starts_past_a_tenth_of_the_window() {
    let tools = vec![tool("t", &"word ".repeat(800))];
    let tokens = tools_tokens(&tools);
    assert!(
        !crowds(&tools, tokens * SHARE),
        "at the line, nothing changes"
    );
    assert!(crowds(&tools, tokens * SHARE - SHARE), "just past it");
}
