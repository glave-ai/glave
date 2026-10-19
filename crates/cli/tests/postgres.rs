//! The binary on Postgres: with GLAVE_STORE_URL (or `[store] url`) naming a database, a turn's
//! history goes there and nowhere on this computer, and `glave threads` reads it back. Runs
//! when `GLAVE_TEST_POSTGRES_URL` names a database, in a schema of its own.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod postgres {
    use std::process::Command;

    use glave_ports::block_on;
    use glave_provider_openai::fake::{FakeServer, Reply};
    use tokio_postgres::NoTls;

    use super::support::*;

    /// Runs `query` on the test database, returning the first column of the first row.
    fn count(url: &str, query: &str) -> i64 {
        first(url, query)
    }

    /// Runs `query` on the test database, returning the first column of the first row.
    fn first<T: for<'a> tokio_postgres::types::FromSql<'a>>(url: &str, query: &str) -> T {
        block_on!(async {
            let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let row = client.query_one(query, &[]).await.unwrap();
            drop(client);
            let _ = connection.await;
            row.get(0)
        })
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
        let schema = format!("glave_test_cli_{}", std::process::id());
        execute(
            &url,
            &format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"),
        );
        let separator = if url.contains('?') { '&' } else { '?' };
        let store = format!("{url}{separator}options=-c%20search_path%3D{schema}");
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
        let events = count(&url, &format!("SELECT count(*) FROM {schema}.events"));
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
        assert_eq!(shown.lines().count(), 1, "{shown}");
        execute(&url, &format!("DROP SCHEMA {schema} CASCADE;"));
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
        let schema = format!("glave_test_cli_identity_{}", std::process::id());
        execute(
            &url,
            &format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"),
        );
        let separator = if url.contains('?') { '&' } else { '?' };
        let store = format!("{url}{separator}options=-c%20search_path%3D{schema}");
        // The first home has the file an earlier glave kept; the second has none.
        let first_home = place("postgres-identity-first");
        let person = "01890a5d-ac96-774b-bcce-b302099a8057";
        let agent = "01890a5d-ac96-774b-bcce-b302099a8058";
        std::fs::create_dir_all(&first_home.home).unwrap();
        std::fs::write(
            first_home.home.join("identity"),
            format!("person {person}\nagent {agent}\n"),
        )
        .unwrap();
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
        let kept: String = first(
            &url,
            &format!("SELECT value FROM {schema}.glave_settings WHERE key = 'person'"),
        );
        let people = count(
            &url,
            &format!("SELECT count(*) FROM {schema}.glave_settings WHERE key = 'person'"),
        );
        execute(&url, &format!("DROP SCHEMA {schema} CASCADE;"));
        assert_eq!(kept, person, "the earlier file's person is the database's");
        assert_eq!(people, 1);
    }
}
