import { cwd, identity } from './runtime'
import { fileHostStore, type Host } from '@demicodes/shell'
import { normalizePath } from '@demicodes/utils'
import { createRunnerFileSystem } from './fs'
import { createRunnerProcess } from './process'

export interface RunnerHostOptions {
  /** Default working directory (default: the process's cwd). */
  defaultCwd?: string
  /** Where `Host.store` keeps its JSON files (default `~/.demi/store`). */
  storeDir?: string
  /** The identity reported (default the process's own): for PID 1, the guest user's. */
  identity?: Host['identity']
}

/**
 * The Host over the txiki.js APIs (`docs/demi-next/txiki.md`): the
 * machine's real filesystem and processes, a file-backed store, the
 * process's own identity.
 */
export function createRunnerHost(options: RunnerHostOptions = {}): Host {
  const defaultCwd = normalizePath(options.defaultCwd ?? cwd())
  const fs = createRunnerFileSystem(defaultCwd)
  return {
    defaultCwd,
    fs,
    process: createRunnerProcess(defaultCwd),
    store: fileHostStore(fs, normalizePath(options.storeDir ?? `${identity.homeDir}/.demi/store`)),
    identity: options.identity ?? { uid: identity.uid, gid: identity.gid, hostname: identity.hostname, homeDir: identity.homeDir },
  }
}
