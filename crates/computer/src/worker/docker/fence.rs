//! A worker with a network refuses cloud metadata, as one on Kubernetes does in its network
//! policy: a short-lived helper container joins the worker's network namespace with
//! `NET_ADMIN` and nothing else, runs `glave computer fence` (the list routed to `prohibit`,
//! DNS on port 53 kept), and is removed, all before the worker is handed any command. The
//! worker itself never holds `NET_ADMIN`, so nothing in it can take the routes away. The
//! helper runs glave alone, never the image's own entrypoint.

use http::Method;
use serde_json::{Value, json};

use super::super::backend::{Backend as _, Spec, saying};
use super::{Docker, HOME, program};
use crate::worker::tar::archive;
use glave_ports::computer::ComputerError;

/// The most memory the helper may use: glave, briefly.
const MEMORY: i64 = 256 << 20;

impl Docker {
    /// Refuses cloud metadata in the started worker `worker`'s network namespace, with
    /// `glave` as the program that does it.
    pub(super) async fn fence(
        &self,
        worker: &str,
        spec: &Spec,
        glave: &[u8],
    ) -> Result<(), ComputerError> {
        let name = format!("{worker}-fence");
        let fenced = self.fence_with(&name, worker, spec, glave).await;
        let said = if fenced.is_err() {
            self.logs(&name).await
        } else {
            String::new()
        };
        let removed = self.remove(&name).await;
        fenced.map_err(|error| saying(error, &said))?;
        removed
    }

    async fn fence_with(
        &self,
        name: &str,
        worker: &str,
        spec: &Spec,
        glave: &[u8],
    ) -> Result<(), ComputerError> {
        let helper = json!({
            "Image": spec.image,
            "Entrypoint": [format!("{HOME}/glave")],
            "Cmd": ["computer", "fence"],
            "User": "0",
            "Labels": self.labels(name, spec),
            "HostConfig": {
                "NetworkMode": format!("container:{worker}"),
                "CapDrop": ["ALL"],
                "CapAdd": ["NET_ADMIN"],
                "SecurityOpt": ["no-new-privileges"],
                "PidsLimit": 16,
                "Memory": MEMORY,
                "MemorySwap": MEMORY,
            },
        });
        self.ok(
            Method::POST,
            &format!("/containers/create?name={}", super::query(name)),
            Some(&helper),
        )
        .await?;
        let files = archive(&program(glave)).map_err(|long| {
            ComputerError::Failed(format!("{} is too long a name for tar", long.0))
        })?;
        self.put(name, files).await?;
        self.ok(Method::POST, &format!("/containers/{name}/start"), None)
            .await?;
        let ended = self
            .ok(
                Method::POST,
                &format!("/containers/{name}/wait?condition=not-running"),
                None,
            )
            .await?;
        match ended.get("StatusCode").and_then(Value::as_i64) {
            Some(0) => Ok(()),
            code => Err(ComputerError::Failed(format!(
                "could not keep the worker from cloud metadata (exit code {})",
                code.unwrap_or(-1)
            ))),
        }
    }
}
