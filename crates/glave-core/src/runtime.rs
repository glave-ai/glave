//! Runtime selection — the one product-facing knob ([`Location`]) mapped to a
//! concrete [`Workspace`]. This is the *only* place that knows local from
//! cloud; everything downstream (the agent loop, the tools) is handed the result
//! and stays location-blind.
//!
//! It's also the deliberate seam for cloud execution: adding it means **filling
//! the `Cloud` arm here and nothing else** — the port is already cloud-shaped
//! (relative paths, owned results), so no caller changes. The match is
//! exhaustive, so the compiler will point at this spot when cloud lands.
//!
//! (Distinct from [`crate::ports::RuntimeFactory`], which is the *eval* path —
//! it builds a sandbox from a `TaskSpec`. This is the *product* path: a runtime
//! from a workspace + a location. Parallel on purpose, not unified.)

use std::path::Path;

use anyhow::{bail, Result};

use crate::local::LocalRuntime;
use crate::ports::Workspace;

/// Where the agent's actions execute — the single choice the product exposes.
/// Whether execution is sandboxed, and how the workspace is materialized, both
/// *follow* from this; they are not separate knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// Directly on the user's machine, against a real folder — trusted, edits in
    /// place, git is the undo. No container.
    LocalMachine,
    /// A disposable, isolated sandbox in our cloud — untrusted, multi-tenant.
    /// Not built yet; the seam exists so it drops in without disturbing the loop.
    Cloud,
}

/// Open the runtime for a session, then hand it off — the caller never learns
/// which kind it got. `workspace` is the folder to operate on (local today;
/// cloud will carry its own materialization when that arm is filled).
pub fn open_runtime(location: Location, workspace: &Path) -> Result<Box<dyn Workspace>> {
    match location {
        Location::LocalMachine => Ok(Box::new(LocalRuntime::open(workspace)?)),
        // The seam is here on purpose: only this arm needs building. The port is
        // already cloud-shaped, so filling it won't ripple into any caller.
        Location::Cloud => bail!(
            "cloud execution isn't built yet — run on your machine for now"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_machine_opens_a_usable_runtime() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("glave-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let rt = open_runtime(Location::LocalMachine, &dir)?;
        // It's a real runtime: listing the (empty) workspace works.
        assert!(rt.list_files()?.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn cloud_is_a_clear_stub_for_now() {
        let r = open_runtime(Location::Cloud, Path::new("/tmp"));
        assert!(r.is_err());
        if let Err(e) = r {
            assert!(e.to_string().contains("cloud execution isn't built"));
        }
    }
}
