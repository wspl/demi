import { abi, cwd, identity } from 'tinyjs:runtime'
import { fileHostStore, type Host } from '@demicodes/shell'
import { normalizePath } from '@demicodes/utils'
import { createRunnerFileSystem } from './fs'
import { createRunnerProcess } from './process'

/** The `tinyjs:*` surface this package is written against. */
export const TINYJS_ABI = 1

export interface RunnerHostOptions {
  /** Default working directory (default: the process's cwd). */
  defaultCwd?: string
  /** Where `Host.store` keeps its JSON files (default `~/.demi/store`). */
  storeDir?: string
}

/**
 * The Host over the tinyjs primitives (`docs/demi-next/tinyjs.md`): the
 * machine's real filesystem and processes, a file-backed store, the
 * process's own identity.
 */
export function createRunnerHost(options: RunnerHostOptions = {}): Host {
  if (abi !== TINYJS_ABI) {
    throw new Error(`this runner is built for tinyjs abi ${TINYJS_ABI}; this tinyjs has abi ${abi}`)
  }
  const defaultCwd = normalizePath(options.defaultCwd ?? cwd())
  const fs = createRunnerFileSystem(defaultCwd)
  return {
    defaultCwd,
    fs,
    process: createRunnerProcess(defaultCwd),
    store: fileHostStore(fs, normalizePath(options.storeDir ?? `${identity.homeDir}/.demi/store`)),
    identity: { uid: identity.uid, gid: identity.gid, hostname: identity.hostname },
  }
}
