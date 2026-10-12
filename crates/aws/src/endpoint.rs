//! Where Bedrock's OpenAI-compatible Chat Completions are, in a region, and the name requests
//! there are signed for.

use thiserror::Error;

/// An AWS region, such as `us-east-1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region(String);

/// A name that is not a region.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0:?} is not an AWS region, such as us-east-1")]
pub struct RegionError(String);

impl Region {
    pub fn new(name: &str) -> Result<Self, RegionError> {
        let name = name.trim();
        let valid = (1..=32).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && name.contains('-');
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(RegionError(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which of Bedrock's two inference endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Endpoint {
    /// `bedrock-runtime`, which AWS recommends; models named by inference profile, such as
    /// `us.openai.gpt-oss-120b-1:0`, for those that need one.
    #[default]
    Runtime,
    /// `bedrock-mantle`, for the models only it serves (Gemma 4 among them).
    Mantle,
}

impl Endpoint {
    /// `runtime` or `mantle`.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "runtime" => Some(Self::Runtime),
            "mantle" => Some(Self::Mantle),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Mantle => "mantle",
        }
    }

    /// The OpenAI-compatible API's base in `region`, under which `/chat/completions` is.
    pub fn base_url(self, region: &Region) -> String {
        let region = region.as_str();
        match self {
            Self::Runtime => format!("https://bedrock-runtime.{region}.amazonaws.com/openai/v1"),
            Self::Mantle => format!("https://bedrock-mantle.{region}.api.aws/v1"),
        }
    }

    /// Anthropic's Messages API's base in `region`, under which `/v1/messages` is: Claude,
    /// which Bedrock does not serve through Chat Completions. The request is Anthropic's own
    /// (`anthropic-version: 2023-06-01`, the model in the body), answered as server-sent
    /// events, signed for the same service as the endpoint's other APIs.
    pub fn anthropic_base_url(self, region: &Region) -> String {
        let region = region.as_str();
        match self {
            Self::Runtime => format!("https://bedrock-runtime.{region}.amazonaws.com/anthropic"),
            Self::Mantle => format!("https://bedrock-mantle.{region}.api.aws/anthropic"),
        }
    }

    /// The service name requests here are signed for.
    pub fn signing_name(self) -> &'static str {
        match self {
            Self::Runtime => "bedrock",
            Self::Mantle => "bedrock-mantle",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_endpoint_has_its_url_and_signing_name() {
        let region = Region::new("us-east-1").unwrap();
        assert_eq!(
            Endpoint::Runtime.base_url(&region),
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"
        );
        assert_eq!(
            Endpoint::Mantle.base_url(&region),
            "https://bedrock-mantle.us-east-1.api.aws/v1"
        );
        assert_eq!(Endpoint::Runtime.signing_name(), "bedrock");
        assert_eq!(Endpoint::Mantle.signing_name(), "bedrock-mantle");
        assert_eq!(Endpoint::named("mantle"), Some(Endpoint::Mantle));
        assert_eq!(Endpoint::named("bedrock"), None);
    }

    #[test]
    fn only_region_names_are_regions() {
        assert!(Region::new("eu-central-2").is_ok());
        assert!(Region::new("us-gov-west-1").is_ok());
        for name in [
            "",
            "us_east_1",
            "US-EAST-1",
            "useast1",
            "us-east-1.evil.com",
            "a/b-1",
        ] {
            assert!(Region::new(name).is_err(), "{name}");
        }
    }
}
