use std::sync::Arc;

use glave_event::{Arguments, ServiceAnswer, ServiceFieldKind};
use glave_ports::testing::TempDir;
use glave_ports::{Called, ServiceAsks, ServiceGiven, ToolSource};
use serde_json::{Value, json};

use super::*;
use crate::cancel::Cancel;

const SKILL: &str = "---\nname: release-notes\ndescription: Writes release notes from merged pull requests.\n---\n\nRead the merged pull requests, then write the notes.\n";

fn arguments(value: Value) -> Arguments {
    match value {
        Value::Object(map) => Arguments::Json(map),
        other => panic!("not an object: {other}"),
    }
}

fn skill() -> Arguments {
    arguments(json!({"kind": "skill", "name": "release-notes", "skill": SKILL}))
}

fn server() -> Arguments {
    arguments(json!({"kind": "server", "name": "linear", "server": {
        "type": "http", "url": "https://mcp.linear.test/mcp",
        "headers": {"X-Team": "core", "Authorization": "Bearer ${LINEAR_TOKEN}"}
    }}))
}

/// The `install` tool on a project in `project`, the person's folder `own`.
async fn install(project: &Path, own: &Path, servers: bool) -> Install {
    let workspace = Workspace::open(project, std::env::vars_os()).await.unwrap();
    Install {
        workspace: Arc::new(workspace),
        own: Some(Own {
            folder: own.to_owned(),
            servers,
        }),
    }
}

async fn asked(tool: &Install, arguments: &Arguments) -> ServiceAsks {
    match tool
        .call_asking(INSTALL, arguments, None, &Cancel::new())
        .await
    {
        Called::Asks(asks) => asks,
        Called::Done(output) => panic!("it did not ask: {output:?}"),
    }
}

/// The values of the choices the person is offered.
fn offered(asks: &ServiceAsks) -> Vec<String> {
    let Some(ServiceFieldKind::Choice { options, .. }) = asks.fields.first().map(|f| &f.kind)
    else {
        panic!("no choice: {asks:?}");
    };
    options.iter().map(|option| option.value.clone()).collect()
}

/// The call made again with the person's answer, `place`.
async fn chosen(tool: &Install, asks: ServiceAsks, place: &str) -> ToolOutput {
    let given = ServiceGiven {
        state: asks.state,
        answer: ServiceAnswer::Answered {
            values: json!({ WHERE: place }).as_object().unwrap().clone(),
        },
    };
    match tool
        .call_asking(
            INSTALL,
            &Arguments::Json(serde_json::Map::default()),
            Some(given),
            &Cancel::new(),
        )
        .await
    {
        Called::Done(output) => output,
        Called::Asks(asks) => panic!("it asked again: {asks:?}"),
    }
}

#[test]
fn what_it_adds_is_read_and_shown_in_one_line() {
    let Ok(Change::AddSkill { description, .. }) = change(&skill()) else {
        panic!("the skill was not read");
    };
    assert_eq!(
        description,
        "Writes release notes from merged pull requests."
    );
    let shown = message(&change(&skill()).unwrap());
    assert_eq!(
        shown,
        "Add the skill “release-notes” (6 lines): Writes release notes from merged pull requests."
    );
    assert_eq!(
        message(&change(&server()).unwrap()),
        "Connect “linear”, at https://mcp.linear.test/mcp, sending Authorization from \
         ${LINEAR_TOKEN}, X-Team. Signing in to it stays yours either way."
    );
    let program = arguments(json!({"kind": "server", "name": "files", "server": {
        "command": "npx", "args": ["-y", "files-server"], "env": {"ROOT": "/srv"}
    }}));
    assert_eq!(
        message(&change(&program).unwrap()),
        "Connect “files”, which runs npx -y files-server, with ROOT set."
    );
}

#[test]
fn what_cannot_be_added_says_why() {
    let misnamed = arguments(json!({"kind": "skill", "name": "notes", "skill": SKILL}));
    assert!(change(&misnamed).unwrap_err().contains("must match"));
    let bare = arguments(json!({"kind": "skill", "name": "notes"}));
    assert!(change(&bare).unwrap_err().contains("SKILL.md"));
    let odd =
        arguments(json!({"kind": "server", "name": "x", "server": {"type": "sse", "url": "u"}}));
    assert!(change(&odd).unwrap_err().contains("can't be added"));
    let spaced = arguments(json!({"kind": "server", "name": "my server", "remove": true}));
    assert!(change(&spaced).unwrap_err().contains("letters, digits"));
}

#[test]
fn a_skill_goes_where_the_person_chooses() {
    let (project, own) = (
        TempDir::new("install-p").unwrap(),
        TempDir::new("install-o").unwrap(),
    );
    glave_ports::block_on!(async {
        let tool = install(&project, &own, true).await;
        let asks = asked(&tool, &skill()).await;
        assert_eq!(asks.service, ASKED_BY_GLAVE);
        assert_eq!(offered(&asks), ["project", "everywhere"]);
        // Nothing is written before the answer.
        assert!(!project.join(".agents").exists() && !own.join("skills").exists());

        let done = chosen(&tool, asks.clone(), "project").await;
        assert!(!done.is_error, "{done:?}");
        let file = project.join(".agents/skills/release-notes/SKILL.md");
        assert_eq!(std::fs::read_to_string(file).unwrap(), SKILL);

        // Asked again, it says what it replaces.
        let asks = asked(&tool, &skill()).await;
        assert!(
            asks.message
                .ends_with("It replaces the one by that name in this project.")
        );
        let done = chosen(&tool, asks, "everywhere").await;
        assert!(!done.is_error, "{done:?}");
        let file = own.join("skills/release-notes/SKILL.md");
        assert_eq!(std::fs::read_to_string(file).unwrap(), SKILL);

        // Taking it out asks only where it is, then takes it out there.
        let remove = arguments(json!({"kind": "skill", "name": "release-notes", "remove": true}));
        let asks = asked(&tool, &remove).await;
        // The project's copy is the model's own to delete, with the project's other changes.
        assert_eq!(offered(&asks), ["everywhere"]);
        std::fs::remove_dir_all(own.join("skills")).unwrap();
        let theirs = tool
            .call_asking(INSTALL, &remove, None, &Cancel::new())
            .await;
        assert!(matches!(theirs, Called::Done(output) if output.is_error));
        std::fs::remove_dir_all(project.join(".agents")).unwrap();
        let again = asked(&tool, &skill()).await;
        assert!(!chosen(&tool, again, "everywhere").await.is_error);
        let asks = asked(&tool, &remove).await;
        assert_eq!(offered(&asks), ["everywhere"]);
        assert!(!chosen(&tool, asks, "everywhere").await.is_error);
        assert!(!own.join("skills/release-notes").exists());
        let none = tool
            .call_asking(INSTALL, &remove, None, &Cancel::new())
            .await;
        assert!(matches!(none, Called::Done(output) if output.is_error));
    });
}

#[test]
fn a_server_joins_the_others_and_comes_out_again() {
    let (project, own) = (
        TempDir::new("install-s").unwrap(),
        TempDir::new("install-t").unwrap(),
    );
    let theirs = "{\n  \"mcpServers\": {\"files\": {\"command\": \"files\",  \"args\": []}}\n}\n";
    std::fs::write(project.join(".mcp.json"), theirs).unwrap();
    glave_ports::block_on!(async {
        let tool = install(&project, &own, false).await;
        let asks = asked(&tool, &server()).await;
        // A policy keeps the person's own servers away: only the project is offered.
        assert_eq!(offered(&asks), ["project"]);
        assert!(!chosen(&tool, asks, "project").await.is_error);
        let written = std::fs::read_to_string(project.join(".mcp.json")).unwrap();
        assert!(
            written.contains(r#""files": {"command": "files",  "args": []}"#),
            "{written}"
        );
        assert!(written.contains("https://mcp.linear.test/mcp"), "{written}");

        let remove = arguments(json!({"kind": "server", "name": "linear", "remove": true}));
        let asks = asked(&tool, &remove).await;
        assert!(!chosen(&tool, asks, "project").await.is_error);
        let written = std::fs::read_to_string(project.join(".mcp.json")).unwrap();
        assert!(
            !written.contains("linear") && written.contains("\"files\""),
            "{written}"
        );
    });
}

#[test]
fn nothing_changes_on_a_no_or_with_no_one_to_ask() {
    let (project, own) = (
        TempDir::new("install-n").unwrap(),
        TempDir::new("install-m").unwrap(),
    );
    glave_ports::block_on!(async {
        let tool = install(&project, &own, true).await;
        let asks = asked(&tool, &skill()).await;
        let given = ServiceGiven {
            state: asks.state,
            answer: ServiceAnswer::Declined,
        };
        let empty = Arguments::Json(serde_json::Map::default());
        let said = tool
            .call_asking(INSTALL, &empty, Some(given), &Cancel::new())
            .await;
        assert!(matches!(said, Called::Done(output) if output.is_error));
        let alone = tool.call(INSTALL, &skill(), &Cancel::new()).await;
        assert_eq!(alone, ToolOutput::text(true, NO_ONE));
        assert!(!project.join(".agents").exists() && !own.join("skills").exists());
    });
}
