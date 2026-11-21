//! `cargo xtask connectors`: each connector of glave's catalog (`crates/session/connectors.toml`)
//! still offers what the catalog says of it (`specs/session.md`, "Connectors"), read as glave
//! reads it, through glave-mcp: a `dcr` one's sign-in found as MCP's authorization says (its
//! protected resource metadata, its authorization server's metadata, and a registration
//! endpoint there), and a `none` one started, answering MCP's handshake with no one signed in.
//! Nothing is registered and no one signs in. It reaches the vendors, so it is never part of
//! the gate: a weekly workflow runs it (`.github/workflows/connectors.yml`), and a vendor's bad
//! day never holds up a merge.

use std::collections::BTreeMap;
use std::time::Duration;

use glave_mcp::{Limits, McpServer, Reachable, sign_in_found};
use glave_ports::auth::Auth;
use glave_ports::spoken;
use toml::{Table, Value};

/// The catalog, from the workspace's root.
const CATALOG: &str = "crates/session/connectors.toml";
/// How many connectors are read at once.
const AT_ONCE: usize = 8;
/// The longest one connector's reading may take: a few requests to the vendor's servers.
const LIMIT: Duration = Duration::from_secs(30);

/// A connector, as far as this check reads it.
struct Connector {
    id: String,
    url: String,
    sign_in: String,
}

/// Reads every connector of the catalog, one line each, and fails naming each that drifted
/// and why.
pub(crate) fn check() -> Result<(), String> {
    let path = crate::workspace::root()?.join(CATALOG);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    let connectors = connectors(&text)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;
    let mut drifted = Vec::new();
    for batch in connectors.chunks(AT_ONCE) {
        for (id, read) in runtime.block_on(read_all(batch)) {
            match read {
                Ok(()) => println!("ok       {id}"),
                Err(why) => {
                    println!("drifted  {id}: {why}");
                    drifted.push(format!("{id}: {why}"));
                }
            }
        }
    }
    if drifted.is_empty() {
        println!(
            "all {} connectors read as the catalog says",
            connectors.len()
        );
        return Ok(());
    }
    Err(format!(
        "{} of {} connectors drifted from the catalog:\n  {}",
        drifted.len(),
        connectors.len(),
        drifted.join("\n  ")
    ))
}

/// The connectors of the catalog `text`.
fn connectors(text: &str) -> Result<Vec<Connector>, String> {
    let catalog: Table = text
        .parse()
        .map_err(|err| format!("the catalog does not read: {err}"))?;
    let listed = catalog
        .get("connector")
        .and_then(Value::as_array)
        .ok_or("the catalog lists no connectors")?;
    listed
        .iter()
        .map(|connector| {
            let field = |name: &str| {
                connector
                    .get(name)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("a connector of the catalog has no {name}"))
            };
            Ok(Connector {
                id: field("id")?,
                url: field("url")?,
                sign_in: field("sign_in")?,
            })
        })
        .collect()
}

/// Each of `batch` read at once, in its order.
async fn read_all(batch: &[Connector]) -> Vec<(String, Result<(), String>)> {
    let reading: Vec<_> = batch
        .iter()
        .map(|connector| {
            let (url, sign_in) = (connector.url.clone(), connector.sign_in.clone());
            let id = connector.id.clone();
            tokio::spawn(async move {
                let read = tokio::time::timeout(LIMIT, read(&id, &url, &sign_in)).await;
                read.unwrap_or_else(|_| Err(format!("no answer in {}", spoken(LIMIT))))
            })
        })
        .collect();
    let mut read = Vec::new();
    for (connector, reading) in batch.iter().zip(reading) {
        // Each ends within its own limit, above.
        let done = reading.await;
        let done = done.unwrap_or_else(|err| Err(format!("its reading failed: {err}")));
        read.push((connector.id.clone(), done));
    }
    read
}

/// Whether the connector `id` at `url` offers what its `sign_in` says.
async fn read(id: &str, url: &str, sign_in: &str) -> Result<(), String> {
    match sign_in {
        "dcr" => sign_in_found(url, Reachable::OnlyPublic)
            .await
            .map_err(|err| err.to_string()),
        "none" => {
            let limits = Limits {
                startup: LIMIT,
                reachable: Reachable::OnlyPublic,
                ..Limits::default()
            };
            let server = McpServer::remote(id, url, &BTreeMap::new(), Auth::None, limits);
            let started = server.started().await;
            server.close().await;
            started.map_err(|err| err.to_string())
        }
        other => Err(format!("its sign-in {other:?} is not one this check reads")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_connector_of_the_catalog_is_one_this_check_reads() {
        let text = std::fs::read_to_string(format!("../{CATALOG}")).unwrap();
        let connectors = connectors(&text).unwrap();
        assert!(connectors.len() >= 100, "{}", connectors.len());
        for connector in &connectors {
            let known = ["dcr", "none"].contains(&connector.sign_in.as_str());
            assert!(known, "{}: {}", connector.id, connector.sign_in);
        }
    }
}
