//! A person's computer measured under the runtime class CI gives it (`GLAVE_TEST_MEASURE=1` on
//! `kind`, once under runc and once under gVisor, `specs/computers.md`, decision 12): how long
//! it takes to wake, whether glave's sandbox (bubblewrap) starts inside it, and how long work
//! takes there. Each number is printed with the runtime class; the work must succeed, and the
//! probes of bubblewrap (bare `bwrap`, not glave's whole sandbox) are findings, never asserted.

use std::time::{Duration, Instant};

use glave_ports::block_on;
use glave_ports::computer::{Computers as _, Environment, Image, Limits, Network};

use super::super::tests::{cluster, left_behind};
use super::PersonComputer;
use super::tests::ran;
use crate::worker::backend::Backend as _;
use crate::{WorkerComputers, WorkerCredentials};

/// Whether bubblewrap starts as glave's sandbox starts it for a command without the network
/// (`local/sandbox/linux/bubblewrap.rs`'s probe), and what it says when it does not.
const BUBBLEWRAP_OFFLINE: &str = "bwrap --unshare-user --unshare-pid --unshare-ipc --unshare-net \
    --ro-bind / / --dev /dev --proc /proc -- true 2>&1 && echo starts || echo 'does not start'";

/// The same for a command with the network, which shares the computer's.
const BUBBLEWRAP_ONLINE: &str = "bwrap --unshare-user --unshare-pid --unshare-ipc \
    --ro-bind / / --dev /dev --proc /proc -- true 2>&1 && echo starts || echo 'does not start'";

/// Python's library compiled from a copy in the home: many small files written, and CPU.
const COMPILE: &str = "rm -rf py && cp -r /usr/lib/python3.13 py && \
    python3 -m compileall -q -f -j 2 py >/dev/null && echo compiled";

/// The stable Rust toolchain fetched into the home: the network and many files written.
const TOOLCHAIN: &str = "rustup set profile minimal >/dev/null && \
    rustup default stable >/dev/null 2>&1 && echo fetched";

/// A small Rust program built.
const BUILD: &str = "rm -rf hello && cargo new -q --vcs none hello && cd hello && \
    cargo build -q 2>&1 && echo built";

/// `script`'s answer on `computers`, `computer`, and how long it took.
async fn timed(
    computers: &WorkerComputers,
    computer: glave_ports::computer::ComputerId,
    script: &str,
) -> (Duration, Result<String, String>) {
    let started = Instant::now();
    let said = ran(computers, computer, script).await;
    (started.elapsed(), said)
}

#[test]
fn a_persons_computer_measured_under_its_runtime() {
    let (Ok(_), Ok(image), Ok("1")) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
        std::env::var("GLAVE_TEST_MEASURE").as_deref(),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_MEASURE=1 with GLAVE_TEST_WORKER_GLAVE and \
             GLAVE_TEST_COMPUTER_IMAGE to measure a person's computer"
        );
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let runtime = runtime_class
            .clone()
            .unwrap_or_else(|| "the default".into());
        let Some(cluster) = cluster(runtime_class.as_deref(), "measured").await else {
            return;
        };
        let id = crate::local::new_id().unwrap().simple().to_string();
        let key = format!("measured-{}", id.get(id.len() - 12..).unwrap());
        let person = PersonComputer::new(cluster.clone(), &key, 2).unwrap();
        // Two CPUs and 2 GiB, the same under both runtimes, so the work compares.
        let limits = Limits {
            isolation: cluster.isolation(),
            cpus: Some(2),
            memory_gb: Some(2),
            ..Limits::default()
        };
        let computers = WorkerComputers::on(Box::new(person), limits, WorkerCredentials::None);
        let environment = Environment {
            image: Image::Oci(image),
            network: Some(Network::On),
            ..Environment::host()
        };
        let started = Instant::now();
        let woke = computers.create(&environment).await;
        let wake_took = started.elapsed();
        let mut said = Vec::new();
        let mut failed = Vec::new();
        if let Ok(woke) = &woke {
            for (what, script) in [
                ("bubblewrap without the network", BUBBLEWRAP_OFFLINE),
                ("bubblewrap with the network", BUBBLEWRAP_ONLINE),
                ("compile Python's library", COMPILE),
                ("fetch the Rust toolchain", TOOLCHAIN),
                ("build a small Rust program", BUILD),
            ] {
                let (took, answer) = timed(&computers, woke.id, script).await;
                if answer.is_err() && !what.starts_with("bubblewrap") {
                    failed.push(what);
                }
                said.push(format!(
                    "{what}: {} ms, {}",
                    took.as_millis(),
                    answer.unwrap_or_else(|why| why).trim()
                ));
            }
            let _ = computers.destroy(woke.id).await;
        }
        let destroyed = PersonComputer::new(cluster.clone(), &key, 2)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
        eprintln!(
            "measured under {runtime}: woke in {} ms\n{}",
            wake_took.as_millis(),
            said.join("\n")
        );
        woke.unwrap();
        assert_eq!(failed, Vec::<&str>::new(), "the work ran");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}
