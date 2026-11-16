"""What a command on a person's computer can reach, find and become, one `check=result` a line.

Run by `contained.rs` on CI's `kind` cluster: as a command on a person's computer outside
glave's sandbox, as Full access runs one (`all`), and inside it with the network (`net`); in a
Pod with no network policy, as the control that shows each address inside answers (`control`,
which then stands in for glave's server, listening on 4180); and on another person's computer,
listening on 8080 for anyone who gets in (`listen`). The addresses come as JSON in the second
argument.

Each address is tried once, a TCP handshake and nothing more, within five seconds, all at once;
the only bytes sent are one DNS question to the cluster's own DNS and one HTTPS request to
example.com. No scanning, no ranges, nothing aimed outside the cluster but example.com.
"""

import concurrent.futures
import ctypes
import json
import os
import re
import socket
import stat
import struct
import sys
import time
import urllib.request

WAIT = 5
LISTEN_FOR = 150
GVISOR_VERSION = "#1 SMP Sun Jan 10 15:06:54 PST 2016"


def connect(host, port):
    """One handshake with `host` at `port`: reached, refused (and why), or unresolved."""
    try:
        family, kind, proto, _, address = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)[0]
    except OSError:
        return "unresolved"
    try:
        with socket.socket(family, kind, proto) as s:
            s.settimeout(WAIT)
            s.connect(address)
            return "reached"
    except TimeoutError:
        return "refused timeout"
    except OSError as error:
        return f"refused {error.errno}"


def ask(server, name):
    """One DNS question for `name` (an A record) to the DNS server at `server`."""
    question = b"".join(bytes([len(part)]) + part.encode() for part in name.split("."))
    query = struct.pack(">HHHHHH", 0x6C76, 0x0100, 1, 0, 0, 0) + question + b"\0\0\1\0\1"
    family = socket.AF_INET6 if ":" in server else socket.AF_INET
    try:
        with socket.socket(family, socket.SOCK_DGRAM) as s:
            s.settimeout(WAIT)
            s.sendto(query, (server, 53))
            s.recv(512)
            return "answered"
    except TimeoutError:
        return "refused timeout"
    except OSError as error:
        return f"refused {error.errno}"


def resolves(name):
    """Whether this computer's own resolver gives `name` an address."""
    try:
        socket.getaddrinfo(name, 443)
        return "resolved"
    except OSError:
        return "unresolved"


def https(url):
    """One HTTPS request: reached when it answers 200."""
    try:
        with urllib.request.urlopen(url, timeout=20) as answer:
            return "reached" if answer.status == 200 else f"answered {answer.status}"
    except OSError as error:
        return f"refused {error}"


def inside(t):
    """Everything inside the cluster a computer must not reach, by name: how to try it."""
    return {
        "api_service": lambda: connect(t["api_service"], 443),
        "api_name": lambda: connect("kubernetes.default.svc.cluster.local", 443),
        "api_endpoint": lambda: connect(t["api_endpoint"], t["api_endpoint_port"]),
        "kubelet": lambda: connect(t["node"], 10250),
        "server_pod": lambda: connect(t["server"], 4180),
        "server_service": lambda: connect(t["server_service"], 4180),
        "server_name": lambda: connect(t["server_name"], 4180),
        "another_computer": lambda: connect(t["another_computer"], 8080),
        "another_pod": lambda: connect(t["another_pod"], 9153),
        "cluster_dns": lambda: ask(t["cluster_dns"], "kubernetes.default.svc.cluster.local"),
        "dns_pod": lambda: connect(t["another_pod"], 53),
        "internal_name": lambda: resolves("kubernetes.default.svc.cluster.local"),
        "metadata": lambda: connect("169.254.169.254", 80),
        "metadata_name": lambda: connect("metadata.google.internal", 80),
        "metadata_gke": lambda: connect("169.254.169.252", 988),
        "metadata_v6": lambda: connect("fd00:ec2::254", 80),
    }


def network(t, control):
    """Every address tried at once, and, for a computer, the internet by name."""
    checks = inside(t)
    if control:
        # The control is no computer: these it has no reason to try.
        for name in ["server_pod", "server_service", "server_name", "metadata", "metadata_name",
                     "metadata_gke", "metadata_v6"]:
            del checks[name]
    else:
        checks["internet"] = lambda: https("https://example.com/")
    with concurrent.futures.ThreadPoolExecutor(len(checks)) as pool:
        tried = {name: pool.submit(check) for name, check in checks.items()}
        return {name: future.result() for name, future in tried.items()}


def status():
    """This process's own lines of /proc/self/status, by name."""
    with open("/proc/self/status") as lines:
        return dict(line.rstrip("\n").split(":\t", 1) for line in lines if ":\t" in line)


def refused(attempt):
    """`attempt()`'s errno when it fails, or what it did."""
    try:
        return attempt()
    except OSError as error:
        return f"refused {error.errno}"


def libc_call(name, *args):
    """A call into libc: refused with its errno when it returns -1, else what it returned."""
    libc = ctypes.CDLL(None, use_errno=True)
    result = getattr(libc, name)(*args)
    return f"refused {ctypes.get_errno()}" if result == -1 else str(result)


def credentials():
    """Credentials a command could find: a service account, a kubeconfig, secret variables."""
    accounts = ["/var/run/secrets/kubernetes.io/serviceaccount", "/run/secrets/kubernetes.io"]
    configs = [os.path.expanduser(p) for p in ["~/.kube/config", "~/.config/gcloud", "~/.aws"]]
    configs.append("/etc/kubernetes")
    secretive = re.compile("TOKEN|SECRET|PASSW|CREDENTIAL|API_?KEY|PRIVATE", re.IGNORECASE)
    # Names only, never a value.
    variables = sorted(name for name in os.environ if secretive.search(name))
    servers = []
    for top in ["/home/glave", "/opt/glave", "/etc", "/run", "/var"]:
        for folder, _, files in os.walk(top):
            servers += [os.path.join(folder, f) for f in files if f in ("root.key", "serve.token")]
    return {
        "service_account": "present" if any(map(os.path.exists, accounts)) else "absent",
        "cloud_configs": ",".join(p for p in configs if os.path.exists(p)) or "none",
        "secret_variables": ",".join(variables) or "none",
        "server_files": ",".join(servers) or "none",
    }


def mounts():
    """Mount points beside the Pod's own: its volumes, its files and the kernel's folders."""
    own = {"/", "/etc/hosts", "/etc/hostname", "/etc/resolv.conf", "/dev/termination-log",
           "/opt/glave", "/opt/glave/secret", "/home/glave", "/tmp"}
    kernel = ("/proc", "/sys", "/dev")
    with open("/proc/self/mountinfo") as lines:
        points = {line.split()[4] for line in lines}
    other = sorted(p for p in points if p not in own and not p.startswith(kernel))
    return ",".join(other) or "pod-only"


def node():
    """What of the machine underneath shows: its runtime's sockets, devices, processes."""
    sockets = ["/run/containerd/containerd.sock", "/var/run/containerd/containerd.sock",
               "/run/docker.sock", "/var/run/docker.sock", "/run/crio/crio.sock",
               "/var/run/crio/crio.sock", "/run/k3s/containerd/containerd.sock",
               "/var/run/cri-dockerd.sock", "/run/runsc", "/var/lib/kubelet"]
    devices = []
    for name in os.listdir("/dev"):
        mode = os.lstat(os.path.join("/dev", name)).st_mode
        if stat.S_ISBLK(mode) or name in ("mem", "kmem", "port", "kmsg", "nvram"):
            devices.append(name)
    host = {"kubelet", "containerd", "dockerd", "systemd", "init", "runsc", "runsc-sandbox",
            "runsc-gofer", "kube-proxy", "sshd", "containerd-shim", "kindnetd"}
    seen = set()
    for pid in filter(str.isdigit, os.listdir("/proc")):
        try:
            with open(f"/proc/{pid}/comm") as comm:
                seen.add(comm.read().strip())
        except OSError:
            pass
    with open("/proc/1/cmdline", "rb") as first:
        pid1 = os.path.basename(first.read().split(b"\0")[0].decode())

    def opened_for_writing(path):
        os.close(os.open(path, os.O_WRONLY))
        return "opened"

    return {
        "runtime_sockets": ",".join(p for p in sockets if os.path.exists(p)) or "none",
        "devices": ",".join(sorted(devices)) or "none",
        "mounts": mounts(),
        "pid1": pid1,
        "host_processes": ",".join(sorted(seen & host)) or "none",
        "pid1_root": refused(lambda: f"listed {len(os.listdir('/proc/1/root'))}"),
        "proc_sys": refused(lambda: opened_for_writing("/proc/sys/vm/drop_caches")),
        "cgroup": refused(lambda: opened_for_writing("/sys/fs/cgroup/cgroup.procs")),
    }


def privilege():
    """Whether it is root or can become it: ids, capabilities, setuid, mount, ptrace."""
    own = status()
    caps = [f"{name}={own[name]}" for name in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"]
            if int(own.get(name, "0"), 16)]
    os.makedirs("/tmp/glave-probe-mount", exist_ok=True)
    mounted = libc_call("mount", b"none", b"/tmp/glave-probe-mount", b"tmpfs", 0, None)
    if mounted == "0":
        libc_call("umount", b"/tmp/glave-probe-mount")
    # PTRACE_ATTACH to the Pod's first process, someone else's; detached at once were it allowed.
    traced = libc_call("ptrace", 16, 1, None, None)
    if traced == "0":
        libc_call("ptrace", 17, 1, None, None)
    return {
        "ids": f"{os.getuid()}/{os.geteuid()}/{os.getgid()}/{os.getegid()}",
        "capabilities": ",".join(caps) or "none",
        # PR_GET_NO_NEW_PRIVS.
        "no_new_privs": libc_call("prctl", 39, 0, 0, 0, 0),
        "seccomp": own.get("Seccomp", "?"),
        "setuid_root": refused(lambda: (os.setuid(0), "root")[1]),
        "mount": "mounted" if mounted == "0" else mounted,
        "ptrace": "attached" if traced == "0" else traced,
    }


def kernel():
    """gVisor's own kernel, which says so in its log, or the host's, by its release."""
    libc = ctypes.CDLL(None, use_errno=True)
    log = ctypes.create_string_buffer(1 << 16)
    # SYSLOG_ACTION_READ_ALL.
    read = libc.klogctl(3, log, len(log))
    # gVisor's kernel names itself in its log, and its version is one fixed string.
    if (read > 0 and b"gVisor" in log.raw[:read]) or os.uname().version == GVISOR_VERSION:
        return "gvisor"
    return f"linux {os.uname().release}"


def listen(port, then=None):
    """Listens on `port`, says so, runs `then`, and names everyone it accepts for a while."""
    server = socket.create_server(("0.0.0.0", port))
    print("listening", flush=True)
    if then:
        then()
    server.settimeout(1)
    ends = time.monotonic() + LISTEN_FOR
    while time.monotonic() < ends:
        try:
            peer, address = server.accept()
            print(f"accepted {address[0]}", flush=True)
            peer.close()
        except TimeoutError:
            pass


def show(found):
    for name, result in found.items():
        print(f"{name}={result}", flush=True)


def main():
    mode = sys.argv[1]
    targets = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
    if mode == "listen":
        listen(8080)
    elif mode == "control":
        listen(4180, then=lambda: (show(network(targets, control=True)), print("probed", flush=True)))
    elif mode == "net":
        show(network(targets, control=False))
    else:
        show(network(targets, control=False))
        show(credentials())
        show(node())
        show(privilege())
        show({"kernel": kernel()})


main()
