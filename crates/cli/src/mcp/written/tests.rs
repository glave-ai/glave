//! The person's `mcp.json` changed by glave: a server added and taken out again, the rest as
//! the person wrote it, in its order; one they set up themselves left alone.

use glave_ports::testing::TempDir;

use super::*;

const SEARCH: &str = "https://mcp.exa.ai/mcp?tools=web_search_exa";

fn file(home: &Path) -> std::path::PathBuf {
    home.join(super::super::FILE)
}

fn read(home: &Path) -> String {
    std::fs::read_to_string(file(home)).unwrap()
}

#[test]
fn a_server_joins_the_person_s_own_and_leaves_them_as_they_wrote_them() {
    let home = TempDir::new("mcp-written").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    let writer = Writer::default();
    let theirs = r#"{"other": 1, "mcpServers": {"zed": {"command": "zed-mcp", "args": ["--x"]},
        "docs": {"env": {"KEY": "${KEY}"}, "command": "docs-mcp"}}}"#;
    std::fs::write(file(&home), theirs).unwrap();
    runtime.block_on(async {
        assert_eq!(writer.add(&home, "search", SEARCH).await.unwrap(), "search");
        assert_eq!(
            writer.add(&home, "search", SEARCH).await.unwrap(),
            "search",
            "on already"
        );
        let text = read(&home);
        let order = ["\"other\"", "\"zed\"", "\"docs\"", "\"search\""].map(|it| text.find(it));
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(
            text.contains(r#"{"env": {"KEY": "${KEY}"}, "command": "docs-mcp"}"#),
            "{text}"
        );
        let servers =
            glave_ports::settings::mcp::parse(&text, &std::collections::BTreeMap::new()).unwrap();
        assert!(servers["search"].is_remote());

        assert_eq!(writer.remove(&home, SEARCH).await.unwrap(), Removed::Gone);
        assert_eq!(
            writer.remove(&home, SEARCH).await.unwrap(),
            Removed::NotThere
        );
        let text = read(&home);
        assert!(!text.contains("search"), "{text}");
        assert!(text.contains(r#""args": ["--x"]"#), "{text}");
    });
}

#[test]
fn a_name_taken_gets_another_and_a_server_set_up_by_hand_stays() {
    let home = TempDir::new("mcp-written-theirs").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    let writer = Writer::default();
    let key = r#""headers": {"x-api-key": "${EXA_KEY}"}"#;
    let theirs = format!(
        r#"{{"mcpServers": {{"search": {{"command": "my-search"}},
            "exa": {{"url": "{SEARCH}", {key}}}}}}}"#
    );
    std::fs::write(file(&home), &theirs).unwrap();
    runtime.block_on(async {
        assert_eq!(
            writer.add(&home, "search", SEARCH).await.unwrap(),
            "exa",
            "theirs, there"
        );
        assert_eq!(writer.remove(&home, SEARCH).await.unwrap(), Removed::Theirs);
        assert_eq!(read(&home), theirs, "left as they wrote it");

        let other = "https://search.example.com/mcp";
        assert_eq!(
            writer.add(&home, "search", other).await.unwrap(),
            "search-2"
        );
        assert!(read(&home).contains(&format!(r#""search-2": {{"type":"http","url":"{other}"}}"#)));
    });
}

#[test]
fn with_no_settings_the_file_is_made_and_one_that_does_not_read_is_left_alone() {
    let home = TempDir::new("mcp-written-new").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    let writer = Writer::default();
    runtime.block_on(async {
        assert_eq!(writer.add(&home, "search", SEARCH).await.unwrap(), "search");
        assert!(read(&home).contains(SEARCH));
        std::fs::write(file(&home), "{not json").unwrap();
        let refused = writer.add(&home, "search", SEARCH).await.unwrap_err();
        assert!(
            refused.message.contains("can't be read"),
            "{}",
            refused.message
        );
        assert!(!refused.message.contains("mcp.json"), "plain words");
        assert_eq!(read(&home), "{not json");
    });
}

#[test]
fn a_linked_file_is_written_where_the_link_points() {
    let home = TempDir::new("mcp-written-link").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    let kept = home.join("dotfiles.json");
    std::fs::write(&kept, r#"{"mcpServers": {}}"#).unwrap();
    std::os::unix::fs::symlink(&kept, file(&home)).unwrap();
    runtime.block_on(async {
        Writer::default()
            .add(&home, "search", SEARCH)
            .await
            .unwrap();
    });
    assert!(std::fs::symlink_metadata(file(&home)).unwrap().is_symlink());
    assert!(std::fs::read_to_string(&kept).unwrap().contains(SEARCH));
}
