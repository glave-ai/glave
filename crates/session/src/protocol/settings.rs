//! What of a person's own a request changes, which their other clients hear of
//! (`settings/changed`) and read again, and an organization's brand, which everyone in it hears
//! of.

use serde::{Deserialize, Serialize};

use super::Request;

/// A person's own settings or connections, and their organization's brand, by what a client
/// reads to show them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    /// Their model accounts (`provider/list`), and so the models threads may choose
    /// (`model/list`).
    Providers,
    /// Their git hosts (`credential/list`).
    Credentials,
    /// Their sign-ins to connections (`connection/list`).
    Connections,
    /// Their secrets (`secret/list`).
    Secrets,
    /// Whether their agent may search the web (`search/get`).
    Search,
    /// How their clients group their threads (`grouping/get`).
    Grouping,
    /// Their filters among a router's providers (`filters/get`).
    Filters,
    /// Their accent (`appearance/get`).
    Appearance,
    /// Their organization's brand: how glave looks to them (`appearance/get`) and what their
    /// organization is called (`organization/list`).
    Brand,
    /// Whether the computer is kept awake while glave works (`awake/get`).
    Awake,
    /// Whether glave remembers for them (`memory/get`).
    Memory,
    /// Their passkeys (`passkey/list`).
    Passkeys,
    /// The language the server writes its own sentences to them in (`language/get`): what a
    /// client shows of the server's words, an approval's included, it reads again.
    Language,
}

impl Setting {
    /// Every setting, as a watcher that may have missed some is told.
    pub(crate) const ALL: [Self; 13] = [
        Self::Providers,
        Self::Credentials,
        Self::Connections,
        Self::Secrets,
        Self::Search,
        Self::Grouping,
        Self::Filters,
        Self::Appearance,
        Self::Brand,
        Self::Awake,
        Self::Memory,
        Self::Passkeys,
        Self::Language,
    ];

    /// Whether it reads the same in every organization the person is in, so a change made
    /// working in one is theirs to hear in all: their passkeys and their name belong to their
    /// account. Every other setting is kept in one organization, or is the local person's, who
    /// is in one.
    pub(crate) const fn follows_the_person(self) -> bool {
        matches!(self, Self::Passkeys)
    }

    /// Whether it is everyone's in the organization, so a change one of them makes is theirs
    /// all to hear: its brand. Every other setting is the person's who changed it.
    pub(crate) const fn follows_the_organization(self) -> bool {
        matches!(self, Self::Brand)
    }
}

impl Request {
    /// What of the caller's own, or of their organization's, this request changes once it is
    /// answered, unless with a sign-in still waiting for them; none for one that changes neither.
    pub(crate) fn changes(&self) -> Option<Setting> {
        Some(match self {
            Self::ProviderConnect { .. }
            | Self::ProviderRefresh {}
            | Self::ProviderRemove { .. } => Setting::Providers,
            Self::CredentialConnect { .. }
            | Self::CredentialRemove { .. }
            | Self::CredentialDevicePoll { .. } => Setting::Credentials,
            Self::ConnectionPoll { .. }
            | Self::ConnectionRemove { .. }
            | Self::ConnectorConnect { .. }
            | Self::ConnectorDisconnect { .. } => Setting::Connections,
            Self::SecretAdd { .. } | Self::SecretReplace { .. } | Self::SecretRemove { .. } => {
                Setting::Secrets
            }
            Self::SearchSet { .. } => Setting::Search,
            Self::GroupingSet { .. } => Setting::Grouping,
            Self::FiltersSet { .. } => Setting::Filters,
            Self::AppearanceSet { .. } => Setting::Appearance,
            Self::BrandSet { .. } => Setting::Brand,
            Self::AwakeSet { .. } => Setting::Awake,
            Self::MemorySet { .. } => Setting::Memory,
            Self::PasskeyCreate { .. }
            | Self::PasskeyAdd { .. }
            | Self::PasskeyRemove { .. }
            | Self::PersonName { .. } => Setting::Passkeys,
            Self::LanguageSet { .. } => Setting::Language,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_says_what_it_changes_and_a_read_nothing() {
        assert_eq!(
            Request::SearchSet { on: true }.changes(),
            Some(Setting::Search)
        );
        let poll = Request::ConnectionPoll {
            flow: "flow".into(),
        };
        assert_eq!(poll.changes(), Some(Setting::Connections));
        assert_eq!(Request::SearchGet { usage: false }.changes(), None);
        let filters = Request::FiltersSet {
            filters: glave_event::Filters::default(),
        };
        assert_eq!(filters.changes(), Some(Setting::Filters));
        let appearance = Request::AppearanceSet {
            accent: Some(glave_event::Accent::Plain),
        };
        assert_eq!(appearance.changes(), Some(Setting::Appearance));
        let brand = Request::BrandSet {
            name: None,
            logo: None,
            accent: glave_event::Accent::Plain,
            locked: false,
        };
        assert_eq!(brand.changes(), Some(Setting::Brand));
        assert_eq!(Request::FiltersGet {}.changes(), None);
        assert_eq!(
            Request::ProviderRefresh {}.changes(),
            Some(Setting::Providers)
        );
        assert_eq!(Request::GroupingGet {}.changes(), None);
        let chosen = Request::LanguageSet {
            language: Some("en-XA".into()),
        };
        assert_eq!(chosen.changes(), Some(Setting::Language));
        assert_eq!(Request::LanguageGet {}.changes(), None);
    }
}
