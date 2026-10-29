//! How the harness reaches glave in a Pod: always through the API server, so it works wherever
//! glave runs, the Pod takes no connection from the cluster's network, and it works under GKE
//! Sandbox (gVisor), where a port-forward does not (`specs/computers.md`, decision 12). Each
//! connection is one exec of `glave computer bridge` in the Pod, whose standard input and
//! output carry the protocol to the socket glave serves on there, signed in with the token; an
//! exec that drops (an API server restarting) ends only that bridge.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use glave_ports::computer::ComputerError;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{AttachParams, AttachedProcess};
use tokio::io::{AsyncRead, ReadBuf};

use super::super::backend::{Bridge, Stream};
use super::objects::{CONTAINER, GLAVE, SOCKET};

/// Bytes a bridge holds ahead of a slow reader, each way.
const PIPE: usize = 256 * 1024;

/// Bridges into the Pod `name`, each a new exec.
pub(crate) fn bridges(pods: Api<Pod>, name: &str) -> Bridge {
    let name = name.to_owned();
    Arc::new(move || {
        let (pods, name) = (pods.clone(), name.clone());
        Box::pin(async move { bridge(&pods, &name).await })
    })
}

/// One bridge into the Pod `name`: an exec of `glave computer bridge` there, its standard
/// input and output the stream. Its standard error is not taken: unread, it would hold up the
/// output.
async fn bridge(pods: &Api<Pod>, name: &str) -> Result<Stream, ComputerError> {
    let params = AttachParams::default()
        .container(CONTAINER)
        .stdin(true)
        .stdout(true)
        .stderr(false)
        .max_stdin_buf_size(PIPE)
        .max_stdout_buf_size(PIPE);
    let mut process = pods
        .exec(name, [GLAVE, "computer", "bridge", SOCKET], &params)
        .await
        .map_err(|error| {
            ComputerError::Unreachable(format!("could not open a bridge into {name}: {error}"))
        })?;
    let (Some(write), Some(read)) = (process.stdin(), process.stdout()) else {
        return Err(ComputerError::Failed(format!(
            "the bridge into {name} gave no standard input and output"
        )));
    };
    let read = WithExec {
        read: Box::new(read),
        _process: process,
    };
    Ok((Box::new(read), Box::new(write)))
}

/// A bridge's output, holding its exec: when whoever reads it lets go, the exec ends.
struct WithExec {
    read: Box<dyn AsyncRead + Unpin + Send>,
    _process: AttachedProcess,
}

impl AsyncRead for WithExec {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}
