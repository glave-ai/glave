//! The seccomp filter every sandboxed command runs under, Codex's: no `ptrace` or reading and
//! writing other processes' memory, no `io_uring` (which can open sockets without `socket`),
//! and the network by socket family. With the network off, only Unix sockets may be made and
//! nothing may connect, bind, listen or send to an address; with it on, only internet sockets
//! may be made, so no Unix socket on the machine (`docker.sock`, the ssh agent) is reachable,
//! as on macOS. Built with Firecracker's `seccompiler`; a blocked call fails with `EPERM`.

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};

/// Blocked always.
const ALWAYS: &[i64] = &[
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
];

/// Blocked when the network is off.
const NETWORK: &[i64] = &[
    libc::SYS_connect,
    libc::SYS_accept,
    libc::SYS_accept4,
    libc::SYS_bind,
    libc::SYS_listen,
    libc::SYS_getpeername,
    libc::SYS_getsockname,
    libc::SYS_shutdown,
    libc::SYS_sendto,
    libc::SYS_sendmmsg,
    // `recvfrom` stays: tools such as `cargo clippy` talk to their children over socket pairs.
    libc::SYS_recvmmsg,
    libc::SYS_getsockopt,
    libc::SYS_setsockopt,
];

/// The filter, compiled for this processor.
pub(super) fn program(network: bool) -> Result<BpfProgram, String> {
    let family_is_not = |families: &[i32]| {
        let conditions = families
            .iter()
            .map(|family| {
                SeccompCondition::new(
                    0,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Ne,
                    u64::from(family.unsigned_abs()),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        SeccompRule::new(conditions)
    };
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    for call in ALWAYS {
        rules.insert(*call, Vec::new());
    }
    let only_unix = family_is_not(&[libc::AF_UNIX]).map_err(failed)?;
    rules.insert(libc::SYS_socketpair, vec![only_unix.clone()]);
    if network {
        let only_internet = family_is_not(&[libc::AF_INET, libc::AF_INET6]).map_err(failed)?;
        rules.insert(libc::SYS_socket, vec![only_internet]);
    } else {
        for call in NETWORK {
            rules.insert(*call, Vec::new());
        }
        rules.insert(libc::SYS_socket, vec![only_unix]);
    }
    let rules = with_x32(rules);
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM.unsigned_abs()),
        arch()?,
    )
    .map_err(failed)?;
    BpfProgram::try_from(filter).map_err(failed)
}

fn failed(error: impl std::fmt::Display) -> String {
    format!("could not build the seccomp filter: {error}")
}

/// The filter as bubblewrap reads it from a file descriptor: the instructions' raw bytes.
pub(super) fn bytes(program: &BpfProgram) -> Vec<u8> {
    let mut out = Vec::with_capacity(program.len() * 8);
    for instruction in program {
        out.extend(instruction.code.to_ne_bytes());
        out.push(instruction.jt);
        out.push(instruction.jf);
        out.extend(instruction.k.to_ne_bytes());
    }
    out
}

/// Applies the filter to this thread and what it runs next (the helper, before `exec`).
pub(super) fn apply(network: bool) -> Result<(), String> {
    let program = program(network)?;
    seccompiler::apply_filter(&program)
        .map_err(|error| format!("could not apply the seccomp filter: {error}"))
}

fn arch() -> Result<TargetArch, String> {
    TargetArch::try_from(std::env::consts::ARCH).map_err(|_| {
        format!(
            "the sandbox has no seccomp filter for this processor ({})",
            std::env::consts::ARCH
        )
    })
}

/// On x86-64, the same rules for the x32 calls, which share its architecture tag: a 32-bit
/// program could otherwise make the calls the filter blocks under their x32 numbers.
#[cfg(target_arch = "x86_64")]
fn with_x32(rules: BTreeMap<i64, Vec<SeccompRule>>) -> BTreeMap<i64, Vec<SeccompRule>> {
    const X32: i64 = 0x4000_0000;
    let x32 = |call: i64| match call {
        libc::SYS_ptrace => 521,
        libc::SYS_recvmmsg => 537,
        libc::SYS_sendmmsg => 538,
        libc::SYS_process_vm_readv => 539,
        libc::SYS_process_vm_writev => 540,
        libc::SYS_setsockopt => 541,
        libc::SYS_getsockopt => 542,
        common => common,
    };
    let mut all = rules.clone();
    all.extend(
        rules
            .into_iter()
            .map(|(call, rule)| (X32 | x32(call), rule)),
    );
    all
}

#[cfg(not(target_arch = "x86_64"))]
fn with_x32(rules: BTreeMap<i64, Vec<SeccompRule>>) -> BTreeMap<i64, Vec<SeccompRule>> {
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_builds_for_both_network_settings() {
        for network in [false, true] {
            let program = program(network).unwrap();
            assert!(program.len() > 3, "more than the architecture check");
            assert_eq!(bytes(&program).len(), program.len() * 8);
        }
        assert_ne!(program(false).unwrap(), program(true).unwrap());
    }
}
