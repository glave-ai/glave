//! Each project as this glave reads it: asked about in plain words, a yes kept exactly as the
//! terminal keeps it, what it brings on from then and only then, a change asked about again,
//! the model told what stays off; what each project chooses reaching its own threads alone,
//! a model glave does not offer set aside with a sentence; a project's server giving way to
//! the person's own of the same name, the model told; and a project's servers closed after 30
//! idle minutes, never while a turn holds them, and started again when needed.

use std::path::{Path, PathBuf};

use glave_config::policy::{self, ConfigFile, Constraints, ModelEntry, Source};
use glave_event::PrincipalId;
use glave_harness::Permissions;
use glave_ports::block_on;
use glave_ports::settings::{Judge, Protections};
use glave_ports::testing::{ManualClock, TempDir};

use super::servers::Kept;
use super::*;
use crate::gateway::{Allowed, gateway};
use crate::model::Model;
use glave_harness::ProjectFolders as _;
use glave_ports::{Clock as _, SystemClock};
use glave_session::ProjectTrust as _;

/// A home folder and a project in it whose `.mcp.json` has `servers` and whose config is
/// `config`, both fresh for the test `name`, in a folder removed when the first drops.
fn place(name: &str, servers: &str, config: &str) -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new(&format!("projects-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let project = another(&dir, "project", servers, config);
    let home = dir.join("home");
    (dir, home, project)
}

/// Another project beside it, called `name`.
fn another(dir: &Path, name: &str, servers: &str, config: &str) -> PathBuf {
    let project = dir.join(name);
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    write(&project, servers, config);
    project
}

fn write(project: &Path, servers: &str, config: &str) {
    let mcp = format!(r#"{{"mcpServers": {{{servers}}}}}"#);
    std::fs::write(project.join(".mcp.json"), mcp).unwrap();
    std::fs::write(project.join(".glave/config.toml"), config).unwrap();
}

const DOCS: &str = r#""docs": {"command": "/nonexistent/docs-mcp", "args": ["--fast"]}"#;
const GUARD: &str = "[[hooks.PreToolUse]]\nmatcher = \"bash\"\ncommand = \"./guard.sh\"\n";

/// glave's own levels in `home`, with nothing chosen.
async fn loaded(home: &Path) -> Loaded {
    let locations = Locations {
        root: home.to_owned(),
        person: home.join("config.toml"),
        project: home.join("none.toml"),
        policies: Vec::new(),
    };
    let none = ConfigFile::default();
    policy::load(&locations, &[], None, none.clone(), none)
        .await
        .unwrap()
}

/// The models a gateway offers: its main one, `big`, and each of `models`.
fn offering(models: &[&str]) -> GatewaySwitch {
    let endpoint = crate::gateway::tests::endpoint();
    let entries: Vec<ModelEntry> = models
        .iter()
        .map(|id| ModelEntry {
            id: (*id).to_owned(),
            name: None,
        })
        .collect();
    let allowed = Allowed {
        constraints: &Constraints::default(),
        main: None,
        judge: &Source::Default,
        models: &Source::Default,
    };
    let provider = Model::for_endpoint(&endpoint).unwrap();
    let built = gateway(provider, &endpoint, None, &entries, Vec::new(), &allowed).unwrap();
    GatewaySwitch::new(Some(built), None)
}

/// The projects of a glave in `home` already running the person's own servers `taken`,
/// offering the models `models` besides its main one.
async fn projects(home: &Path, taken: Vec<String>, models: &[&str]) -> ProjectFolders {
    timed(home, taken, models, ManualClock::at_millis(NOW)).await
}

/// When the tests' clocks stand.
const NOW: i64 = 1_791_000_000_000;

/// `projects` on `clock`.
async fn timed(
    home: &Path,
    taken: Vec<String>,
    models: &[&str],
    clock: ManualClock,
) -> ProjectFolders {
    let loaded = loaded(home).await;
    let own = Own {
        choices: loaded.settle().unwrap(),
        loaded,
        taken,
        models: offering(models),
        memory: None,
        clock: Box::new(clock),
        processes: Processes::unrecorded(),
        trust: Trust::in_memory().await.unwrap().1,
    };
    ProjectFolders::new(home.to_owned(), own, true)
}

fn project_id() -> ProjectId {
    ProjectId::from_uuid(uuid::Uuid::from_u128(2))
}

fn plain(ask: &TrustAsk) -> Vec<(&str, Option<&str>)> {
    ask.items
        .iter()
        .map(|item| (&*item.text, item.detail.as_deref()))
        .collect()
}

#[test]
fn servers_put_away_close_only_once_no_turn_holds_them() {
    let kept = |root: &str| Kept {
        root: PathBuf::from(root),
        servers: BTreeMap::new(),
        started: Vec::new(),
        sources: vec![Arc::new(glave_ports::InMemoryTools::new(Vec::new())) as Arc<dyn ToolSource>],
        used: jiff::Timestamp::UNIX_EPOCH,
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
    let (_dir, home, project) = place("asked", DOCS, GUARD);
    block_on!(async {
        let projects = projects(&home, Vec::new(), &[]).await;
        let ask = projects.ask(&project).await.unwrap().unwrap();
        assert_eq!(
            plain(&ask),
            [
                ("Runs a tool server", Some("/nonexistent/docs-mcp --fast")),
                ("Runs a hook before each command", Some("./guard.sh")),
            ]
        );
        let before = projects.brought(&project, project_id()).await;
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
        let record = projects.trust.record().await.unwrap();
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
        let after = projects.brought(&project, project_id()).await;
        assert_eq!(after.tools.len(), 2, "the server, then its resources");
        assert_eq!(after.hooks.all().count(), 1);
        let again = projects.brought(&project, project_id()).await;
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
        let meanwhile = projects.brought(&project, project_id()).await;
        assert!(meanwhile.tools.is_empty());
        assert_eq!(meanwhile.hooks.all().count(), 1, "the hook did not change");
        projects.close().await;
    });
}

/// A project that chooses almost everything a project may, two of them needing trust.
const CHOOSES: &str = "model = \"alpha\"\njudge_model = \"alpha-judge\"\n\
                       permissions = \"ask\"\nnetwork = false\ntitles = false\n\
                       worktree = true\n[web]\nfetch = false\n";

#[test]
fn each_project_s_settings_reach_its_own_threads_alone() {
    let (_dir, home, alpha) = place("settled", "", CHOOSES);
    let beta = another(home.parent().unwrap(), "beta", "", "model = \"beta\"\n");
    block_on!(async {
        let projects = projects(&home, Vec::new(), &["alpha", "alpha-judge", "beta"]).await;
        let ask = projects.ask(&alpha).await.unwrap().unwrap();
        assert_eq!(
            plain(&ask),
            [(
                "Chooses which model judges risky actions",
                Some("alpha-judge")
            )]
        );
        let untrusted = projects.settled(&alpha).await;
        assert_eq!(
            untrusted.model.as_deref(),
            Some("alpha"),
            "a model needs no trust"
        );
        assert_eq!(untrusted.judge, None, "its judge does");
        assert!(projects.trust(&alpha, &ask.ask).await.unwrap());
        let settled = projects.settled(&alpha).await;
        assert_eq!(
            settled.protections,
            Protections {
                permissions: Some(Permissions::Ask),
                offline: true,
            }
        );
        assert_eq!(
            (settled.judge, settled.titles, settled.worktree),
            (
                Some(Judge::Model("alpha-judge".into())),
                Some(false),
                Some(true)
            )
        );
        let brought = projects.brought(&alpha, project_id()).await;
        let web = brought.web.expect("its own web_fetch");
        assert!(web.tools().await.unwrap().is_empty(), "turned off");

        let other = projects.settled(&beta).await;
        assert_eq!(other.model.as_deref(), Some("beta"));
        assert_eq!(
            (other.protections, other.judge, other.titles, other.worktree),
            (Protections::default(), None, None, None),
            "none of alpha's"
        );
        assert!(projects.brought(&beta, project_id()).await.web.is_none());
        assert_eq!(projects.set_aside(&beta).await, Vec::<String>::new());
    });
}

#[test]
fn a_model_glave_does_not_offer_is_set_aside_with_a_sentence() {
    let (_dir, home, project) = place("aside", "", "model = \"elsewhere\"\n");
    block_on!(async {
        let projects = projects(&home, Vec::new(), &[]).await;
        assert_eq!(projects.settled(&project).await.model, None, "the default");
        let aside = projects.set_aside(&project).await;
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert!(
            aside[0].contains("\"elsewhere\"") && aside[0].contains("does not offer"),
            "{aside:?}"
        );
    });
}

#[test]
fn a_project_s_server_gives_way_to_the_person_s_own_of_the_same_name() {
    let (_dir, home, project) = place("taken", DOCS, "");
    block_on!(async {
        let projects = projects(&home, vec!["docs".into()], &[]).await;
        let ask = projects.ask(&project).await.unwrap().unwrap();
        assert!(projects.trust(&project, &ask.ask).await.unwrap());
        let brought = projects.brought(&project, project_id()).await;
        assert!(brought.tools.is_empty(), "the person's own runs instead");
        let told = projects
            .for_folder(&project, Some(&project), true)
            .await
            .unwrap();
        let said = "MCP server docs is off: the person's own server of the same name runs instead";
        assert!(told.contains(said), "the model is told: {told}");
        assert_eq!(projects.set_aside(&project).await, [said], "and the person");
    });
}

#[test]
fn an_idle_project_s_servers_close_and_start_again_when_needed() {
    let (_dir, home, project) = place("idle", DOCS, "");
    block_on!(async {
        let clock = ManualClock::at_millis(NOW);
        let projects = timed(&home, Vec::new(), &[], clock.clone()).await;
        let ask = projects.ask(&project).await.unwrap().unwrap();
        assert!(projects.trust(&project, &ask.ask).await.unwrap());
        let held = projects.brought(&project, project_id()).await.tools;
        let minutes = |n: i64| clock.advance_millis(n * 60_000);
        let running = |projects: &ProjectFolders| projects.lock().kept.len();
        minutes(60);
        projects.sweep().await;
        assert_eq!(running(&projects), 1, "a turn holds them: in use");
        drop(held);
        minutes(29);
        projects.sweep().await;
        assert_eq!(running(&projects), 1, "used 29 minutes ago");
        minutes(2);
        projects.sweep().await;
        assert_eq!(running(&projects), 0, "closed after 30 minutes unused");
        let again = projects.brought(&project, project_id()).await;
        assert_eq!(again.tools.len(), 2, "started again for the next turn");
        assert_eq!(running(&projects), 1);
        projects.close().await;
    });
}

#[test]
fn a_project_on_a_person_s_computer_chooses_within_the_server_s_rules_and_loosens_only_once_trusted()
 {
    let home = TempDir::new("settled-at-home").unwrap();
    block_on!(async {
        let projects = projects(&home, Vec::new(), &["alpha"]).await;
        let root = Path::new("/home/glave/garden");
        let person = || PrincipalId::new(SystemClock.now(), crate::accounts::random().unwrap());
        let (ana, bo) = (person().unwrap(), person().unwrap());
        let settled = projects
            .settled_at_home(root, "model = \"alpha\"\n", ana)
            .await;
        assert_eq!(settled.model.as_deref(), Some("alpha"));
        let not_offered = projects
            .settled_at_home(root, "model = \"beta\"\n", ana)
            .await;
        assert_eq!(not_offered.model, None, "one the server doesn't offer");
        let broken = projects.settled_at_home(root, "model = ", ana).await;
        assert_eq!(
            broken,
            Settled::default(),
            "one that doesn't parse chooses nothing"
        );

        // What needs trust stays off, and is asked of its person, until they say yes.
        let judged = "judge_model = \"alpha\"\npermissions = \"full-access\"\n";
        let loosening = projects.settled_at_home(root, judged, ana).await;
        assert_eq!(loosening.judge, None, "what needs trust stays off");
        assert_eq!(loosening.protections.permissions, None, "and full access");
        let asked = projects
            .ask_there(ana, root, judged)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !projects
                .trust_there(ana, root, judged, "another")
                .await
                .unwrap()
        );
        assert!(
            projects
                .trust_there(ana, root, judged, &asked.ask)
                .await
                .unwrap()
        );
        let trusted = projects.settled_at_home(root, judged, ana).await;
        assert_eq!(trusted.judge, Some(Judge::Model("alpha".into())));
        assert_eq!(
            trusted.protections.permissions,
            Some(Permissions::FullAccess)
        );
        assert_eq!(projects.ask_there(ana, root, judged).await.unwrap(), None);

        // Their yes is theirs: the same path on someone else's computer is another's to answer.
        let others = projects.settled_at_home(root, judged, bo).await;
        assert_eq!(others.protections.permissions, None);
        assert!(
            projects
                .ask_there(bo, root, judged)
                .await
                .unwrap()
                .is_some()
        );

        // A wider [web] isn't theirs to bring: `web_fetch` runs on the server.
        let web = "[web]\nprivate_addresses = true\n";
        assert_eq!(projects.ask_there(ana, root, web).await.unwrap(), None);
    });
}
