//! Where a project's Vertex AI models are: a location (global, a multi-region such as `us`, or
//! a region such as `us-east5`), its host, and the APIs under it.

use thiserror::Error;

/// A project or location that cannot be part of a Vertex URL.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EndpointError {
    #[error("{0:?} is not a Google Cloud project id (lower-case letters, digits and hyphens)")]
    Project(String),
    #[error(
        "{0:?} is not a Vertex AI location: global, a multi-region (us, eu) or a region such as \
         us-east5"
    )]
    Location(String),
}

/// Where requests are served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// `global`: routed to any region with capacity, at no premium. Google's recommended.
    Global,
    /// A multi-region, `us` or `eu`: routed within that geography.
    Multi(String),
    /// One region, such as `us-east5` or `europe-west1`.
    Region(String),
}

impl Location {
    pub fn new(name: &str) -> Result<Self, EndpointError> {
        let name = name.trim();
        match name {
            "global" => return Ok(Self::Global),
            "us" | "eu" => return Ok(Self::Multi(name.to_owned())),
            _ => {}
        }
        let mut parts = name.split('-');
        let regional = matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(area), Some(place), None)
                if !area.is_empty()
                    && area.bytes().all(|b| b.is_ascii_lowercase())
                    && place.bytes().any(|b| b.is_ascii_digit())
                    && place.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    && place.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        );
        if regional {
            Ok(Self::Region(name.to_owned()))
        } else {
            Err(EndpointError::Location(name.to_owned()))
        }
    }

    /// The location's name, as it goes in a URL's path.
    pub fn name(&self) -> &str {
        match self {
            Self::Global => "global",
            Self::Multi(name) | Self::Region(name) => name,
        }
    }

    /// The host that serves it: `aiplatform.googleapis.com` for global,
    /// `aiplatform.us.rep.googleapis.com` for a multi-region, `us-east5-aiplatform.googleapis.com`
    /// for a region.
    pub fn host(&self) -> String {
        match self {
            Self::Global => "aiplatform.googleapis.com".to_owned(),
            Self::Multi(name) => format!("aiplatform.{name}.rep.googleapis.com"),
            Self::Region(name) => format!("{name}-aiplatform.googleapis.com"),
        }
    }
}

/// A project's models at one location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vertex {
    project: String,
    location: Location,
}

impl Vertex {
    /// `project` (its id, such as `acme-ai-prod`) at `location`.
    pub fn new(project: &str, location: &str) -> Result<Self, EndpointError> {
        let project = project.trim();
        let valid = (1..=63).contains(&project.len())
            && project.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b':')
            });
        if !valid {
            return Err(EndpointError::Project(project.to_owned()));
        }
        Ok(Self {
            project: project.to_owned(),
            location: Location::new(location)?,
        })
    }

    /// `https://<host>/v1/projects/<project>/locations/<location>`, under which the APIs are.
    pub fn base(&self) -> String {
        format!(
            "https://{}/v1/projects/{}/locations/{}",
            self.location.host(),
            self.project,
            self.location.name()
        )
    }

    /// The OpenAI-compatible API's base, under which `/chat/completions` is; Gemini is named
    /// there as `google/<model>`, such as `google/gemini-3.5-flash`.
    pub fn openai_base_url(&self) -> String {
        format!("{}/endpoints/openapi", self.base())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_location_has_its_host() {
        let base = |location: &str| Vertex::new("acme-ai", location).unwrap().base();
        assert_eq!(
            base("global"),
            "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global"
        );
        assert_eq!(
            base("eu"),
            "https://aiplatform.eu.rep.googleapis.com/v1/projects/acme-ai/locations/eu"
        );
        assert_eq!(
            base("us-east5"),
            "https://us-east5-aiplatform.googleapis.com/v1/projects/acme-ai/locations/us-east5"
        );
        assert_eq!(
            Vertex::new("acme-ai", "global").unwrap().openai_base_url(),
            "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global/endpoints/openapi"
        );
    }

    #[test]
    fn only_project_ids_and_locations_are_accepted() {
        assert!(Vertex::new("example.com:acme-ai", "europe-west1").is_ok());
        for project in ["", "Acme", "acme/ai", "acme ai", "acme?x=1"] {
            assert!(
                matches!(
                    Vertex::new(project, "global"),
                    Err(EndpointError::Project(_))
                ),
                "{project}"
            );
        }
        for location in [
            "",
            "Global",
            "us-east-5-x",
            "useast5",
            "us-",
            "-east5",
            "us-east5.evil.com",
            "us-east5/x",
            "asia",
        ] {
            assert!(
                matches!(
                    Vertex::new("acme-ai", location),
                    Err(EndpointError::Location(_))
                ),
                "{location}"
            );
        }
    }
}
