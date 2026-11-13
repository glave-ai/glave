//! What a worker may be: the limits an organization's policy sets, and the check of an
//! environment against them before anything starts.

use std::ffi::OsStr;

use glave_ports::computer::{ComputerError, Environment, Image, Limits, NetworkMode, Refusal};
use glave_ports::secrets::is_secret_variable;

/// A worker's settings once its environment passed the limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decided {
    pub(crate) image: String,
    pub(crate) cpus: Option<u32>,
    pub(crate) memory_gb: Option<u32>,
    pub(crate) network: bool,
}

/// Whether `reference` names its image by digest (`name@sha256:<64 hex digits>`), so the same
/// bytes run every time.
pub(crate) fn pinned(reference: &str) -> bool {
    reference
        .rsplit_once("@sha256:")
        .is_some_and(|(name, hex)| {
            !name.is_empty()
                && hex.len() == 64
                && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        })
}

/// Checks `environment` against `limits`, and says what the worker will be.
pub(crate) fn decide(environment: &Environment, limits: &Limits) -> Result<Decided, ComputerError> {
    let Image::Oci(image) = &environment.image else {
        return Err(ComputerError::Refused(Refusal::HostNotImage.into()));
    };
    if !pinned(image) {
        return Err(ComputerError::Refused(
            Refusal::NotPinned {
                image: image.clone(),
            }
            .into(),
        ));
    }
    let cpus = within(
        |asked, most| Refusal::TooManyCpus { asked, most },
        environment.size.cpus,
        limits.cpus,
    )?;
    let memory_gb = within(
        |asked, most| Refusal::TooMuchMemory { asked, most },
        environment.size.memory_gb,
        limits.memory_gb,
    )?;
    let network = limits.network.is_none_or(|most| most == NetworkMode::Open);
    if let Some(name) = environment
        .env
        .keys()
        .find(|name| is_secret_variable(OsStr::new(name.as_str())))
    {
        return Err(ComputerError::Refused(
            Refusal::SecretNamed { name: name.clone() }.into(),
        ));
    }
    Ok(Decided {
        image: image.clone(),
        cpus,
        memory_gb,
        network,
    })
}

/// A size asked for, within the limit; the limit itself when nothing is asked.
fn within(
    refused: fn(String, String) -> Refusal,
    asked: Option<u32>,
    most: Option<u32>,
) -> Result<Option<u32>, ComputerError> {
    match (asked, most) {
        (Some(asked), Some(most)) if asked > most => Err(ComputerError::Refused(
            refused(asked.to_string(), most.to_string()).into(),
        )),
        (Some(asked), _) => Ok(Some(asked)),
        (None, most) => Ok(most),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str =
        "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

    fn environment() -> Environment {
        Environment {
            image: Image::Oci(IMAGE.into()),
            ..Environment::host()
        }
    }

    #[test]
    fn only_images_pinned_by_digest_run() {
        assert!(pinned(IMAGE));
        assert!(!pinned("alpine:3.21"));
        assert!(!pinned(
            "@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507"
        ));
        let refused = decide(
            &Environment {
                image: Image::Oci("alpine:3.21".into()),
                ..Environment::host()
            },
            &Limits::default(),
        );
        assert!(matches!(refused, Err(ComputerError::Refused(m)) if m.contains("digest")));
        assert!(decide(&Environment::host(), &Limits::default()).is_err());
    }

    #[test]
    fn sizes_stay_within_policy() {
        let limits = Limits {
            cpus: Some(2),
            memory_gb: Some(4),
            ..Limits::default()
        };
        let decided = decide(&environment(), &limits).unwrap();
        assert_eq!((decided.cpus, decided.memory_gb), (Some(2), Some(4)));
        assert!(decided.network, "on unless policy says offline");
        let mut big = environment();
        big.size.cpus = Some(8);
        let refused = decide(&big, &limits).unwrap_err();
        assert!(refused.message().contains("[workers] cpu"), "{refused}");
    }

    #[test]
    fn the_network_is_policy_s() {
        let network = |mode| Limits {
            network: Some(mode),
            ..Limits::default()
        };
        let offline = decide(&environment(), &network(NetworkMode::Offline));
        assert!(!offline.unwrap().network);
        assert!(
            decide(&environment(), &network(NetworkMode::Open))
                .unwrap()
                .network
        );
    }

    #[test]
    fn a_variable_named_like_a_secret_is_refused() {
        let mut leaky = environment();
        leaky.env.insert("API_TOKEN".into(), "x".into());
        assert!(matches!(
            decide(&leaky, &Limits::default()),
            Err(ComputerError::Refused(_))
        ));
    }
}
