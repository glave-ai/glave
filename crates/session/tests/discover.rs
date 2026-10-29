//! What `server/discover` says exists beyond a person's own server, each from the server's
//! configuration: nothing on a server that works for one person, and an organization and
//! workers where an identity provider and workers are set up.

use std::sync::Arc;

use glave_computer::LocalComputers;
use glave_harness::Workers;
use glave_ports::computer::{Environment, Sources};
use glave_session::protocol::{Exists, Reply, Request, ServerInfo};
use glave_session::{Answering, OnWorkers};

use crate::support::*;

/// What `server` says it is.
async fn discovered(server: &Tested) -> ServerInfo {
    match server.request(Request::Discover {}).await.unwrap() {
        Reply::Server(info) => info,
        other => panic!("server info, not {other:?}"),
    }
}

#[test]
fn a_server_for_one_person_says_nothing_else_exists() {
    let dir = folder("discover-none");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let info = discovered(&server).await;
        assert_eq!(info.exists, Exists::default());
        let sent = serde_json::to_value(&info).unwrap();
        assert!(sent.get("exists").is_none(), "{sent}");
    });
}

#[test]
fn a_server_with_an_identity_provider_and_workers_says_both_exist() {
    let dir = folder("discover-organization");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        setup.workers = Some(OnWorkers {
            workers: Workers {
                computers: Arc::new(LocalComputers::new(dir.join("workers"), Vec::new())),
                sources: Sources::Git {
                    url: dir.join("origin").to_string_lossy().into_owned(),
                    at: None,
                },
                attachments: Vec::new(),
            },
            environment: Environment::host(),
            project: scope().project(),
        });
        let (server, _) = setup.build(&dir).await;
        let info = discovered(&server).await;
        let both = Exists {
            organization: true,
            workers: true,
        };
        assert_eq!(info.exists, both);
        let sent = serde_json::to_value(&info).unwrap();
        assert_eq!(
            sent["exists"],
            serde_json::json!({"organization": true, "workers": true})
        );
    });
}

#[test]
fn an_identity_provider_alone_makes_an_organization_without_workers() {
    let dir = folder("discover-no-workers");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build(&dir).await;
        let info = discovered(&server).await;
        let organization = Exists {
            organization: true,
            workers: false,
        };
        assert_eq!(info.exists, organization);
    });
}
