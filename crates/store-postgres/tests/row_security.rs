//! Row-level security (`specs/storage.md`): the database itself keeps each organization's rows
//! apart, so a query that forgets to name its organization still reads and writes only the
//! organization its connection is set to, and none while it is set to none.

use std::time::Duration;

use glave_event::{
    At, EventId, Header, MessageAdded, Origin, OriginKind, Part, Principal, PrincipalId, Producer,
    RawEvent, Role, Seq, ThreadId, ThreadStarted, encode,
};
use glave_ports::{
    BlobStore as _, EventLog as _, FolderKey, Holder, Leased, Leases as _, OrgId, Pieces, Project,
    ProjectId, ProjectKind, Projects as _, Scope,
};
use glave_store_postgres::{Store, StoreError};
use tokio_postgres::Client;
use uuid::{Builder, Uuid};

use crate::support::{Scratch, block_on};

/// Every table that holds organizations' rows.
const TABLES: [&str; 10] = [
    "events",
    "blobs",
    "blob_threads",
    "org_keys",
    "project_keys",
    "project_records",
    "project_folders",
    "threads",
    "leases",
    "event_search",
];

/// How many rows of `table` the connection sees, and how many of them are not `org`'s, with no
/// organization named in the query.
async fn seen(client: &Client, schema: &str, table: &str, org: Uuid) -> (i64, i64) {
    let row = client
        .query_one(
            &format!("SELECT count(*), count(*) FILTER (WHERE org <> $1) FROM {schema}.{table}"),
            &[&org],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// The events of a thread with a message in it: rows in the log, the search index and the
/// threads view.
fn thread_of(org: u128) -> (ThreadId, Vec<RawEvent>) {
    let thread = ThreadId::try_from(id(org, 3)).unwrap();
    let header = |seq: u64| Header {
        id: EventId::try_from(id(org, 10 + u128::from(seq))).unwrap(),
        thread,
        seq: Seq::new(seq).unwrap(),
        at: At::try_from("2026-10-29T09:00:00.000Z".to_owned()).unwrap(),
    };
    let started = ThreadStarted {
        producer: Producer {
            name: "glave".into(),
            version: "0.0.0".into(),
        },
        fork: None,
        parent: None,
        for_: None,
        agent: None,
        origin: Origin::of(OriginKind::Prompt),
    };
    let said = MessageAdded {
        role: Role::User,
        parts: vec![Part::Text {
            text: "Plant the marigolds by the gate.".into(),
        }],
        author: None,
        thought: None,
    };
    let events = vec![
        encode(header(1), &started).unwrap(),
        encode(header(2), &said).unwrap(),
    ];
    (thread, events)
}

/// A UUID v7 for organization `org`'s `n`th id.
fn id(org: u128, n: u128) -> Uuid {
    let random = (org * 1_000 + n).to_be_bytes();
    let mut ten = [0u8; 10];
    ten.copy_from_slice(&random[6..]);
    Builder::from_unix_timestamp_millis(1_791_000_000_000, &ten).into_uuid()
}

/// Rows of two organizations in every table, through the store, which names each one.
async fn two_organizations(scratch: &Scratch) -> Vec<Uuid> {
    let store = scratch.store().await.unwrap();
    let mut orgs = Vec::new();
    for n in [1, 2] {
        let org = OrgId::from_uuid(id(n, 1));
        let scope = Scope::glaves_own(org, ProjectId::from_uuid(id(n, 2)));
        let (thread, events) = thread_of(n);
        store.log().append(&scope, &events).await.unwrap();
        store
            .blobs()
            .put(
                &scope,
                Some(thread),
                "text/plain",
                1024,
                Box::new(Pieces::from(b"the gate's plan".to_vec())),
            )
            .await
            .unwrap();
        let created = At::try_from("2026-10-29T09:00:00.000Z".to_owned()).unwrap();
        let project = Project {
            id: scope.project(),
            kind: ProjectKind::Project,
            name: "garden".into(),
            owner: Principal::person(PrincipalId::try_from(id(n, 4)).unwrap()),
            created_at: created,
            updated_at: created,
        };
        store
            .projects()
            .folder(org, &[FolderKey::Path("/code/garden".into())], &project)
            .await
            .unwrap();
        let holder = Holder::from_uuid(id(n, 5));
        let leased = store
            .leases()
            .acquire(org, Leased::Scheduler, holder, Duration::from_secs(30))
            .await
            .unwrap();
        assert!(leased.is_some());
        orgs.push(org.as_uuid());
    }
    orgs
}

/// Set to `ours`, the connection may write no row of `theirs`, and a delete naming no
/// organization reaches only its own.
async fn writes_stay_in(client: &Client, scratch: &Scratch, [ours, theirs]: [Uuid; 2]) {
    let schema = &scratch.schema;
    client
        .execute(
            "SELECT set_config('glave.org', $1, false)",
            &[&ours.to_string()],
        )
        .await
        .unwrap();
    let written = client
        .execute(
            &format!(
                "INSERT INTO {schema}.leases (org, key, holder, expires) VALUES ($1, 'x', $1, 0)"
            ),
            &[&theirs],
        )
        .await;
    let refused = written.err().and_then(|error| error.code().cloned());
    assert_eq!(
        refused,
        Some(tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "a row of another organization is refused by the policy"
    );
    let changed = client
        .execute(&format!("DELETE FROM {schema}.events"), &[])
        .await
        .unwrap();
    let left: i64 = scratch
        .admin
        .query_one(
            &format!("SELECT count(*) FROM {schema}.events WHERE org = $1"),
            &[&theirs],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        changed > 0 && left > 0,
        "a delete reaches only its own organization"
    );
}

#[test]
fn a_query_that_names_no_organization_sees_only_the_one_its_connection_is_set_to() {
    block_on!(async {
        let Some(scratch) = Scratch::new("row_security").await else {
            return;
        };
        let orgs = two_organizations(&scratch).await;
        let schema = scratch.schema.clone();
        for table in TABLES {
            let both: i64 = scratch
                .admin
                .query_one(
                    &format!("SELECT count(DISTINCT org) FROM {schema}.{table}"),
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(both, 2, "{table} holds rows of both organizations");
        }

        // The store's own role, as glave signs in.
        let (client, connection) = scratch.config.connect(tokio_postgres::NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        for table in TABLES {
            let (rows, _) = seen(&client, &schema, table, Uuid::nil()).await;
            assert_eq!(rows, 0, "{table} shows nothing with no organization set");
        }
        for org in &orgs {
            client
                .execute(
                    "SELECT set_config('glave.org', $1, false)",
                    &[&org.to_string()],
                )
                .await
                .unwrap();
            for table in TABLES {
                let (rows, others) = seen(&client, &schema, table, *org).await;
                assert!(rows > 0, "{table} shows the organization's own rows");
                assert_eq!(others, 0, "{table} shows no other organization's rows");
            }
        }
        writes_stay_in(&client, &scratch, [orgs[0], orgs[1]]).await;
        drop(client);
        let _ = connection.await;
        scratch.drop_schema().await;
    });
}

#[test]
fn a_role_that_skips_row_level_security_is_refused() {
    block_on!(async {
        let Some(scratch) = Scratch::new("bypassing").await else {
            return;
        };
        let mut config: glave_store_postgres::Config =
            std::env::var("GLAVE_TEST_POSTGRES_ADMIN_URL")
                .unwrap()
                .parse()
                .unwrap();
        config.options(format!("-c search_path={}", scratch.schema));
        let opened = Store::connect_with(
            config,
            scratch.keys(),
            std::sync::Arc::new(scratch.objects.clone()),
        )
        .await;
        scratch.drop_schema().await;
        assert!(
            matches!(opened, Err(StoreError::Schema(ref said)) if said.contains("row-level security")),
            "a superuser's sign-in is refused"
        );
    });
}
