# Glave security: the client/server trust boundary

The harness is client/server-shaped, and that split *is* the moat. Two parties:

- **Brain (server, trusted).** Holds the IP: orchestration logic, prompt
  construction, model routing/selection, and the model API keys. Runs in our
  environment. Never ships its logic or secrets to the client.
- **Actuator (client, untrusted).** A sandbox that runs *doubly-untrusted* code:
  the target repo's code **and** shell commands an external model chose. Assume
  it can be fully compromised; it must still be unable to reach our secrets, our
  logic, or the host.

The boundary is the `WorkspaceRuntime` port: the brain sends concrete **actions**
(read / write / run / list) and gets **observations** back. Nothing else crosses.
A remote adapter implementing the same port is the cloud split — no core change.

## What's contained today (Docker adapter)

- **Secrets never cross.** `docker run`/`exec` pass no `--env`; the container does
  not inherit the host environment. The model key lives only in the brain process.
- **Logic never crosses.** The actuator sees files (bind mount) + shell commands.
  Never prompts, routing, or selection.
- **No egress.** `--network none`, now **fail-closed**: a non-"none" request in a
  task file is ignored unless `GLAVE_ALLOW_NETWORK` is set by the operator.
- **Privilege.** `--cap-drop ALL`, `--security-opt no-new-privileges`.
- **Resources.** memory + memory-swap (no swap) + cpus + pids caps → host DoS-safe.
- **Oracle integrity.** Mounted read-only; the actuator can't tamper with grading.
- **Ephemeral.** Container + run dir destroyed after each task.
- **No host shell injection.** `Command` arg arrays, not a host shell, carry model
  commands; they execute only inside the container via `docker exec sh -c`.
- **File-path confinement.** Model-supplied paths (`read`/`create`/`str_replace`)
  resolve through `safe_join`: relative + normal components only, and the closest
  existing ancestor must canonicalize within the workspace — defeating `../`
  traversal *and* symlink escape. (The file IO runs host-side, so this matters.)
- **MCP trust.** Workspace-provided MCP config (`.glave/mcp.json`) is off unless
  the operator sets `GLAVE_TRUST_WORKSPACE_MCP`, so an untrusted repo can't spawn
  brain-side processes; spawned servers get a scrubbed env (no secrets leak in).
  The trusted path is operator-supplied configs via `McpHub::from_configs`.

## Known gaps / roadmap (in priority order)

1. **Run as non-root + read-only rootfs.** Container still runs as the image's
   default user. Add `--user` + `--read-only` with tmpfs for build caches.
   (cap-drop + no-new-privileges already defang root-in-container.)
2. **Disposable microVM** (Firecracker) for the SWE-bench-Pro tier, where the
   image itself may be untrusted — shared-kernel container isolation isn't a
   boundary against kernel exploits.
3. **Image supply chain** for untrusted upstream images: digest-pin + local
   mirror + scan (scanning is hygiene, not a boundary).
4. **Remote brain/actuator split** (the serving crate): authenticated channel,
   brain server-side only. The port is already the seam; deferred until the
   brain's capability surface stabilizes so the protocol isn't churned.
