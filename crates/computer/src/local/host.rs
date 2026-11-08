//! What a computer on the host itself cannot honour, which it refuses rather than ignores.

use glave_ports::computer::{ComputerError, Environment, Image, Network};

/// Refuses what a computer on the host cannot honour, rather than quietly ignoring it.
pub(super) fn check(environment: &Environment) -> Result<(), ComputerError> {
    if let Image::Oci(image) = &environment.image {
        return Err(ComputerError::Unsupported(format!(
            "this machine runs work on the host itself and cannot run the image {image}; \
             computers made from images come with cloud computers"
        )));
    }
    if !environment.setup.is_empty() {
        return Err(ComputerError::Unsupported(
            "setup commands are cached in a snapshot of an image; on the host, run them \
             yourself"
                .into(),
        ));
    }
    if matches!(environment.network, Some(Network::Off | Network::Allowlist)) {
        return Err(ComputerError::Unsupported(
            "the host cannot limit the network per computer; use the sandbox, or a computer \
             made from an image"
                .into(),
        ));
    }
    Ok(())
}
