//! Isolation (`specs/invariants.md`, 6 and 17): no organization sees another's data, and no
//! person another's. Two organizations share one store, each served by its own server, with
//! two people each; every method of the session API is called by every person with every other
//! person's ids, and must answer exactly as it does for an id nobody has, so not even whether
//! something exists leaks. Runs on SQLite, and on Postgres when `GLAVE_TEST_POSTGRES_URL`
//! names a database. The methods come from the protocol's one table, through the schema made
//! from it: a method this suite has no entry for fails it.

mod sweep;
mod table;
mod world;

use std::collections::BTreeSet;
use std::sync::Arc;

use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::EventLog;
use glave_session::schema;
use serde_json::Value;

use crate::support::*;
use table::{Kind, NOT_HELD, placeholders, table};
use world::{Parts, World};

/// The protocol's type a param names, through `$ref` or an optional `anyOf`.
fn type_of(param: &Value) -> Option<String> {
    let reference = param.get("$ref").or_else(|| {
        param
            .get("anyOf")?
            .as_array()?
            .iter()
            .find_map(|one| one.get("$ref"))
    })?;
    Some(reference.as_str()?.rsplit('/').next()?.to_owned())
}

#[test]
fn every_method_and_every_id_it_takes_is_in_the_table() {
    let document = schema::document().unwrap();
    let methods = document["methods"].as_object().unwrap();
    let table = table();
    let listed: BTreeSet<&str> = table.iter().map(|(method, _)| *method).collect();
    let served: BTreeSet<&str> = methods.keys().map(String::as_str).collect();
    assert_eq!(
        served.difference(&listed).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "methods the isolation suite does not call: add them to its table"
    );
    assert_eq!(listed, served, "methods the protocol no longer has");
    let owned: BTreeSet<&str> = Kind::ALL.iter().filter_map(|k| k.schema_type()).collect();
    for (method, templates) in &table {
        let params = methods[*method]["params"]["properties"].as_object();
        for (param, schema) in params.into_iter().flatten() {
            let Some(named) = type_of(schema) else {
                continue;
            };
            if !owned.contains(named.as_str()) {
                continue;
            }
            let filled = templates.iter().any(|template| {
                placeholders(template)
                    .iter()
                    .any(|(at, kind)| at == param && kind.schema_type() == Some(named.as_str()))
            });
            assert!(
                filled,
                "{method}'s `{param}` names a {named}: a template must fill it with someone's"
            );
        }
    }
}

/// The whole suite on the store `parts` makes: what the people's own calls name, the sweep,
/// and that nobody's things changed. Every finding is reported at once.
async fn isolated<L: EventLog + Clone + 'static>(name: &str, parts: Parts<L>) {
    let mut world = World::build(name, &parts).await;
    let mut found = sweep::own_calls(&world).await;
    world.sign_in_and_hold().await;
    for (kind, why) in NOT_HELD {
        assert!(
            world
                .people
                .iter()
                .all(|person| !person.ids.contains_key(kind)),
            "{kind:?} is held now ({why} no longer holds): take it off NOT_HELD"
        );
    }
    for kind in Kind::ALL {
        let held = world.people.iter().any(|p| p.ids.contains_key(&kind));
        let excused = NOT_HELD.iter().any(|(not, _)| *not == kind);
        assert!(
            held || excused,
            "nobody holds a {kind:?}: the sweep never names one"
        );
    }
    // Reading a thread may first close what it left open, so the first look is not compared.
    sweep::snapshot(&world).await;
    let before = sweep::snapshot(&world).await;
    found.extend(sweep::sweep(&world).await);
    let after = sweep::snapshot(&world).await;
    for ((person, before), after) in world.people.iter().zip(&before).zip(&after) {
        if before != after {
            found.push(format!(
                "what {} sees of their own changed:\n  before: {before}\n  after: {after}",
                person.name
            ));
        }
    }
    world.settle().await;
    assert!(
        found.is_empty(),
        "{} leaks:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn on_sqlite_no_one_reaches_anothers_data() {
    block_on!(async {
        let root = InMemoryRoot::new().unwrap();
        let store =
            glave_store_sqlite::Store::open_in_memory(KeySettings::new(Arc::new(root))).unwrap();
        let parts = Parts {
            log: store.log(),
            projects: Arc::new(store.projects()),
            eraser: store.keyring(),
        };
        isolated("isolation-sqlite", parts).await;
    });
}

#[test]
fn on_postgres_no_one_reaches_anothers_data() {
    let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
        eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    block_on!(async {
        let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let schema = format!("glave_test_isolation_{}", std::process::id());
        let fresh = format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};");
        admin.batch_execute(&fresh).await.unwrap();
        let mut config: glave_store_postgres::Config = url.parse().unwrap();
        config.options(format!("-c search_path={schema}"));
        let root = InMemoryRoot::new().unwrap();
        let keys = KeySettings::new(Arc::new(root));
        let objects = Arc::new(glave_ports::InMemoryObjects::new());
        let store = glave_store_postgres::Store::connect_with(config, keys, objects)
            .await
            .unwrap();
        let parts = Parts {
            log: store.log(),
            projects: Arc::new(store.projects()),
            eraser: store.keyring(),
        };
        isolated("isolation-postgres", parts).await;
        admin
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE;"))
            .await
            .unwrap();
        drop(admin);
        let _ = connection.await;
    });
}
