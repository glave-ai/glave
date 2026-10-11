//! Where glave has no sandbox (Windows, for now): a sandboxed command is refused.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::{ComputerError, Sandbox};

use super::Launch;

pub(super) type Held = ();

pub(super) fn available() -> bool {
    false
}

pub(super) fn helper(_args: &[OsString]) -> String {
    "the sandbox helper runs only on Linux".into()
}

#[derive(Debug)]
pub(super) enum Enforcer {}

impl Enforcer {
    pub(super) fn prepare(
        _sandbox: &Sandbox,
        _root: &Path,
        _git_common: Option<&Path>,
        _helper: Option<&Path>,
        _cache: Option<&Path>,
    ) -> std::future::Ready<Result<(Self, PathBuf), ComputerError>> {
        std::future::ready(Err(ComputerError::Unsupported(
            "the sandbox is not available on this computer (macOS and Linux only for now)".into(),
        )))
    }

    pub(super) fn launch(&self) -> std::future::Ready<Result<Launch, ComputerError>> {
        match *self {}
    }
}
