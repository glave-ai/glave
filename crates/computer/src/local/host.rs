//! What a computer on the host itself cannot honour, which it refuses rather than ignores.

use glave_ports::computer::{ComputerError, Environment, Image};

/// Refuses what a computer on the host cannot honour, rather than quietly ignoring it.
pub(super) fn check(environment: &Environment) -> Result<(), ComputerError> {
    if let Image::Oci(image) = &environment.image {
        return Err(ComputerError::Unsupported(format!(
            "this machine runs work on the host itself and cannot run the image {image}; \
             computers made from images come with cloud computers"
        )));
    }
    Ok(())
}
