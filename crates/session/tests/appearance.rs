//! A person's accent (`specs/model.md`, "The look"): plain until they choose, their choice kept
//! in their settings ledger so every server on one store reads the same, said as the light and
//! dark values to draw it in, and theirs alone.

use glave_event::{Accent, Color};
use glave_ports::Caller;
use glave_session::protocol::{Reply, Request, Shades};
use glave_session::signin::person;

use crate::support::*;

fn blue() -> Color {
    Color::new("#1558d6").unwrap()
}

/// Someone signed in from elsewhere, a person of their own.
fn grace() -> Caller {
    Caller {
        person: Some(person("https://id.example.com", "grace").unwrap()),
        ..Caller::default()
    }
}

async fn appearance(server: &Tested<Shared>, caller: &Caller) -> (Accent, Option<Shades>) {
    match server.request_as(caller, Request::AppearanceGet {}).await {
        Ok(Reply::Appearance { chosen, shades }) => (chosen, shades),
        other => panic!("the caller's appearance, not {other:?}"),
    }
}

async fn pick(server: &Tested<Shared>, caller: &Caller, accent: Accent) -> Option<Shades> {
    match server
        .request_as(caller, Request::AppearanceSet { accent })
        .await
    {
        Ok(Reply::Appearance { shades, .. }) => shades,
        other => panic!("the caller's appearance, not {other:?}"),
    }
}

#[test]
fn plain_until_the_person_chooses_then_theirs_on_every_server_and_theirs_alone() {
    let (one, other) = (folder("appearance-one"), folder("appearance-other"));
    block_on!(async {
        let log = Shared::default();
        let (first, _) = Setup::new(Vec::new()).build_on(&one, log.clone()).await;
        let (second, _) = Setup::new(Vec::new()).build_on(&other, log).await;
        let (ada, grace) = (Caller::default(), grace());
        assert_eq!(appearance(&first, &ada).await, (Accent::Plain, None));

        let exact = Accent::Color {
            color: blue(),
            adjust: false,
        };
        let shades = pick(&first, &ada, exact).await;
        assert_eq!(
            shades,
            Some(Shades {
                light: blue(),
                dark: blue()
            }),
            "not tuned, the exact colour in both"
        );
        let (chosen, read) = appearance(&second, &ada).await;
        assert_eq!(read, shades, "another server on the store reads the same");
        assert_eq!(chosen, exact);
        assert_eq!(appearance(&first, &grace).await, (Accent::Plain, None));

        let tuned = pick(
            &first,
            &ada,
            Accent::Color {
                color: blue(),
                adjust: true,
            },
        )
        .await
        .unwrap();
        assert_ne!(tuned.dark, blue(), "too dark to read in dark, so tuned");

        assert_eq!(pick(&first, &ada, Accent::Plain).await, None);
        assert_eq!(appearance(&second, &ada).await, (Accent::Plain, None));
    });
}

#[test]
fn a_colour_is_read_only_as_one() {
    let parsed: Result<Request, _> = serde_json::from_value(serde_json::json!({
        "method": "appearance/set",
        "params": {"accent": {"color": {"color": "url(evil)", "adjust": true}}},
    }));
    assert!(parsed.is_err(), "only #rrggbb ever reaches a style");
}
