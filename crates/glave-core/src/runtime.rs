//! Runtime selection — the one product-facing knob ([`Plan`]) mapped to a
//! concrete runtime. This is the *only* place that knows how a session's
//! workspace is built; everything downstream (the agent loop, the tools) is
//! handed a [`Workspace`] and stays location-blind.
//!
//! One knob: **where it runs.** Isolation and materialization *follow* from the
//! [`Plan`] variant — they are not separate choices, and illegal combos (e.g.
//! "cloud but no container") are unrepresentable.
//!
//! The product path is [`open`] → `Box<dyn Workspace>` (no grader: the product
//! does work, it doesn't grade). The *eval* path is parallel on purpose —
//! [`crate::ports::RuntimeFactory`] builds a `Box<dyn EvalRuntime>` (workspace +
//! grader) from a `Plan` plus an oracle dir, with the oracle reaching the grader
//! by a path the workspace itself never sees.

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::local::LocalRuntime;
use crate::ports::Workspace;

/// How a session's workspace is built and where it executes — the one knob the
/// product exposes. Isolation + materialization FOLLOW from the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Directly on a real folder on this machine — trusted, edits in place, no
    /// container, git is the undo. The local product path.
    Direct { folder: PathBuf },
    /// A disposable container: `place` is whose machine runs it, `source` how the
    /// workspace is materialized, `sandbox` the image + resource limits. Carries
    /// no oracle — the held tests reach the grader separately, never the plan.
    Sandboxed {
        place: Place,
        source: Source,
        sandbox: Sandbox,
    },
}

/// Whose machine a sandboxed plan runs on. `Cloud` is the multi-tenant seam,
/// stubbed until cloud execution lands. There is no "direct cloud" — `Cloud`
/// exists only under [`Plan::Sandboxed`], so that illegal state can't be formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Local,
    Cloud,
}

/// How a sandbox's workspace is populated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Copy an existing local directory in (eval copies the task's `workspace/`).
    CopyFrom(PathBuf),
}

/// The container spec — image + resource limits. Deliberately carries no oracle:
/// the workspace plan must never be able to reach the held tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Sandbox {
    pub image: String,
    pub workdir: String,
    pub mem_mb: u64,
    pub cpus: f64,
    pub pids: u64,
    pub network: String,
}

/// Open the product runtime for a [`Plan`] and hand it off as a [`Workspace`] —
/// the caller never learns which kind it got, and never receives a grader.
///
/// The match is exhaustive, so the compiler will point here when the sandboxed /
/// cloud arms are built.
pub fn open(plan: &Plan) -> Result<Box<dyn Workspace>> {
    match plan {
        Plan::Direct { folder } => Ok(Box::new(LocalRuntime::open(folder)?)),
        // Containerized product execution (a local OS-sandbox/Docker cell, or a
        // cloud cell) isn't built yet; the seam is here so it drops in without
        // disturbing the loop. Eval's sandbox is built behind `RuntimeFactory`.
        Plan::Sandboxed { place, .. } => bail!(
            "sandboxed execution ({place:?}) isn't built yet — run Direct on your machine for now"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_sandbox() -> Sandbox {
        Sandbox {
            image: "x".to_string(),
            workdir: "/work".to_string(),
            mem_mb: 512,
            cpus: 1.0,
            pids: 256,
            network: "none".to_string(),
        }
    }

    #[test]
    fn direct_opens_a_usable_workspace() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("glave-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let rt = open(&Plan::Direct {
            folder: dir.clone(),
        })?;
        // It's a real workspace: listing the (empty) folder works.
        assert!(rt.list_files()?.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn sandboxed_is_a_clear_stub_for_now() {
        let plan = Plan::Sandboxed {
            place: Place::Cloud,
            source: Source::CopyFrom("/tmp".into()),
            sandbox: a_sandbox(),
        };
        let r = open(&plan);
        assert!(r.is_err());
        if let Err(e) = r {
            assert!(e.to_string().contains("isn't built yet"));
        }
    }
}
