use serde_json::json;

use super::*;
use crate::payload::approval::{ApprovalRequested, decode_requested};

fn deletes() -> Risk {
    Risk::DeletesOutside {
        path: "/home/ana/notes".into(),
    }
}

#[test]
fn a_caution_reads_as_it_was_written_before_it_was_typed() {
    let risky = Caution::of(Some(deletes()), None).unwrap();
    assert_eq!(
        risky.because().as_deref(),
        Some("it deletes /home/ana/notes, outside the project")
    );
    assert_eq!(
        risky.sentences().as_deref(),
        Some("It deletes /home/ana/notes, outside the project.")
    );
    let outside = Caution::of(Some(deletes()), Some(Lifted::Sandbox)).unwrap();
    assert_eq!(
        outside.because().as_deref(),
        Some(
            "it deletes /home/ana/notes, outside the project, and it runs outside the sandbox, \
             where it can read and change any file you can"
        )
    );
    let network = Caution::of(None, Some(Lifted::Network)).unwrap();
    assert_eq!(
        network.sentences().as_deref(),
        Some("It can reach the internet, which commands here otherwise cannot.")
    );
    assert_eq!(Caution::of(None, None), None);
}

#[test]
fn how_glave_settled_it_follows_why() {
    let settled = |settled| Caution {
        settled: Some(settled),
        ..Caution::of(Some(Risk::Pushes), None).unwrap()
    };
    let cleared = settled(Settled::Cleared {
        why: Some(" the person asked to push ".into()),
    });
    assert_eq!(
        cleared.sentences().as_deref(),
        Some("It pushes to a remote. Allowed without asking: the person asked to push.")
    );
    assert_eq!(cleared.because().as_deref(), Some("it pushes to a remote"));
    let plain = settled(Settled::Cleared { why: None });
    assert_eq!(
        plain.sentences().as_deref(),
        Some("It pushes to a remote. Allowed without asking.")
    );
    let declined = Caution {
        risk: None,
        lifted: None,
        settled: Some(Settled::Declined),
    };
    assert_eq!(declined.because(), None);
    assert_eq!(
        declined.sentences().as_deref(),
        Some("Declined without asking: Full access never shows the model a secret.")
    );
}

#[test]
fn values_are_written_where_their_names_are_and_never_read_as_names() {
    let risk = Risk::ReadsSecrets {
        path: "{ $path }".into(),
    };
    assert_eq!(
        risk.english(),
        "it reads { $path }, which usually holds secrets"
    );
    assert_eq!(fill("{ $missing } stays", &[]), "{ $missing } stays");
    assert_eq!(fill("no values", &[("x", "y")]), "no values");
}

#[test]
fn a_clause_reads_as_a_sentence_of_its_own() {
    assert_eq!(
        sentence("this file usually holds secrets; the model would see them"),
        "This file usually holds secrets; the model would see them."
    );
    assert_eq!(sentence(" Already one. "), "Already one.");
    assert_eq!(sentence(""), "");
}

#[test]
fn every_risk_is_found_again_by_its_id() {
    for risk in Risk::every() {
        let written = serde_json::to_value(&risk).unwrap();
        assert_eq!(written["says"], risk.id());
        assert_eq!(serde_json::from_value::<Risk>(written).unwrap(), risk);
    }
}

#[test]
fn a_request_reads_with_its_reason_and_without_one() {
    let read = |data: serde_json::Value| {
        let raw = serde_json::value::RawValue::from_string(data.to_string()).unwrap();
        decode_requested(1, &raw).unwrap().unwrap()
    };
    let before: ApprovalRequested = read(json!({
        "call_id": "call_1", "tool": "bash", "summary": "bash: git push", "secret": false,
        "caution": "It pushes to a remote."
    }));
    assert_eq!(before.reason, None);
    let typed = read(json!({
        "call_id": "call_1", "tool": "bash", "summary": "bash: rm -rf ~/notes", "secret": false,
        "caution": "It deletes /home/ana/notes, outside the project.",
        "reason": {
            "risk": {"says": "risk-deletes-outside", "path": "/home/ana/notes"},
            "lifted": "sandbox",
            "settled": {"cleared": {"why": "asked"}}
        }
    }));
    let reason = typed.reason.unwrap();
    assert_eq!(reason.risk, Some(deletes()));
    assert_eq!(reason.lifted, Some(Lifted::Sandbox));
    assert_eq!(
        reason.settled,
        Some(Settled::Cleared {
            why: Some("asked".into())
        })
    );
    let declined = serde_json::to_value(Settled::Declined).unwrap();
    assert_eq!(declined, json!("declined"));
}
