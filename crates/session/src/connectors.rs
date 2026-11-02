//! glave's catalog of connectors (`specs/session.md`, "Connectors"): official remote MCP
//! servers, each hosted by the vendor whose service it reaches, that a person connects with one
//! click. It is `connectors.toml`, built into glave and versioned with it, never fetched. Each
//! description and category name is a Fluent message, written in the caller's language where
//! the server answers; a vendor's name never is.

use serde::Deserialize;
use thiserror::Error;

use crate::protocol::Told;

/// The catalog, as glave was built with it.
const BUILT_IN: &str = include_str!("../connectors.toml");

/// Why the catalog could not be read: only a catalog glave was built with, which its tests read.
#[derive(Debug, Error)]
#[error("glave's catalog of connectors does not read: {0}")]
pub struct CatalogError(String);

/// glave's catalog of connectors.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Its categories, in the order a client shows them.
    #[serde(rename = "category")]
    pub categories: Vec<Category>,
    /// Its connectors, in the order a client shows them within their categories.
    #[serde(rename = "connector")]
    pub connectors: Vec<Connector>,
}

/// A category of connectors.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    /// Its name, in English.
    pub name: String,
}

/// A connector: an official remote MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connector {
    /// Stable: it names the connector to `[mcp] catalog`, to its tools (`mcp__<id>__<tool>`)
    /// and in the accounts ledger.
    pub id: String,
    /// The vendor's name for it, never translated.
    pub name: String,
    /// Its server's address, streamable HTTP.
    pub url: String,
    /// Its category's id.
    pub category: String,
    pub auth: Auth,
    /// The vendor's own page about it, which shows the server is theirs.
    pub docs: String,
    /// What it reaches, in English.
    pub description: String,
}

/// How a connector is signed in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Auth {
    /// It asks no one to sign in.
    None,
    /// The person signs in, glave registering there as a client (dynamic client registration).
    Dcr,
}

impl Catalog {
    /// The catalog glave was built with.
    pub fn built_in() -> Result<Self, CatalogError> {
        Self::read(BUILT_IN)
    }

    /// The connector called `id`, if the catalog has one.
    pub fn connector(&self, id: &str) -> Option<&Connector> {
        self.connectors.iter().find(|connector| connector.id == id)
    }

    /// The category called `id`, if the catalog has one.
    fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|category| category.id == id)
    }

    /// The catalog `text` holds, once each id is a plain one and only its own, each connector's
    /// category is the catalog's, and each address is one glave reaches servers at.
    fn read(text: &str) -> Result<Self, CatalogError> {
        let catalog: Self = toml::from_str(text).map_err(|e| CatalogError(e.to_string()))?;
        let ids = catalog.categories.iter().map(|category| &category.id);
        let ids = ids.chain(catalog.connectors.iter().map(|connector| &connector.id));
        let mut seen = std::collections::BTreeSet::new();
        for id in ids {
            let plain = id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if id.is_empty() || !plain || !seen.insert(id) {
                return Err(CatalogError(format!("{id:?} is not a plain id of its own")));
            }
        }
        for connector in &catalog.connectors {
            let id = &connector.id;
            if catalog.category(&connector.category).is_none() {
                return Err(CatalogError(format!(
                    "{id} is in no category of the catalog"
                )));
            }
            let https = connector.url.starts_with("https://");
            if !https || glave_mcp::address(&connector.url).is_none() {
                return Err(CatalogError(format!("{id} is at no https address")));
            }
        }
        Ok(catalog)
    }
}

impl Connector {
    /// What it reaches, as the caller reads it.
    pub(crate) fn told(&self) -> Told {
        Told::catalogued(format!("connector-{}", self.id), self.description.clone())
    }
}

impl Category {
    /// Its name, as the caller reads it.
    pub(crate) fn told(&self) -> Told {
        Told::catalogued(format!("connector-category-{}", self.id), self.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_catalog_reads_and_says_each_thing_once() {
        let catalog = Catalog::built_in().unwrap();
        assert!(
            catalog.connectors.len() >= 100,
            "{}",
            catalog.connectors.len()
        );
        for category in &catalog.categories {
            let used = catalog
                .connectors
                .iter()
                .any(|it| it.category == category.id);
            assert!(used, "{} holds no connector", category.id);
        }
        let urls: std::collections::BTreeSet<_> = catalog
            .connectors
            .iter()
            .filter_map(|it| glave_mcp::address(&it.url))
            .collect();
        assert_eq!(urls.len(), catalog.connectors.len(), "each server once");
        for connector in &catalog.connectors {
            assert!(connector.docs.starts_with("https://"), "{}", connector.id);
        }
    }

    #[test]
    fn a_catalog_with_a_broken_entry_is_refused() {
        let entry = |id: &str, category: &str, url: &str| {
            format!(
                "[[category]]\nid = \"dev\"\nname = \"Dev\"\n\n[[connector]]\nid = \"{id}\"\n\
                 name = \"X\"\nurl = \"{url}\"\ncategory = \"{category}\"\nauth = \"dcr\"\n\
                 docs = \"https://x.example.com\"\ndescription = \"X.\"\n"
            )
        };
        assert!(Catalog::read(&entry("x", "dev", "https://mcp.example.com/mcp")).is_ok());
        for broken in [
            entry("X Y", "dev", "https://mcp.example.com/mcp"),
            entry("dev", "dev", "https://mcp.example.com/mcp"),
            entry("x", "other", "https://mcp.example.com/mcp"),
            entry("x", "dev", "http://mcp.example.com/mcp"),
            entry("x", "dev", "https://mcp.example.com/mcp").replace("dcr", "key"),
        ] {
            assert!(Catalog::read(&broken).is_err(), "{broken}");
        }
    }
}
