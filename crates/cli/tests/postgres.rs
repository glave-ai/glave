//! The binary on Postgres: with GLAVE_STORE_URL (or `[store] url`) naming a database, a turn's
//! history goes there and nowhere on this computer, and `glave threads` reads it back. Runs
//! when `GLAVE_TEST_POSTGRES_URL` names a database, in a schema of its own.

use std::process::Command;

use glave_ports::block_on;
use glave_provider_openai::fake::{FakeServer, Reply};
use tokio_postgres::NoTls;

use crate::served::trust_project;
use crate::support::*;

/// Runs `query` on the test database, returning the first column of the first row.
fn count(url: &str, query: &str) -> i64 {
    first(url, query)
}

/// Runs `query` on the test database, returning the first column of the first row.
fn first<T: for<'a> tokio_postgres::types::FromSql<'a>>(url: &str, query: &str) -> T {
    first_after(url, "", query)
}

/// Runs `setup`, then `query`, on one connection to the test database, returning the first
/// column of the first row.
fn first_after<T: for<'a> tokio_postgres::types::FromSql<'a>>(
    url: &str,
    setup: &str,
    query: &str,
) -> T {
    block_on!(async {
        let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        client.batch_execute(setup).await.unwrap();
        let row = client.query_one(query, &[]).await.unwrap();
        drop(client);
        let _ = connection.await;
        row.get(0)
    })
}

/// How many events the server's own organization has in `schema`: the store's role sees only
/// the organization its connection is set to (`specs/storage.md`, "Row-level security").
fn events_in(url: &str, schema: &str) -> i64 {
    first_after(
        url,
        &format!(
            "SELECT set_config('glave.org',
                 (SELECT value FROM {schema}.glave_settings WHERE key = 'org'), false)"
        ),
        &format!("SELECT count(*) FROM {schema}.events"),
    )
}

fn execute(url: &str, sql: &str) {
    block_on!(async {
        let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        client.batch_execute(sql).await.unwrap();
        drop(client);
        let _ = connection.await;
    });
}

#[test]
fn history_goes_to_the_database_the_store_url_names() {
    let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
        eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    let (schema, store) = scratch(&url, "history");
    let place = place("postgres");
    let server = FakeServer::start(vec![Reply::text("Stored far away.")]).unwrap();
    let child = start_with(
        &place,
        &server,
        &["--model", "fake-model", "remember this"],
        &[("GLAVE_STORE_URL", &store)],
    );
    let output = child.wait_with_output().unwrap();
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("Stored far away."), "{shown}");
    let events = events_in(&url, &schema);
    assert!(events > 0, "the turn's events are in the database");
    assert!(
        !place.home.join("glave.db").exists(),
        "nothing is kept in the local file"
    );
    let listed = Command::new(env!("CARGO_BIN_EXE_glave"))
        .arg("threads")
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_STORE_URL", &store)
        .output()
        .unwrap();
    let shown = text(&listed);
    assert!(listed.status.success(), "{shown}");
    // One thread on standard output; a server in the command's own process may say more on
    // standard error, such as that the sandbox cannot run here.
    let ids = String::from_utf8_lossy(&listed.stdout);
    assert_eq!(ids.lines().count(), 1, "{shown}");
    execute(&url, &format!("DROP SCHEMA {schema} CASCADE;"));
}

/// A schema of its own in the test database, and the store URL that names it.
fn scratch(url: &str, name: &str) -> (String, String) {
    let schema = format!("glave_test_cli_{name}_{}", std::process::id());
    execute(
        url,
        &format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"),
    );
    let separator = if url.contains('?') { '&' } else { '?' };
    let store = format!("{url}{separator}options=-c%20search_path%3D{schema}");
    (schema, store)
}

#[test]
fn two_homes_on_one_database_read_one_trust_record() {
    let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
        eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    let (schema, store) = scratch(&url, "trust");
    let first = place("postgres-trust-first");
    std::fs::write(
        first.project.join(".mcp.json"),
        r#"{"mcpServers": {"docs": {"command": "docs-mcp"}}}"#,
    )
    .unwrap();
    // The person said yes from the first home.
    trust_project(&first, &[("GLAVE_STORE_URL", &store)]);
    // The second home works in the same folder, with nothing of its own.
    let second = Place {
        project: first.project.clone(),
        home: first.home.with_file_name("second-home"),
        key: first.key.clone(),
        lifeline: first.lifeline.clone(),
        folder: None,
    };
    let explain = |place: &Place| {
        let output = Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["policy", "explain"])
            .current_dir(&place.project)
            .env("GLAVE_HOME", &place.home)
            .env("GLAVE_STORE_URL", &store)
            .env("GLAVE_MODEL", "gemma4:12b")
            .env_remove("GLAVE_JUDGE_MODEL")
            .output()
            .unwrap();
        text(&output)
    };
    let shown = explain(&first);
    assert!(shown.contains("trusted (your answer)"), "{shown}");
    // Servers on one database share its root, as they would a KMS.
    std::fs::create_dir_all(&second.home).unwrap();
    let key = second.home.join("root.key");
    std::fs::copy(first.home.join("root.key"), &key).unwrap();
    set_private(&key);
    let shown = explain(&second);
    execute(&url, &format!("DROP SCHEMA {schema} CASCADE;"));
    assert!(shown.contains("trusted (your answer)"), "{shown}");
}

/// Leaves `path` readable only by its owner, as a key file must be.
fn set_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn two_homes_on_one_database_are_one_person_and_keep_no_identity_file() {
    let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
        eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    let (schema, store) = scratch(&url, "identity");
    let first_home = place("postgres-identity-first");
    let second_home = place("postgres-identity-second");
    let server = FakeServer::start(vec![Reply::text("One."), Reply::text("Two.")]).unwrap();
    let run = |place: &Place| {
        let child = start_with(
            place,
            &server,
            &["--model", "fake-model", "hello"],
            &[("GLAVE_STORE_URL", &store)],
        );
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", text(&output));
        assert!(!place.home.join("identity").exists());
    };
    run(&first_home);
    // Servers on one database share its root, as they would a KMS.
    std::fs::create_dir_all(&second_home.home).unwrap();
    let root = second_home.home.join("root.key");
    std::fs::copy(first_home.home.join("root.key"), &root).unwrap();
    set_private(&root);
    run(&second_home);
    let people = count(
        &url,
        &format!("SELECT count(*) FROM {schema}.glave_settings WHERE key = 'person'"),
    );
    execute(&url, &format!("DROP SCHEMA {schema} CASCADE;"));
    assert_eq!(people, 1, "both homes are the database's one person");
}
