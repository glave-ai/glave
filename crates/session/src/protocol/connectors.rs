//! glave's catalog of connectors as clients see it (`connector/*`, `specs/session.md`,
//! "Connectors"): each connector with its company, categories and tags, what it returns and how
//! it is signed in to. Categories and tags are fixed here, each with its Fluent message, so
//! none can exist without its words.

use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

use super::Told;

/// An enum of ids, each with its id on the wire and the English of its Fluent message, whose
/// id is `<prefix>-<id>`, in the order a client shows them.
macro_rules! labelled {
    (
        $(#[$meta:meta])*
        $name:ident, $prefix:literal {
            $($variant:ident = $id:literal $english:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
        )]
        pub enum $name {
            $(
                #[doc = $english]
                #[serde(rename = $id)]
                $variant,
            )*
        }

        impl $name {
            /// Each of them, in the order a client shows them.
            pub(crate) const ALL: &[Self] = &[$(Self::$variant,)*];

            /// Its name, as the caller reads it.
            pub(crate) fn told(self) -> Told {
                let (id, english) = match self {
                    $(Self::$variant => ($id, $english),)*
                };
                Told::catalogued(format!("{}-{id}", $prefix), english.to_owned())
            }
        }
    };
}

labelled! {
    /// What kind of thing a connector is for. A connector has one or more.
    ConnectorCategoryId, "connector-category" {
        Dev = "dev" "Developer tools",
        Cloud = "cloud" "Cloud and infrastructure",
        Data = "data" "Data and analytics",
        Productivity = "productivity" "Productivity",
        Business = "business" "Business",
        Design = "design" "Design and media",
        Everyday = "everyday" "Everyday",
    }
}

labelled! {
    /// What a connector works with, finer than its categories. A connector has one or more.
    ConnectorTagId, "connector-tag" {
        AiMedia = "ai-media" "AI images and video",
        AiModels = "ai-models" "AI models",
        Analytics = "analytics" "Analytics",
        ApiTesting = "api-testing" "API testing",
        Automation = "automation" "Automation",
        CiCd = "ci-cd" "Builds and deployments",
        CodeHosting = "code-hosting" "Code hosting",
        CompanyFormation = "company-formation" "Company formation",
        Compliance = "compliance" "Compliance",
        ContentManagement = "content-management" "Content management",
        Crm = "crm" "Sales and CRM",
        CustomerSupport = "customer-support" "Customer support",
        Databases = "databases" "Databases",
        Design = "design" "Design",
        Documentation = "documentation" "Documentation",
        Domains = "domains" "Domains",
        Email = "email" "Email",
        FeatureFlags = "feature-flags" "Feature flags",
        Files = "files" "Files",
        Finance = "finance" "Finance",
        Forms = "forms" "Forms",
        Hosting = "hosting" "Hosting",
        Identity = "identity" "Users and sign-in",
        Issues = "issues" "Issues and bugs",
        Knowledge = "knowledge" "Team knowledge",
        Marketing = "marketing" "Marketing",
        MediaLibrary = "media-library" "Media libraries",
        Meetings = "meetings" "Meetings",
        Monitoring = "monitoring" "Monitoring",
        Notes = "notes" "Notes",
        Payments = "payments" "Payments",
        Presentations = "presentations" "Presentations",
        ProjectManagement = "project-management" "Project management",
        Scheduling = "scheduling" "Scheduling",
        Security = "security" "Security",
        SiteSearch = "site-search" "Search for your site",
        ToDoLists = "to-do-lists" "To-do lists",
        WebScraping = "web-scraping" "Reading websites",
        WebSearch = "web-search" "Web search",
        Websites = "websites" "Websites",
        Whiteboards = "whiteboards" "Whiteboards and diagrams",
    }
}

/// A connector of glave's catalog the caller may connect (`specs/session.md`, "Connectors"):
/// an official remote MCP server, hosted by its vendor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectorInfo {
    /// Stable: what `connector/connect` and `[mcp] catalog` name it by, and its tools are named
    /// after (`mcp__linear__…`).
    pub id: String,
    /// The vendor's name for it, never translated: `Linear`.
    pub name: String,
    /// Its company's id, one of the list's `companies`.
    pub company: String,
    /// Its server's address.
    pub url: String,
    /// Its categories, each one of the list's `categories`. The first is its home, where a
    /// client that shows it once puts it.
    pub categories: Vec<ConnectorCategoryId>,
    /// Its tags, each one of the list's `tags`.
    pub tags: Vec<ConnectorTagId>,
    pub returns: ConnectorReturns,
    pub sign_in: ConnectorSignIn,
    /// What it reaches, in the caller's language.
    pub description: Told,
    /// The vendor's own page about it.
    pub docs: String,
    pub status: ConnectorStatus,
    /// Connected, the one project whose threads alone use it, the person's Chats among them:
    /// none where every thread of theirs does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<ProjectId>,
    /// Connected, where it is used, in the caller's language: `Everywhere, including chats`,
    /// `Only in Acme Site`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<Told>,
}

/// What a connector's tools hand back, as its vendor's docs say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorReturns {
    /// Text and data only: the docs list its tools, and none hands back a file.
    Text,
    /// A tool hands back a file or media too.
    Files,
    /// Its vendor's docs don't say.
    Unknown,
}

/// How a connector is signed in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSignIn {
    /// It asks no one to sign in.
    None,
    /// The person signs in, glave registering there as a client (dynamic client registration).
    Dcr,
}

/// Whether a connector is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    /// Not connected.
    Available,
    /// Connected: the agent may use it.
    Connected,
    /// Connected, but its sign-in no longer works or its server asked for more: the person
    /// connects it again, or disconnects it.
    SignInAgain,
}

/// A company that runs connectors: one vendor can run several.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectorCompany {
    /// Stable: what `connector/list` narrows by.
    pub id: String,
    /// Its name, never translated: `Cloudflare`.
    pub name: String,
    /// Its website.
    pub homepage: String,
}

/// A category of connectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectorCategory {
    pub id: ConnectorCategoryId,
    /// Its name, in the caller's language.
    pub name: Told,
}

/// A tag of connectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectorTag {
    pub id: ConnectorTagId,
    /// Its name, in the caller's language.
    pub name: Told,
}
