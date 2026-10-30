//! A person's filters among a router's providers (`specs/gateway.md`, "A person's filters"):
//! none until they choose, their choice kept in their settings ledger so every server on one
//! store reads the same, what no one can have meant refused, and read for each call of their
//! work as the gateway asks, refused while no server answers.

use std::sync::Arc;

use glave_event::RawEvent;
use glave_event::{Filters, MaxPrice, Serve, Sort};
use glave_gateway::Filtering;
use glave_ports::{Caller, EventLog, Failure, Payer, ProjectKind, ProviderError, Scope};
use glave_session::PersonFilters;
use glave_session::protocol::{Reply, Request, code};
use glave_session::signin::person;

use crate::support::*;

const FLASH: &str = "deepseek/deepseek-v4.1-flash";

/// Someone signed in from elsewhere, a person of their own.
fn grace() -> Caller {
    Caller {
        person: Some(person("https://id.example.com", "grace").unwrap()),
        ..Caller::default()
    }
}

fn us_only() -> Filters {
    Filters {
        based_in: vec!["US".into()],
        ..Filters::default()
    }
}

/// `caller`'s filters, as `server` says them, and what it says Europe is.
async fn filters(server: &Tested<Shared>, caller: &Caller) -> (Filters, Vec<String>) {
    match server.request_as(caller, Request::FiltersGet {}).await {
        Ok(Reply::Filters { filters, europe }) => (filters, europe),
        other => panic!("the caller's filters, not {other:?}"),
    }
}

async fn set(server: &Tested<Shared>, caller: &Caller, filters: Filters) {
    let set = server
        .request_as(caller, Request::FiltersSet { filters })
        .await;
    assert!(matches!(set, Ok(Reply::Filters { .. })), "{set:?}");
}

#[test]
fn none_until_the_person_chooses_then_theirs_on_every_server_and_theirs_alone() {
    let (one, other) = (folder("filters-one"), folder("filters-other"));
    block_on!(async {
        let log = Shared::default();
        let (first, _) = Setup::new(Vec::new()).build_on(&one, log.clone()).await;
        let (second, _) = Setup::new(Vec::new()).build_on(&other, log).await;
        let (ada, grace) = (Caller::default(), grace());
        let (none, europe) = filters(&first, &ada).await;
        assert_eq!(none, Filters::default());
        assert_eq!(europe.len(), 32);
        assert!(
            ["DE", "FR", "GB", "CH", "NO"]
                .iter()
                .all(|c| europe.iter().any(|e| e == c))
        );

        let mut chosen = us_only();
        chosen.sort = Some(Sort::Speed);
        chosen
            .serves
            .insert(FLASH.into(), Serve::Provider("fireworks".into()));
        set(&first, &ada, chosen.clone()).await;
        assert_eq!(filters(&second, &ada).await.0, chosen);
        assert_eq!(filters(&second, &grace).await.0, Filters::default());
    });
}

#[test]
fn a_place_that_is_not_a_country_a_provider_that_is_not_a_slug_or_no_price_is_refused() {
    let dir = folder("filters-refused");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new())
            .build_on(&dir, Shared::default())
            .await;
        let ada = Caller::default();
        for (filters, says) in [
            (
                Filters {
                    based_in: vec!["usa".into()],
                    ..Filters::default()
                },
                "\"usa\" is not a country: name each by its two-letter code, such as US",
            ),
            (
                Filters {
                    avoid: vec!["DeepSeek".into()],
                    ..Filters::default()
                },
                "\"DeepSeek\" is not a provider: name each as the router does, such as deepinfra",
            ),
            (
                Filters {
                    max_price: Some(MaxPrice {
                        input: None,
                        output: None,
                    }),
                    ..Filters::default()
                },
                "a price limit says what input, output or both may cost at most",
            ),
        ] {
            let Err(refused) = server
                .request_as(&ada, Request::FiltersSet { filters })
                .await
            else {
                panic!("refused");
            };
            assert_eq!(refused.code, code::INVALID_PARAMS);
            assert_eq!(refused.message, says);
        }
        assert_eq!(filters(&server, &ada).await.0, Filters::default());
    });
}

#[test]
fn the_gateway_reads_the_filters_of_the_person_whose_work_a_call_is() {
    let dir = folder("filters-calls");
    block_on!(async {
        let kept = Arc::new(PersonFilters::default());
        let mut setup = Setup::new(Vec::new());
        setup.filters = Some(Arc::clone(&kept));
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let org = scope().org();
        let ada = Payer {
            org,
            person: server.person(),
        };
        assert_eq!(kept.filters(ada).await, Ok(None), "none chosen");
        set(&server, &Caller::default(), us_only()).await;
        assert_eq!(kept.filters(ada).await, Ok(Some(us_only())));
        let local = Payer { org, person: None };
        assert_eq!(
            kept.filters(local).await,
            Ok(Some(us_only())),
            "the person the server works for"
        );
        let grace = Payer {
            org,
            person: grace().person,
        };
        assert_eq!(kept.filters(grace).await, Ok(None), "theirs alone");
    });
}

#[test]
fn filters_no_server_answers_refuse_every_call_that_asks() {
    block_on!(async {
        let kept = PersonFilters::default();
        let payer = Payer {
            org: scope().org(),
            person: None,
        };
        let Err(ProviderError::Unserved { failure, reason }) = kept.filters(payer).await else {
            panic!("refused");
        };
        assert_eq!(failure, Failure::Unavailable);
        assert!(reason.contains("isn't ready"), "{reason}");
    });
}

#[test]
fn filters_this_glave_cannot_read_refuse_rather_than_count_as_none() {
    let dir = folder("filters-unreadable");
    block_on!(async {
        let log = Shared::default();
        let kept = Arc::new(PersonFilters::default());
        let mut setup = Setup::new(Vec::new());
        setup.filters = Some(Arc::clone(&kept));
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let ada = Caller::default();
        // Read once, which makes the person's own project the ledger is kept in.
        let Ok(Reply::Filters { .. }) = server.request_as(&ada, Request::FiltersGet {}).await
        else {
            panic!("none yet");
        };
        let Ok(Reply::Projects { projects, .. }) = server
            .request_as(&ada, Request::ProjectList { limit: None })
            .await
        else {
            panic!("the projects");
        };
        let personal = projects
            .iter()
            .find(|project| project.kind == ProjectKind::Personal)
            .unwrap();
        let person = server.person().unwrap();
        let ledger = Scope::of_owner(scope().org(), personal, person).unwrap();
        // A later glave's filters, which this one cannot read.
        let raw = RawEvent::from_json(
            r#"{"glave":1,"id":"0192f5c2-7000-7000-8000-000000000099","thread":"00000000-0000-7000-8000-000000000008","seq":1,"at":"2026-10-30T10:00:00.000Z","type":"filters.set","v":99,"data":{}}"#,
        )
        .unwrap();
        log.append(&ledger, &[raw]).await.unwrap();
        let Err(refused) = server.request_as(&ada, Request::FiltersGet {}).await else {
            panic!("refused");
        };
        assert_eq!(
            refused.message,
            "glave can't read your filters among providers: choose them again"
        );
        let payer = Payer {
            org: scope().org(),
            person: Some(person),
        };
        assert!(kept.filters(payer).await.is_err(), "never taken for none");
    });
}

#[test]
fn once_its_server_is_gone_every_call_that_asks_is_refused() {
    let dir = folder("filters-gone");
    block_on!(async {
        let kept = Arc::new(PersonFilters::default());
        let mut setup = Setup::new(Vec::new());
        setup.filters = Some(Arc::clone(&kept));
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let payer = Payer {
            org: scope().org(),
            person: server.person(),
        };
        assert_eq!(kept.filters(payer).await, Ok(None));
        drop(server);
        let Err(ProviderError::Unserved { reason, .. }) = kept.filters(payer).await else {
            panic!("refused");
        };
        assert!(reason.contains("stopping"), "{reason}");
    });
}
