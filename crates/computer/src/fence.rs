//! Cloud metadata refused in a network namespace (`glave computer fence`), as a worker on
//! Docker needs: every block of [`CLOUD_METADATA`](glave_ports::host::CLOUD_METADATA) routed
//! to `prohibit` in a table of glave's own, which every connection but DNS on port 53 is
//! looked up in before the namespace's own routes, so one to a metadata address fails at once
//! while DNS to Azure's resolver (one of the list) still goes, as on Kubernetes. A short-lived
//! helper holding `NET_ADMIN` runs it in a worker's namespace before the worker is handed any
//! command; the worker never holds `NET_ADMIN`, so nothing in it can take the routes away.

// Built in tests everywhere, so the messages are checked on any machine.
#[cfg(any(target_os = "linux", test))]
mod netlink;

/// Refuses cloud metadata in this process's network namespace; IPv6 too unless the namespace
/// has none.
pub fn fence() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        netlink::fence()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "refusing cloud metadata in a network namespace needs Linux",
        ))
    }
}
