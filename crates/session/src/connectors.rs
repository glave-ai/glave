//! glave's catalog of connectors (`specs/session.md`, "Connectors"): official remote MCP
//! servers, each hosted by the vendor whose service it reaches, that a person connects with one
//! click. It is `connectors.toml`, built into glave and versioned with it, never fetched. Each
//! description, and the name of each category and tag (`protocol/connectors.rs`), is a Fluent
//! message, written in the caller's language where the server answers; a vendor's name never is.

use std::collections::BTreeSet;
use std::fmt::Debug;

use serde::Deserialize;
use thiserror::Error;

use crate::protocol::{
    ConnectorCategoryId, ConnectorCompany, ConnectorReturns, ConnectorSignIn, ConnectorTagId, Told,
};

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
    /// The companies that run its connectors.
    #[serde(rename = "company")]
    pub companies: Vec<ConnectorCompany>,
    /// Its connectors, in the order a client shows them within their categories.
    #[serde(rename = "connector")]
    pub connectors: Vec<Connector>,
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
    /// Its company's id.
    pub company: String,
    /// Its server's address, streamable HTTP.
    pub url: String,
    /// Its categories, the first its home.
    pub categories: Vec<ConnectorCategoryId>,
    pub tags: Vec<ConnectorTagId>,
    /// What its tools hand back, as its vendor's docs say.
    pub returns: ConnectorReturns,
    pub sign_in: ConnectorSignIn,
    /// The vendor's own page about it, which shows the server is theirs.
    pub docs: String,
    /// What it reaches, in English.
    pub description: String,
}

/// What a caller narrows `connector/list` by.
#[derive(Debug, Default)]
pub(crate) struct Narrowed {
    pub(crate) company: Option<String>,
    pub(crate) category: Option<ConnectorCategoryId>,
    pub(crate) tag: Option<ConnectorTagId>,
}

impl Narrowed {
    /// Whether `connector` has all that was given.
    pub(crate) fn has(&self, connector: &Connector) -> bool {
        self.company
            .as_ref()
            .is_none_or(|it| *it == connector.company)
            && self
                .category
                .is_none_or(|it| connector.categories.contains(&it))
            && self.tag.is_none_or(|it| connector.tags.contains(&it))
    }
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

    /// The catalog `text` holds, once each id is a plain one and only its own, each company
    /// runs a connector, and each connector has a company of the catalog's, a category, a tag,
    /// words to say, and an address glave reaches servers at.
    fn read(text: &str) -> Result<Self, CatalogError> {
        let catalog: Self = toml::from_str(text).map_err(|e| CatalogError(e.to_string()))?;
        plain_ids(catalog.companies.iter().map(|company| &company.id))?;
        plain_ids(catalog.connectors.iter().map(|connector| &connector.id))?;
        for company in &catalog.companies {
            let id = &company.id;
            if !company.homepage.starts_with("https://") {
                return Err(CatalogError(format!("{id} has no https homepage")));
            }
            if !catalog.connectors.iter().any(|it| it.company == *id) {
                return Err(CatalogError(format!("{id} runs no connector")));
            }
        }
        for connector in &catalog.connectors {
            let id = &connector.id;
            if !catalog
                .companies
                .iter()
                .any(|it| it.id == connector.company)
            {
                return Err(CatalogError(format!(
                    "{id} is of no company of the catalog"
                )));
            }
            each_once(id, "category", &connector.categories)?;
            each_once(id, "tag", &connector.tags)?;
            if connector.description.trim().is_empty() {
                return Err(CatalogError(format!("{id} has no description")));
            }
            let https = connector.url.starts_with("https://");
            if !https || glave_mcp::address(&connector.url).is_none() {
                return Err(CatalogError(format!("{id} is at no https address")));
            }
        }
        Ok(catalog)
    }
}

/// `Err` unless each of `ids` is a plain id, and only its own.
fn plain_ids<'a>(ids: impl Iterator<Item = &'a String>) -> Result<(), CatalogError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        let plain = id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if id.is_empty() || !plain || !seen.insert(id) {
            return Err(CatalogError(format!("{id:?} is not a plain id of its own")));
        }
    }
    Ok(())
}

/// `Err` unless the connector `id` has at least one of `items`, each once.
fn each_once<T: Copy + PartialEq + Debug>(
    id: &str,
    what: &str,
    items: &[T],
) -> Result<(), CatalogError> {
    let again = items
        .iter()
        .enumerate()
        .any(|(i, it)| items.iter().take(i).any(|before| before == it));
    if items.is_empty() || again {
        return Err(CatalogError(format!(
            "{id} needs at least one {what}, each once"
        )));
    }
    Ok(())
}

impl Connector {
    /// What it reaches, as the caller reads it.
    pub(crate) fn told(&self) -> Told {
        Told::catalogued(format!("connector-{}", self.id), self.description.clone())
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
        let urls: BTreeSet<_> = catalog
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
    fn each_category_is_a_home_and_each_tag_holds_a_connector() {
        let catalog = Catalog::built_in().unwrap();
        for category in ConnectorCategoryId::ALL {
            let home = catalog
                .connectors
                .iter()
                .any(|it| it.categories.first() == Some(category));
            assert!(home, "{category:?} is no connector's home");
        }
        for tag in ConnectorTagId::ALL {
            let used = catalog.connectors.iter().any(|it| it.tags.contains(tag));
            assert!(used, "{tag:?} holds no connector");
        }
    }

    #[test]
    fn one_company_can_run_several_connectors() {
        let catalog = Catalog::built_in().unwrap();
        let of = |id: &str| catalog.connector(id).unwrap().company.clone();
        assert_eq!(of("cloudflare-docs"), of("cloudflare-radar"));
        assert_eq!(of("context7"), of("upstash"));
    }

    #[test]
    fn narrowing_keeps_those_that_have_all_that_is_given() {
        let catalog = Catalog::built_in().unwrap();
        let listed = |narrowed: Narrowed| -> Vec<String> {
            let kept = catalog.connectors.iter().filter(|it| narrowed.has(it));
            kept.map(|it| it.id.clone()).collect()
        };
        assert_eq!(listed(Narrowed::default()).len(), catalog.connectors.len());
        let doc = |company: &str| Narrowed {
            company: Some(company.into()),
            tag: Some(ConnectorTagId::Documentation),
            ..Narrowed::default()
        };
        assert_eq!(listed(doc("cloudflare")), ["cloudflare-docs"]);
        assert_eq!(listed(doc("nobody")), Vec::<String>::new());
        let second = Narrowed {
            category: Some(ConnectorCategoryId::Everyday),
            tag: Some(ConnectorTagId::Notes),
            ..Narrowed::default()
        };
        let notes = listed(second);
        assert!(
            notes.contains(&"notion".to_owned()),
            "a category that isn't its home counts: {notes:?}"
        );
    }

    /// A catalog of one company and one connector, whose lines the test replaces.
    const ONE: &str = "[[company]]\nid = \"x\"\nname = \"X\"\nhomepage = \"https://x.example.com\"\n\n\
        [[connector]]\nid = \"x\"\nname = \"X\"\ncompany = \"x\"\n\
        url = \"https://mcp.example.com/mcp\"\ncategories = [\"dev\"]\ntags = [\"issues\"]\n\
        returns = \"text\"\nsign_in = \"dcr\"\ndocs = \"https://x.example.com\"\n\
        description = \"X.\"\n";

    #[test]
    fn a_catalog_with_a_broken_entry_is_refused() {
        assert!(Catalog::read(ONE).is_ok());
        for (line, broken) in [
            (
                "id = \"x\"\nname = \"X\"\ncompany",
                "id = \"X Y\"\nname = \"X\"\ncompany",
            ),
            ("company = \"x\"", "company = \"other\""),
            ("https://x.example.com\"\n\n", "http://x.example.com\"\n\n"),
            ("https://mcp.example.com", "http://mcp.example.com"),
            ("[\"dev\"]", "[]"),
            ("[\"dev\"]", "[\"dev\", \"dev\"]"),
            ("[\"dev\"]", "[\"other\"]"),
            ("[\"issues\"]", "[]"),
            ("[\"issues\"]", "[\"other\"]"),
            ("\"text\"", "\"pictures\""),
            ("\"dcr\"", "\"key\""),
            ("description = \"X.\"", "description = \" \""),
            ("description = \"X.\"\n", ""),
            ("homepage", "site = \"x\"\nhomepage"),
        ] {
            let broken = ONE.replacen(line, broken, 1);
            assert_ne!(broken, ONE, "{line}");
            assert!(Catalog::read(&broken).is_err(), "{broken}");
        }
        let idle = format!(
            "{ONE}\n[[company]]\nid = \"y\"\nname = \"Y\"\nhomepage = \"https://y.example.com\"\n"
        );
        assert!(Catalog::read(&idle).is_err(), "a company that runs nothing");
    }
}
