# Cloud runtime inputs

`release.json` pins the gVisor release the machine manager runs: upstream's
version, the same on amd64 and arm64, and the SHA-512 of each architecture's
archive. `demi-server` fetches the archive of the server's architecture from
upstream, checks its SHA-512 and unpacks what the manager runs into
`/opt/demi/gvisor/<version>/`
([gVisor runtime](../../../docs/delivery/builds-and-releases.md#gvisor-runtime)).
The runtime profile belongs to
[Managed hosts](../../../docs/cloud/managed-hosts.md#isolation-and-joining).

## Adopting a new version

A new pin passes the Cloud acceptance on both Linux architectures
([Verification](../../../docs/cloud/managed-hosts.md#verification)) and the
checks of two bugs earlier versions had:

- **The seccomp trap's first argument, on arm64.** When a seccomp filter
  returns `SECCOMP_RET_TRAP`, Linux delivers SIGSYS with X0 still holding the
  syscall's first argument; gVisor before release-20261005.0 wrote the
  syscall number there instead, and Chrome's sandbox, whose signal handler
  reads that argument, crashed. Demi ran a patched arm64 build until upstream
  fixed it. `seccomp-trap.c` traps a `getpid` call whose first argument is
  `0x123456` and prints what the handler finds in X0; on arm64, natively and
  under the new `runsc`, both runs must print `0x123456`:

  ```sh
  cc -O2 -static -o seccomp-trap seccomp-trap.c
  ./seccomp-trap
  sudo runsc --platform=systrap --network=none do ./seccomp-trap
  ```

- **Short reads of a host Unix socket.** A read of a host-backed Unix stream
  socket returns at most the socket's `SO_SNDBUF`, even when more is queued
  ([google/gvisor#15639](https://github.com/google/gvisor/issues/15639)), so
  the backend keeps its runner socket's connections from queueing more
  ([Backend socket](../../../docs/cloud/managed-hosts.md#backend-socket)). A
  pin that fixes it lets the workaround in `crates/backend-http/src/listener.rs`
  go, once the Cloud suite's large attachments arrive without it.
