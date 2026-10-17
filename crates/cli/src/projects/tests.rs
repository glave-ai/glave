//! Each project as this glave reads it: asked about in plain words, a yes kept exactly as the
//! terminal keeps it, what it brings on from then and only then, a change asked about again,
//! the model told what stays off, and nothing twice of the project glave started in.

use std::path::{Path, PathBuf};

use glave_config::policy::{self, ConfigFile};
use glave_config::trust::Record;
use glave_ports::block_on;

use super::*;

/// A home folder and a project in it whose `.mcp.json` has `servers` and whose config is
/// `config`, both fresh for the test `name`.
fn place(name: &str, servers: &str, config: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("glave-projects-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let project = dir.join("project");
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    write(&project, servers, config);
    let dir = std::fs::canonicalize(dir).unwrap();
    (dir.join("home"), dir.join("project"))
}

fn write(project: &Path, servers: &str, config: &str) {
    let mcp = format!(r#"{{"mcpServers": {{{servers}}}}}"#);
    std::fs::write(project.join(".mcp.json"), mcp).unwrap();
    std::fs::write(project.join(".glave/config.toml"), config).unwrap();
}

const DOCS: &str = r#""docs": {"command": "/nonexistent/docs-mcp", "args": ["--fast"]}"#;
const GUARD: &str = "[[hooks.PreToolUse]]\nmatcher = \"bash\"\ncommand = \"./guard.sh\"\n";

/// glave's own choices in `home`, with nothing chosen.
async fn choices(home: &Path) -> Choices {
    let locations = Locations {
        root: home.to_owned(),
        person: home.join("config.toml"),
        project: home.join("none.toml"),
        policies: Vec::new(),
    };
    let none = ConfigFile::default();
    let loaded = policy::load(&locations, &[], None, none.clone(), none)
        .await
        .unwrap();
    loaded.settle().unwrap()
}

async fn projects(home: &Path, started: Started, taken: Vec<String>) -> Projects {
    let choices = choices(home).await;
    Projects::new(
        home.to_owned(),
        Constraints::default(),
        choices,
        started,
        taken,
        true,
    )
}

fn plain(ask: &TrustAsk) -> Vec<(&str, Option<&str>)> {
    ask.items
        .iter()
        .map(|item| (item.text.as_str(), item.detail.as_deref()))
        .collect()
}

#[test]
fn servers_put_away_close_only_once_no_turn_holds_them() {
    let kept = |root: &str| Kept {
        root: PathBuf::from(root),
        servers: BTreeMap::new(),
        started: Vec::new(),
        sources: vec![Arc::new(glave_ports::MemoryTools::new(Vec::new())) as Arc<dyn ToolSource>],
    };
    let mut running = Running::default();
    let held = kept("/held");
    let turn = held.sources.clone();
    let closing = running.retire([held, kept("/idle")]);
    assert_eq!(closing.len(), 1);
    assert_eq!(closing[0].root, PathBuf::from("/idle"));
    assert_eq!(running.retired.len(), 1, "a turn still holds it");
    drop(turn);
    let closing = running.retire([]);
    assert_eq!(closing.len(), 1, "its turn is over");
    assert!(running.retired.is_empty());
}

#[test]
fn a_project_is_asked_about_in_plain_words_and_on_once_trusted_and_only_then() {
    let (home, project) = place("asked", DOCS, GUARD);
    block_on!(async {
        let projects = projects(&home, Started::default(), Vec::new()).await;
        let ask = projects.ask(&project).await.unwrap().unwrap();
        assert_eq!(
            plain(&ask),
            [
                ("Runs a tool server", Some("/nonexistent/docs-mcp --fast")),
                ("Runs a hook before each command", Some("./guard.sh")),
            ]
        );
        let before = projects.brought(&project).await;
        assert!(
            before.tools.is_empty() && before.hooks.is_empty(),
            "off until trusted"
        );
        let off = projects
            .for_folder(&project, Some(&project), true)
            .await
            .unwrap();
        assert!(
            off.contains("# Project tools that are off")
                && off.contains("the person has not trusted this project yet"),
            "{off}"
        );

        assert!(projects.trust(&project, &ask.ask).await.unwrap());
        let record = Record::load(&home).await.unwrap();
        let read = Project::read(&project, &crate::mcp::environment())
            .await
            .unwrap();
        let items = &record.entry_for(&read).unwrap().items;
        assert_eq!(
            items.keys().collect::<Vec<_>>(),
            ["hook:PreToolUse:1", "mcp:docs"],
            "kept per item, as the terminal keeps them"
        );
        assert!(items.values().all(|decision| decision.trusted));
        assert_eq!(projects.ask(&project).await.unwrap(), None, "asked once");
        let theirs = projects
            .for_folder(&project, Some(&project), false)
            .await
            .unwrap();
        assert!(
            theirs.contains("MCP server docs (/nonexistent/docs-mcp --fast) is off")
                && theirs.contains("never for anyone else's threads"),
            "trusted, and still off on another person's thread: {theirs}"
        );
        let after = projects.brought(&project).await;
        assert_eq!(after.tools.len(), 2, "the server, then its resources");
        assert_eq!(after.hooks.all().count(), 1);
        let again = projects.brought(&project).await;
        assert!(
            Arc::ptr_eq(&after.tools[0], &again.tools[0]),
            "the same server, still running"
        );
        let told = projects.for_folder(&project, Some(&project), true).await;
        assert!(
            told.as_ref().is_none_or(|told| !told.contains("is off")),
            "{told:?}"
        );

        // The repository changes its server: asked about again, only it, and off meanwhile.
        write(&project, &DOCS.replace("--fast", "--slow"), GUARD);
        let changed = projects.ask(&project).await.unwrap().unwrap();
        assert_eq!(
            plain(&changed),
            [("Runs a tool server", Some("/nonexistent/docs-mcp --slow"))]
        );
        assert!(
            !projects.trust(&project, &ask.ask).await.unwrap(),
            "not what was shown"
        );
        let meanwhile = projects.brought(&project).await;
        assert!(meanwhile.tools.is_empty());
        assert_eq!(meanwhile.hooks.all().count(), 1, "the hook did not change");
        projects.close().await;
    });
}

#[test]
fn what_glave_started_with_in_its_own_project_is_not_brought_twice_but_its_hooks_are_its_own() {
    let (home, project) = place("started", DOCS, &format!("approval = \"auto\"\n{GUARD}"));
    block_on!(async {
        let first = projects(&home, Started::default(), Vec::new()).await;
        let ask = first.ask(&project).await.unwrap().unwrap();
        assert!(first.trust(&project, &ask.ask).await.unwrap());
        let read = Project::read(&project, &crate::mcp::environment())
            .await
            .unwrap();
        let outcome = trust::assessed(&home, read, &Constraints::default())
            .await
            .unwrap()
            .apply(Answer::Later);
        let started = Started::from_outcome(&project, &home, &outcome);
        assert!(started.settings);
        let restarted = projects(&home, started, vec!["docs".into()]).await;
        let brought = restarted.brought(&project).await;
        assert!(
            brought.tools.is_empty(),
            "its server already runs for every thread"
        );
        assert_eq!(brought.loosened, Loosened::default(), "nor its settings");
        assert_eq!(
            brought.hooks.all().count(),
            1,
            "its hooks run for its own threads alone, as any project's do"
        );
        let elsewhere = projects(&home, Started::default(), vec!["docs".into()]).await;
        let brought = elsewhere.brought(&project).await;
        assert!(
            brought.tools.is_empty(),
            "a server of the same name runs already"
        );
        assert_eq!(brought.hooks.all().count(), 1);
        assert_eq!(brought.loosened.approval, Some(ApprovalMode::Auto));
        first.close().await;
    });
}
