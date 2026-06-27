# @demicodes/host-local

The Node reference implementation of the `@demicodes/shell` `Host` contract: real
filesystem, process spawning, and a temp-dir-backed JSON store. Use it to run
Demi against the local machine, or as the template for a remote/container/sandbox
`Host`.

```ts
import { LocalHost } from '@demicodes/host-local'

const host = new LocalHost(process.cwd())
```

> `LocalHost` grants full local filesystem and process access. For untrusted
> agents, supply a sandboxing `Host` instead — see
> [Implement a Host](../../docs/guides/implement-a-host.md).

Part of [Demi](../../README.md). Apache-2.0.
