//! Where AWS keys come from, in the order AWS's own libraries look, for the sources that fit a
//! server: keys in the environment, IRSA's web identity, then a container credentials endpoint
//! (EKS Pod Identity, or ECS). Shared config files and the instance metadata service are not
//! read.

use std::fmt;

use glave_ports::auth::{AuthError, Fresh, Secret};
use jiff::Timestamp;

use crate::container::Container;
use crate::sts::WebIdentity;

/// One set of AWS keys.
#[derive(Clone)]
pub(crate) struct Keys {
    pub access_key_id: String,
    pub secret_access_key: Secret,
    /// Present for temporary keys, which IRSA and Pod Identity give.
    pub session_token: Option<Secret>,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keys")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .field("session_token", &self.session_token)
            .finish()
    }
}

/// Where the keys come from.
pub struct Source(Kind);

enum Kind {
    /// `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and optionally `AWS_SESSION_TOKEN`.
    Environment(Keys),
    /// IRSA: `AWS_WEB_IDENTITY_TOKEN_FILE` and `AWS_ROLE_ARN`, through STS.
    WebIdentity(WebIdentity),
    /// EKS Pod Identity or ECS: `AWS_CONTAINER_CREDENTIALS_FULL_URI` (or `_RELATIVE_URI`).
    Container(Container),
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

impl Source {
    /// The first source the environment, read through `env`, sets up; `None` when it sets up
    /// none. STS is asked in `region` unless `AWS_ENDPOINT_URL_STS` names another endpoint.
    /// An error names a variable that is missing or wrong.
    pub fn from_env(
        env: &dyn Fn(&str) -> Option<String>,
        region: &crate::Region,
    ) -> Result<Option<Self>, AuthError> {
        let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
        if let Some(access_key_id) = set("AWS_ACCESS_KEY_ID") {
            let secret = set("AWS_SECRET_ACCESS_KEY").ok_or_else(|| {
                AuthError::refused("AWS_ACCESS_KEY_ID is set but AWS_SECRET_ACCESS_KEY is not")
            })?;
            return Ok(Some(Self(Kind::Environment(Keys {
                access_key_id: access_key_id.trim().to_owned(),
                secret_access_key: Secret::new(secret.trim()),
                session_token: set("AWS_SESSION_TOKEN").map(|token| Secret::new(token.trim())),
            }))));
        }
        if let Some(identity) = WebIdentity::from_env(&set, region)? {
            return Ok(Some(Self(Kind::WebIdentity(identity))));
        }
        Ok(Container::from_env(&set)?.map(|container| Self(Kind::Container(container))))
    }

    /// Keys from this source, and when they expire.
    pub(crate) async fn fetch(&self, http: &reqwest::Client) -> Result<Fresh<Keys>, AuthError> {
        match &self.0 {
            Kind::Environment(keys) => Ok(Fresh {
                value: keys.clone(),
                expires: Timestamp::MAX,
            }),
            Kind::WebIdentity(identity) => identity.assume(http).await,
            Kind::Container(container) => container.fetch(http).await,
        }
    }

    /// Where the keys come from, never a secret.
    pub fn describe(&self) -> String {
        match &self.0 {
            Kind::Environment(keys) => {
                format!("AWS keys in the environment ({})", keys.access_key_id)
            }
            Kind::WebIdentity(identity) => identity.describe(),
            Kind::Container(container) => container.describe(),
        }
    }
}

/// When keys expire, from the ISO 8601 time AWS gives, such as `2026-10-12T18:00:00Z`.
pub(crate) fn expiration(text: &str, from: &str) -> Result<Timestamp, AuthError> {
    text.trim().parse().map_err(|_| {
        AuthError::unreachable(format!("{from} sent an expiration glave could not read"))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use glave_ports::testing::any_secret;

    use super::*;
    use crate::Region;

    fn source(vars: &[(&str, &str)]) -> Result<Option<String>, AuthError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let region = Region::new("us-east-1").map_err(|e| AuthError::refused(e.to_string()))?;
        Source::from_env(&|name| vars.get(name).cloned(), &region)
            .map(|source| source.map(|source| source.describe()))
    }

    #[test]
    fn sources_are_found_in_aws_s_order() {
        assert_eq!(source(&[]), Ok(None));
        let secret = any_secret();
        let keys = [
            ("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE"),
            ("AWS_SECRET_ACCESS_KEY", secret.as_str()),
        ];
        let irsa = [
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/irsa/token"),
            ("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/glave"),
        ];
        let pod = [(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            "http://169.254.170.23/v1/credentials",
        )];
        let all: Vec<_> = keys.iter().chain(&irsa).chain(&pod).copied().collect();
        assert_eq!(
            source(&all).unwrap().unwrap(),
            "AWS keys in the environment (AKIDEXAMPLE)"
        );
        let both: Vec<_> = irsa.iter().chain(&pod).copied().collect();
        assert!(source(&both).unwrap().unwrap().contains("role/glave"));
        assert!(source(&pod).unwrap().unwrap().contains("169.254.170.23"));
    }

    #[test]
    fn half_set_keys_are_an_error_naming_what_is_missing() {
        let error = source(&[("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE")]).unwrap_err();
        assert!(
            error.to_string().contains("AWS_SECRET_ACCESS_KEY"),
            "{error}"
        );
    }

    #[test]
    fn keys_never_show_their_secrets() {
        let (secret, token) = (any_secret(), any_secret());
        let keys = Keys {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: Secret::new(secret.clone()),
            session_token: Some(Secret::new(token.clone())),
        };
        let shown = format!("{keys:?}");
        assert!(shown.contains("AKIDEXAMPLE"), "{shown}");
        assert!(
            !shown.contains(&secret) && !shown.contains(&token),
            "{shown}"
        );
    }
}
