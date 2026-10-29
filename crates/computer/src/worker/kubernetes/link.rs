//! How the harness reaches glave in a Pod: always through the API server, so it works wherever
//! glave runs, the Pod takes no connection from the cluster's network, and it works under GKE
//! Sandbox (gVisor), where a port-forward does not (`specs/computers.md`, decision 12). Each
//! connection is one exec of `glave computer bridge` in the Pod, whose standard input and
//! output carry the protocol to the socket glave serves on there, signed in with the token; an
//! exec that drops (an API server restarting) ends only that bridge.

use std::pin::Pin;
use std::task::{Context, Poll, ready};

use glave_ports::computer::ComputerError;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{AttachParams, AttachedProcess};
use tokio::io::{AsyncRead, ReadBuf};

use super::super::backend::Stream;
use super::exec_failed;
use super::objects::{CONTAINER, GLAVE, SOCKET};

/// Bytes a bridge holds ahead of a slow reader, each way, and of what it says on its standard
/// error, which it says only as it fails.
const PIPE: usize = 256 * 1024;
/// The most of what a failed bridge said that is kept, for saying why.
const SAID: usize = 4 * 1024;

/// A new bridge into the Pod `name`: an exec of `glave computer bridge` there, its standard
/// input and output the stream.
pub(crate) async fn bridge(pods: Api<Pod>, name: &str) -> Result<Stream, ComputerError> {
    let params = AttachParams::default()
        .container(CONTAINER)
        .stdin(true)
        .stdout(true)
        .stderr(true)
        .max_stdin_buf_size(PIPE)
        .max_stdout_buf_size(PIPE)
        .max_stderr_buf_size(PIPE);
    let mut process = match pods
        .exec(name, [GLAVE, "computer", "bridge", SOCKET], &params)
        .await
    {
        Ok(process) => process,
        Err(error) => {
            let what = format!("could not open a bridge into {name}");
            return Err(exec_failed(&pods, name, &what, &error).await);
        }
    };
    let (Some(write), Some(read), Some(errors)) =
        (process.stdin(), process.stdout(), process.stderr())
    else {
        return Err(ComputerError::Failed(format!(
            "the bridge into {name} gave no standard input and output"
        )));
    };
    let read = WithExec {
        read: Box::new(read),
        errors: Box::new(errors),
        said: Vec::new(),
        ended: false,
        _process: process,
    };
    Ok((Box::new(read), Box::new(write)))
}

/// A bridge's output, holding its exec: when whoever reads it lets go, the exec ends. When the
/// output ends with the bridge having said something on its standard error, as a bridge that
/// found no computer to reach does, that is read as why the connection ended.
struct WithExec {
    read: Box<dyn AsyncRead + Unpin + Send>,
    errors: Box<dyn AsyncRead + Unpin + Send>,
    /// What the bridge said on its standard error, once its output ended.
    said: Vec<u8>,
    /// Whether its output has ended.
    ended: bool,
    _process: AttachedProcess,
}

impl AsyncRead for WithExec {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if !this.ended {
            let before = buf.filled().len();
            ready!(Pin::new(&mut this.read).poll_read(cx, buf))?;
            if buf.filled().len() > before {
                return Poll::Ready(Ok(()));
            }
            this.ended = true;
        }
        // Its output ended, so the bridge has: its standard error ends with it.
        loop {
            let mut chunk = [0_u8; 512];
            let mut read = ReadBuf::new(&mut chunk);
            match ready!(Pin::new(&mut this.errors).poll_read(cx, &mut read)) {
                Ok(()) if !read.filled().is_empty() => {
                    let room = SAID.saturating_sub(this.said.len());
                    this.said.extend(read.filled().iter().take(room).copied());
                }
                Ok(()) | Err(_) => break,
            }
        }
        if this.said.is_empty() {
            return Poll::Ready(Ok(()));
        }
        let said = String::from_utf8_lossy(&this.said).trim().to_owned();
        Poll::Ready(Err(std::io::Error::other(format!(
            "the bridge said: {said}"
        ))))
    }
}
