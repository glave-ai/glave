//! Principals: whoever acts. A person, an agent profile, a group, or glave itself deciding on
//! its own. See "Who acted" in `specs/events.md`.

use serde::{Deserialize, Serialize};

use crate::id::PrincipalId;

/// An enum written as a string, whose list of values is open: a value this reader does not
/// know is kept as written, never refused, so newer writers never break older readers.
macro_rules! open_kind {
    ($(#[$meta:meta])* pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
            /// A value this reader does not know, kept as written.
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $text,)*
                    Self::Other(text) => text,
                }
            }
        }

        impl From<String> for $name {
            fn from(text: String) -> Self {
                match text.as_str() {
                    $($text => Self::$variant,)*
                    _ => Self::Other(text),
                }
            }
        }

        impl From<$name> for String {
            fn from(kind: $name) -> Self {
                kind.as_str().to_owned()
            }
        }
    };
}

pub(crate) use open_kind;

open_kind! {
    /// What kind of principal acts.
    pub enum PrincipalKind {
        /// A human.
        Person => "person",
        /// An agent profile: the default agent, a subagent type, a named assistant.
        Agent => "agent",
        /// A team acting as one.
        Group => "group",
        /// glave itself, deciding on its own: a timeout, a cancellation, a rule.
        Glave => "glave",
    }
}

/// A reference to a principal: `{"kind": "person", "id": "…"}`. Every kind but `glave` has an
/// id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub kind: PrincipalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<PrincipalId>,
}

impl Principal {
    pub fn person(id: PrincipalId) -> Self {
        Self {
            kind: PrincipalKind::Person,
            id: Some(id),
        }
    }

    pub fn agent(id: PrincipalId) -> Self {
        Self {
            kind: PrincipalKind::Agent,
            id: Some(id),
        }
    }

    pub fn group(id: PrincipalId) -> Self {
        Self {
            kind: PrincipalKind::Group,
            id: Some(id),
        }
    }

    /// glave itself, deciding on its own.
    pub fn glave() -> Self {
        Self {
            kind: PrincipalKind::Glave,
            id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_principal_is_written_as_kind_and_id_and_glave_has_no_id() {
        let glave = serde_json::to_string(&Principal::glave()).unwrap();
        assert_eq!(glave, r#"{"kind":"glave"}"#);
        let text = r#"{"kind":"person","id":"0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10"}"#;
        let person: Principal = serde_json::from_str(text).unwrap();
        assert_eq!(person.kind, PrincipalKind::Person);
        assert_eq!(serde_json::to_string(&person).unwrap(), text);
    }

    #[test]
    fn an_unknown_kind_is_kept_as_written() {
        let text = r#"{"kind":"robot","id":"0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10"}"#;
        let principal: Principal = serde_json::from_str(text).unwrap();
        assert_eq!(principal.kind, PrincipalKind::Other("robot".into()));
        assert_eq!(serde_json::to_string(&principal).unwrap(), text);
    }
}
