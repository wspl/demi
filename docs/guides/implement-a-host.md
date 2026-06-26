# Implement a Host

A `Host` is the sandbox boundary for `@demi/shell`. The bash engine never touches
a real filesystem or spawns a real process directly — it goes through the `Host`
you give it. Swap the Host and the same agent runs against a container, a remote
machine, an in-memory sandbox, or a restricted subset of the local disk.

`@demi/host-local` is the Node reference implementation; this guide describes the
contract it satisfies.

## The contract

```ts
interface Host {
  defaultCwd: string
  fs: HostFileSystem      // file operations
  process: HostProcess    // spawning
  store: HostStore        // a small key/value JSON store (scoped per agent session)
}
```

### `fs` — file operations

Every call takes an optional `{ cwd }` to resolve relative paths. The full surface
(see `packages/shell/src/host.ts`):

```ts
interface HostFileSystem {
  readFile(path, opts?): Promise<Uint8Array>
  writeFile(path, data, opts?): Promise<void>      // opts.createParents to mkdir -p
  appendFile(path, data, opts?): Promise<void>
  exists(path, opts?): Promise<boolean>
  stat(path, opts?): Promise<HostFileStat>
  lstat(path, opts?): Promise<HostFileStat>
  readdir(path, opts?): Promise<string[] | HostDirent[]>  // withFileTypes toggles the shape
  mkdir(path, opts?): Promise<void>
  rm(path, opts?): Promise<void>
  cp(path, dest, opts?): Promise<void>
  mv(path, dest, opts?): Promise<void>
  chmod(path, mode, opts?): Promise<void>
  symlink(target, path, opts?): Promise<void>
  link(existingPath, path, opts?): Promise<void>
  readlink(path, opts?): Promise<string>
  realpath(path, opts?): Promise<string>
  utimes(path, atime, mtime, opts?): Promise<void>
}
```

File contents are always `Uint8Array` — encode/decode text with `encodeUtf8` /
`decodeUtf8` from `@demi/utils`.

### `process` — spawning

```ts
interface HostProcess {
  spawn(params: HostSpawnParams): Promise<HostSpawnHandle>
}

interface HostSpawnHandle {
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  output?: AsyncIterable<HostProcessOutputChunk>  // optional interleaved stream (preferred when present)
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<HostSpawnExit>                  // { exitCode, signal }
}
```

If you can preserve the real interleaving of stdout/stderr, expose `output`; the
shell uses it to render output in true order and falls back to the separate
streams otherwise.

### `store` — scoped JSON KV

```ts
interface HostStore {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}
```

Used to persist command artifacts (the `/@` virtual filesystem). A throwaway
in-memory map is a valid implementation for ephemeral sandboxes.

## Wiring it up

```ts
const host: Host = new MyContainerHost(/* ... */)
const harness = createCodingAgentHarness({ host })
const server = new AgentServer({ agent: harness, providers })
```

## Tips

- Enforce your sandbox in the Host, not above it — reject paths that escape the
  jail inside `fs`, restrict `spawn`'s command set, etc. The agent cannot bypass it.
- Keep operations `cwd`-relative; the shell passes a `cwd` per call.
- Reuse `@demi/utils` (`normalizePath`, `isAbsolutePath`, byte helpers) rather than
  re-implementing path math.
