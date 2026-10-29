//! A person's computer on a real cluster, with `GLAVE_TEST_KUBE=1` and a real glave and image
//! (`GLAVE_TEST_WORKER_GLAVE`, `GLAVE_TEST_WORKER_IMAGE`), as CI's `kind` job runs it: a file
//! written before it sleeps is there after it wakes, the wake is timed, and destroying it
//! leaves nothing behind.

use std::time::Instant;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerId, Computers, Ending, Environment, Image, Limits, Network, Sources,
};

use super::super::tests::{cluster, left_behind};
use crate::worker::backend::Backend as _;
use super::PersonComputer;
use crate::worker::route::Tail;
use crate::{CancelSwitch, WorkerComputers, WorkerCredentials};

/// Runs `command` at the person's home on the computer `computers` woke, and what it printed.
async fn ran(computers: &WorkerComputers, computer: ComputerId, command: &str) -> String {
    let home = Sources::Folder {
        path: super::super::objects::PERSON_HOME.into(),
    };
    let opened = computers.open(computer, &home).await.unwrap();
    let output = Tail::default();
    let finished = computers
        .run(
            opened.id,
            &Command::new(command),
            &output,
            &CancelSwitch::default(),
        )
        .await
        .unwrap();
    assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{command}");
    output.text(1_000)
}

#[test]
fn a_persons_computer_keeps_its_disk_while_it_sleeps() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_WORKER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_WORKER_IMAGE \
             (an image with bash and git, pinned by digest) to test a person's computer"
        );
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "home").await else {
            return;
        };
        let id = crate::local::new_id().unwrap().simple().to_string();
        let key = format!("test-{}", id.get(id.len() - 12..).unwrap());
        let person = PersonComputer::new(cluster.clone(), &key, 1).unwrap();
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let computers = WorkerComputers::on(Box::new(person), limits, WorkerCredentials::None);
        let environment = Environment {
            image: Image::Oci(image),
            network: Some(Network::On),
            ..Environment::host()
        };

        let first = computers.create(&environment).await.unwrap();
        let wrote = ran(&computers, first.id, "echo kept > kept.txt && echo $HOME").await;
        // Asleep: its Pod is gone, its disk kept.
        computers.destroy(first.id).await.unwrap();
        let waking = Instant::now();
        let again = computers.create(&environment).await;
        let woke = waking.elapsed();
        let read = match &again {
            Ok(again) => Ok(ran(&computers, again.id, "cat kept.txt").await),
            Err(error) => Err(error.clone()),
        };
        if let Ok(again) = &again {
            computers.destroy(again.id).await.unwrap();
        }
        let destroyed = PersonComputer::new(cluster.clone(), &key, 1)
            .unwrap()
            .destroy()
            .await;
        eprintln!("a person's computer woke in {} ms", woke.as_millis());
        assert_eq!(wrote.trim(), "/home/glave");
        assert_eq!(read.unwrap().trim(), "kept");
        destroyed.unwrap();
        assert_eq!(left_behind(&cluster).await, Vec::<String>::new());
    });
}

#[test]
fn a_persons_computer_is_named_by_a_plain_key() {
    for key in ["", "Alice", "a_b", "-a", "a-", "a.b", &"a".repeat(41)] {
        assert!(!super::plain(key), "{key:?}");
    }
    assert!(super::plain("org-3f2a-person-9c1b"));
    assert!(super::plain(&"a".repeat(40)));
}
