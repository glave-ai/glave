//! What a worker may be: the limits an organization's policy sets, and the check of an
//! environment against them before anything starts. What glave cannot enforce yet is refused
//! by name rather than quietly ignored.

use std::ffi::OsStr;

use glave_ports::secrets::is_secret_variable;
use glave_ports::{ComputerError, Environment, Image, Network};

/// Policy's limits on workers (`[workers]` in `policy.toml`). Every one is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// The most CPUs a worker may have; also its size when its environment names none.
    pub cpus: Option<u32>,
    /// The most memory, in GB; also its size when its environment names none.
    pub memory_gb: Option<u32>,
    /// How long a worker may run, in minutes, before it refuses new work.
    pub max_minutes: Option<u32>,
    /// How many workers one person may have at once.
    pub per_person: Option<u32>,
    /// The most network a worker may have; `None` allows it all. An allow-list is not
    /// enforced yet, so a policy asking for one is refused before workers start.
    pub network: Option<Network>,
    /// How far apart from its host a worker must be kept.
    pub isolation: Isolation,
}

/// How a worker is kept apart from the machine it runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Isolation {
    /// A container: its own namespaces and limits on a shared kernel.
    #[default]
    Container,
    /// A sandbox with a kernel of its own (gVisor) or a virtual machine (Kata), from a
    /// Kubernetes runtime class; untrusted code needs it.
    Vm,
}

/// A worker's settings once its environment passed the limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decided {
    pub(crate) image: String,
    pub(crate) cpus: Option<u32>,
    pub(crate) memory_gb: Option<u32>,
    pub(crate) network: bool,
    pub(crate) max_minutes: Option<u32>,
}

fn unsupported(message: &str) -> ComputerError {
    ComputerError::Unsupported(message.to_owned())
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

/// Checks `environment` against `limits` with `running` workers already held, and says what
/// the worker will be.
pub(crate) fn decide(
    environment: &Environment,
    limits: &Limits,
    running: usize,
) -> Result<Decided, ComputerError> {
    let Image::Oci(image) = &environment.image else {
        return Err(ComputerError::Refused(
            "a worker runs an image; this environment names the host. Name an image in \
             devcontainer.json or .glave/environment.toml"
                .into(),
        ));
    };
    if !pinned(image) {
        return Err(ComputerError::Refused(format!(
            "workers run images pinned by digest, so the same bytes run every time: write the \
             image as name@sha256:<digest>, not {image}"
        )));
    }
    if let Some(most) = limits.per_person
        && running >= usize::try_from(most).unwrap_or(usize::MAX)
    {
        return Err(ComputerError::Refused(format!(
            "you already have {running} workers, and policy allows {most} at once \
             ([workers] per_person); release one first"
        )));
    }
    let cpus = within("CPUs", "cpu", environment.size.cpus, limits.cpus)?;
    let memory_gb = within(
        "GB of memory",
        "memory_gb",
        environment.size.memory_gb,
        limits.memory_gb,
    )?;
    if environment.size.disk_gb.is_some() {
        return Err(unsupported(
            "disk size is not enforced on Docker workers yet; remove it from the environment",
        ));
    }
    let network = network(environment, limits)?;
    if !environment.secrets.is_empty() {
        return Err(unsupported(
            "secrets for workers come with the credential helper that asks for each use; \
             remove them from the environment",
        ));
    }
    if environment.idle_minutes.is_some() {
        return Err(unsupported(
            "workers do not sleep yet: they are released when their thread's session ends; \
             remove idle_minutes from the environment",
        ));
    }
    if let Some(name) = environment
        .env
        .keys()
        .find(|name| is_secret_variable(OsStr::new(name.as_str())))
    {
        return Err(ComputerError::Refused(format!(
            "{name} is named like a secret; a worker's variables are never secrets"
        )));
    }
    Ok(Decided {
        image: image.clone(),
        cpus,
        memory_gb,
        network,
        max_minutes: limits.max_minutes,
    })
}

/// A size asked for, within the limit; the limit itself when nothing is asked.
fn within(
    what: &str,
    key: &str,
    asked: Option<u32>,
    most: Option<u32>,
) -> Result<Option<u32>, ComputerError> {
    match (asked, most) {
        (Some(asked), Some(most)) if asked > most => Err(ComputerError::Refused(format!(
            "the environment asks for {asked} {what}, and policy allows at most {most} \
             ([workers] {key})"
        ))),
        (Some(asked), _) => Ok(Some(asked)),
        (None, most) => Ok(most),
    }
}

/// Whether the worker gets a network: what the environment asks, within policy.
fn network(environment: &Environment, limits: &Limits) -> Result<bool, ComputerError> {
    let allowed = !matches!(limits.network, Some(Network::Off));
    if matches!(limits.network, Some(Network::Allowlist))
        || matches!(environment.network, Some(Network::Allowlist))
        || !environment.allow.is_empty()
    {
        return Err(unsupported(
            "an allow-list of hosts is not enforced on workers yet: choose network on or off",
        ));
    }
    match environment.network {
        Some(Network::On) if !allowed => Err(ComputerError::Refused(
            "the environment asks for the network, and policy allows workers none \
             ([workers] network = \"off\")"
                .into(),
        )),
        Some(Network::Off) => Ok(false),
        _ => Ok(allowed),
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
            0,
        );
        assert!(matches!(refused, Err(ComputerError::Refused(m)) if m.contains("digest")));
        assert!(decide(&Environment::host(), &Limits::default(), 0).is_err());
    }

    #[test]
    fn sizes_and_counts_stay_within_policy() {
        let limits = Limits {
            cpus: Some(2),
            memory_gb: Some(4),
            per_person: Some(1),
            ..Limits::default()
        };
        let decided = decide(&environment(), &limits, 0).unwrap();
        assert_eq!((decided.cpus, decided.memory_gb), (Some(2), Some(4)));
        assert!(
            decided.network,
            "on unless policy or the environment says off"
        );
        let mut big = environment();
        big.size.cpus = Some(8);
        let refused = decide(&big, &limits, 0).unwrap_err();
        assert!(refused.message().contains("[workers] cpu"), "{refused}");
        let refused = decide(&environment(), &limits, 1).unwrap_err();
        assert!(refused.message().contains("per_person"), "{refused}");
    }

    #[test]
    fn the_network_is_on_or_off_and_never_more_than_policy_allows() {
        let off = Limits {
            network: Some(Network::Off),
            ..Limits::default()
        };
        assert!(!decide(&environment(), &off, 0).unwrap().network);
        let mut wants = environment();
        wants.network = Some(Network::On);
        assert!(matches!(
            decide(&wants, &off, 0),
            Err(ComputerError::Refused(_))
        ));
        wants.network = Some(Network::Allowlist);
        assert!(matches!(
            decide(&wants, &Limits::default(), 0),
            Err(ComputerError::Unsupported(_))
        ));
        wants.network = Some(Network::Off);
        assert!(!decide(&wants, &Limits::default(), 0).unwrap().network);
    }

    #[test]
    fn what_is_not_enforced_yet_is_refused_by_name() {
        let mut secrets = environment();
        secrets.secrets = vec!["GITHUB_TOKEN".into()];
        assert!(matches!(
            decide(&secrets, &Limits::default(), 0),
            Err(ComputerError::Unsupported(_))
        ));
        let mut disk = environment();
        disk.size.disk_gb = Some(50);
        assert!(decide(&disk, &Limits::default(), 0).is_err());
        let mut idle = environment();
        idle.idle_minutes = Some(30);
        assert!(decide(&idle, &Limits::default(), 0).is_err());
        let mut leaky = environment();
        leaky.env.insert("API_TOKEN".into(), "x".into());
        assert!(matches!(
            decide(&leaky, &Limits::default(), 0),
            Err(ComputerError::Refused(_))
        ));
    }
}
