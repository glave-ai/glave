//! How the harness reaches a worker on Kubernetes, always through the API server, so it works
//! wherever glave runs and the worker takes no connection from the cluster's network. With a
//! network: a listener on 127.0.0.1 here whose every connection is port-forwarded to glave's
//! TLS port in the Pod, so the TLS session (pinned certificate, token) runs end to end inside
//! it. Without: attached to glave's standard input and output, as with Docker.

use glave_ports::computer::ComputerError;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{AttachParams, AttachedProcess, Portforwarder};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::super::backend::{Link, PORT};
use super::objects::CONTAINER;
use crate::tls::Fingerprint;

/// Bytes a stream link holds ahead of a slow reader.
const PIPE: usize = 256 * 1024;
/// The most connections forwarded to one worker at once.
const CONNECTIONS: usize = 16;

/// Ends a port-forward or an attach when whoever holds it stops.
struct Closing<T: Abort>(T);

trait Abort {
    fn abort(&self);
}

impl Abort for Portforwarder {
    fn abort(&self) {
        Portforwarder::abort(self);
    }
}

impl Abort for AttachedProcess {
    fn abort(&self) {
        AttachedProcess::abort(self);
    }
}

impl<T: Abort> Drop for Closing<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A TLS link to the worker `name`: a listener on 127.0.0.1 forwarding to its port.
pub(crate) async fn forwarded(
    pods: Api<Pod>,
    name: &str,
    fingerprint: Fingerprint,
) -> Result<Link, ComputerError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| {
        ComputerError::Failed(format!("could not listen on 127.0.0.1 to forward: {error}"))
    })?;
    let address = listener.local_addr().map_err(|error| {
        ComputerError::Failed(format!("the forwarding listener has no address: {error}"))
    })?;
    let name = name.to_owned();
    let forward = tokio::spawn(async move { accept(listener, pods, name).await });
    Ok(Link::Tls {
        address,
        fingerprint,
        forward: Some(forward),
    })
}

/// Forwards each connection to the listener until stopped; stopping ends them all.
async fn accept(listener: TcpListener, pods: Api<Pod>, name: String) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { break };
                if connections.len() >= CONNECTIONS {
                    continue;
                }
                connections.spawn(one(socket, pods.clone(), name.clone()));
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

/// One connection, port-forwarded to glave's port in the Pod. A worker not listening yet
/// closes it, and the harness tries again.
async fn one(mut socket: TcpStream, pods: Api<Pod>, name: String) {
    let Ok(forwarder) = pods.portforward(&name, &[PORT]).await else {
        return;
    };
    let mut forwarder = Closing(forwarder);
    let Some(mut upstream) = forwarder.0.take_stream(PORT) else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut upstream).await;
}

/// A stream link attached to glave's standard input and output in the worker `name`.
pub(crate) async fn attached(pods: &Api<Pod>, name: &str) -> Result<Link, ComputerError> {
    let params = AttachParams::default()
        .container(CONTAINER)
        .stdin(true)
        .stdout(true)
        .stderr(false)
        .max_stdin_buf_size(PIPE)
        .max_stdout_buf_size(PIPE);
    let mut process = pods.attach(name, &params).await.map_err(|error| {
        ComputerError::Failed(format!("could not attach to the worker {name}: {error}"))
    })?;
    let (Some(write), Some(read)) = (process.stdin(), process.stdout()) else {
        return Err(ComputerError::Failed(format!(
            "attaching to the worker {name} gave no standard input and output"
        )));
    };
    let ended = process.take_status();
    let process = Closing(process);
    let task = tokio::spawn(async move {
        // Held until the attachment ends or the worker is removed.
        let _process = process;
        if let Some(ended) = ended {
            let _ = ended.await;
        } else {
            std::future::pending::<()>().await;
        }
    });
    Ok(Link::Stream {
        read: Box::new(read),
        write: Box::new(write),
        task,
    })
}
