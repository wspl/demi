---
status: proposed
---

# A server runs on Render, and in any container that withholds capabilities

## Problem

Alex clicks **Deploy on Render** to run Demi without a VPS. Render runs Demi's
container image as root, but with six capabilities only: `chown`,
`dac_override`, `fowner`, `setgid`, `setuid` and `sys_chroot`, with
`NoNewPrivs` set. On a free Render service with Demi 0.2.6017's image:

- The backend never starts: `demi-backend.service did not start: Operation not
  permitted`. Its unit narrows the capability bounding set, which takes
  `CAP_SETPCAP`, and raises `CAP_NET_BIND_SERVICE`, which the container doesn't
  grant. Both fail.
- Without `CAP_KILL`, root may not signal another user's process. The
  supervisor stops the backend, which runs as `demi`, by a signal, and the
  machine manager stops a Simple Isolation Cloud by signaling the processes of
  the Cloud's own user. Both signals fail, so a redeploy leaves them running
  until Render kills the container 30 seconds later, without saving a Cloud.

Docker, Railway and Fly grant Docker's default capabilities, so the container
setups so far never met either. An operator who runs Compose with `cap_drop`
would.

## Decision

Demi runs where the container lets it switch users, whatever else the container
withholds, and Render becomes a fourth setup. Nothing changes where the
capabilities are granted.

### Capabilities a container withholds

The supervisor reads what it holds before it starts a process:

- **Ambient capabilities:** a service gets those its unit asks for that the
  supervisor holds. The container's log names the rest:

  ```text
  demi-server supervise: /opt/demi/current/bin/demi-backend runs without
  NET_BIND_SERVICE, which this container does not grant
  ```

  The backend listens on 3271 in a container, so it doesn't need that one. One
  configured below 1024 there fails to bind and says so.
- **The bounding set:** without `CAP_SETPCAP` it can't be narrowed. Its purpose
  for a service that isn't root is that no program the service runs gains a
  capability, such as a setuid program, so the supervisor sets `no_new_privs`
  on that service, which keeps that promise.

### Signals to another user

A signal the kernel refuses is sent again by a child that becomes the target's
user first, which the kernel always allows:

```text
supervisor (root, no CAP_KILL) -- kill(backend) --> EPERM
supervisor --fork--> child: setuid(demi) -- kill(backend) --> delivered
```

A new crate, `shared-processes`, owns this for both programs, along with the
machine manager's reading of `/proc`, which moves there:

```rust
pub fn signal_process(pid: Pid, signal: Signal) -> io::Result<()>;
pub fn signal_group(group: Pid, signal: Signal) -> io::Result<()>;
```

A group's kill reports success when one member took the signal, and its first
process may have ended, so `signal_group` without `CAP_KILL` finds the users of
the group's members in `/proc` and sends once as each. The supervisor needs no
record of who runs what, which a process it took over from an earlier
supervisor wouldn't have.

A container that can't switch users can't run Demi: the start stops with
`demi-backend.service did not start: Operation not permitted`.

### Render

`render.yaml`, at the repository's root because Render's button reads it there,
describes one web service from `ghcr.io/demi-sh/demi:latest`:

- a disk at `/opt/demi`, 5 GB, which also holds attachments, since Render has no
  object storage;
- `DEMI_BACKEND_PUBLIC_URL` from the service's own `RENDER_EXTERNAL_URL`, so
  Demi reads no Render name;
- `PORT=3271`, a health check on `/api/update`, and `autoDeployTrigger: off`,
  as Render advises for a button's Blueprint;
- the Starter plan, about $8.25 a month with the disk.

The README gains a **Deploy on Render** button that opens Render's deploy page,
and `docs/delivery/render.md` a short guide.

A service with a disk stops for a few seconds on each deploy, and Render gives
it 30 seconds after `SIGTERM`.

## Spike

On November 6, 2026, a free Render service (no disk) in Singapore, with a probe
image and Demi 0.2.6017's image:

- Root, capabilities `0x400cb`, `NoNewPrivs: 1`, no seccomp filter, cgroup v2 at
  the root, 512 MB.
- `setpriv --reuid=991` works; root's `SIGTERM` to a uid 991 process fails and
  the process runs on; a uid 991 process signals another uid 991 process.
- `unshare -m` fails, `unshare -U` works. Demi's Cloud check passed on Simple
  Isolation, with gVisor unavailable and the manager's reason.

With this design built, on 0.2.6043's release:

- **On a free Render service:** the backend starts and passes Render's health
  checks, the log names `NET_BIND_SERVICE` as withheld, and a Cloud starts in
  15 seconds and runs a real conversation's command in 512 MB. On a redeploy,
  the old backend stops on its signal.
- **In Docker with Render's capabilities**, `0x400cb` and `no-new-privileges`,
  where `kill` by root on a Cloud's process says `Operation not permitted`:
  - Stopping the container with a Cloud running takes 1.2 seconds and exits 0.
    After a start, the conversation and the Cloud's files are there.
  - Stopping the manager alone, with the container still up, ends all five of
    the Cloud's processes within a second, through its recovery's fence.

Not run, since a disk needs a card: the Deploy button and `render.yaml` end to
end, including `RENDER_EXTERNAL_URL`'s self-reference, the disk across a redeploy,
and heavy work on 512 MB.

## Prior art

- [systemd](https://github.com/systemd/systemd/blob/main/src/basic/capability-util.c):
  `capability_ambient_set_apply` suppresses an ambient capability that's missing
  from the bounding set, with a debug message. Demi does the same and says it
  where an operator reads. `capability_bounding_set_drop` tolerates a failed
  drop only for a capability the set doesn't hold, and otherwise fails. Demi
  departs here: no service could start in such a container, and `no_new_privs`
  keeps what the set would have promised.
- `su` and `sudo -u` followed by `kill` are how an administrator without
  `CAP_KILL` signals another user. Demi forks instead of running a program, so
  an image needs no such program.

## Alternatives

- **Run the services as root where `CAP_KILL` is missing.** Simpler, but the
  backend would hold every file of every Cloud, which the `demi` user exists to
  prevent.
- **Ask the backend to stop through its manage socket.** It covers the backend
  only, not a Cloud's processes.
- **Have each caller name the target's user.** The supervisor knows a unit's
  user only for a process it started, not for one it took over.
- **Keep `render.yaml` in `deploy/render/`, beside the other setups.** Render's
  dashboard takes a path, but its button's documented address has no way to name
  one.

## Open questions

None.
