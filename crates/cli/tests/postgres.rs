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

    use glave_provider_openai::fake::{FakeServer, Reply};
    use tokio_postgres::NoTls;

    use super::support::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Runs `query` on the test database, returning the first column of the first row.
    fn count(url: &str, query: &str) -> i64 {
        block_on(async {
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
        block_on(async {
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
}
