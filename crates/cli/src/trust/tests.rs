use std::collections::BTreeMap;

use glave_config::trust::Decision;
use glave_ports::block_on;

use super::*;

fn entry(path: &str, trusted: bool) -> Entry {
    Entry {
        path: path.to_owned(),
        remote: None,
        items: BTreeMap::from([(
            "mcp:docs".to_owned(),
            Decision {
                definition: r#"{"command":"docs-mcp"}"#.to_owned(),
                trusted,
            },
        )]),
        by: None,
    }
}

/// What the record says of each project: its path and whether its server is trusted.
async fn decided(trust: &Trust) -> Vec<(String, bool)> {
    let record = trust.record().await.unwrap();
    record
        .entries()
        .iter()
        .map(|entry| (entry.path.clone(), entry.items["mcp:docs"].trusted))
        .collect()
}

#[test]
fn a_decision_is_kept_in_the_store_and_the_latest_about_a_project_holds() {
    block_on!(async {
        let (store, trust) = Trust::in_memory().await.unwrap();
        trust.keep(entry("/code/app", true)).await.unwrap();
        trust.keep(entry("/code/web", true)).await.unwrap();
        trust.keep(entry("/code/app", false)).await.unwrap();
        // Read through another handle on the same ledger, as another server would.
        let again = Trust::open(&store, trust.person).await.unwrap();
        assert_eq!(
            decided(&again).await,
            [
                ("/code/web".to_owned(), true),
                ("/code/app".to_owned(), false)
            ]
        );
        let kept = again.record().await.unwrap();
        assert_eq!(
            kept.entries()[0].by,
            Some(Principal::person(trust.person)),
            "who decided is recorded"
        );
    });
}
