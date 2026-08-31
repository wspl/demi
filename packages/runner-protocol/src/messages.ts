import type { HostIdentity, HostSpawnError } from '@demicodes/shell'
import { parsePortableJson, stringifyPortableJson, isRecord } from '@demicodes/utils'

/**
 * Wire protocol between the backend and a runner: one multiplexed connection
 * carrying the claim/auth handshake, liveness pings, and the wire form of the
 * `Host` contract's `fs` and `process` facets. `Host.store` never crosses
 * this protocol — conversation state is backend-local.
 *
 * Messages are text frames of portable JSON (`Uint8Array`/`bigint`/`Date`
 * round-trip via `@demicodes/utils`).
 */

/**
 * Bumped only on incompatible wire changes. The runner binary on user devices
 * is the hardest component to update, so the backend must be able to tell an
 * incompatible runner apart from a broken one (`hello_error`).
 */
export const RUNNER_PROTOCOL_VERSION = 1

export interface RunnerInfo {
  name: string
  platform: string
  version: string
  /** Read synchronously at shell creation, so it must arrive before any Host use. */
  identity: HostIdentity
}

/** Error shape carried for a failed fs call; `code` preserves errno-style codes (ENOENT, …). */
export interface WireCallError {
  message: string
  code?: string
}

/** The `HostFileSystem` method set, proxied one call per message. */
export const HOST_FS_OPS = [
  'readFile',
  'writeFile',
  'appendFile',
  'exists',
  'stat',
  'lstat',
  'readdir',
  'mkdir',
  'rm',
  'cp',
  'mv',
  'chmod',
  'symlink',
  'link',
  'readlink',
  'realpath',
  'utimes',
] as const

export type HostFsOp = (typeof HOST_FS_OPS)[number]

export type RunnerToBackendMessage =
  | {
      type: 'hello'
      protocol: number
      /** Absent on an unclaimed first start. */
      deviceToken?: string
      runner: RunnerInfo
    }
  | { type: 'pong' }
  | { type: 'fs_result'; id: string; ok: true; result: unknown }
  | { type: 'fs_result'; id: string; ok: false; error: WireCallError }
  | { type: 'spawn_output'; spawnId: string; stream: 'stdout' | 'stderr'; bytes: Uint8Array }
  | {
      type: 'spawn_exit'
      spawnId: string
      exitCode: number | null
      signal?: string
      spawnError?: HostSpawnError
    }

export type BackendToRunnerMessage =
  | { type: 'hello_ok'; deviceId: string }
  | { type: 'claim_pending'; claimToken: string }
  | { type: 'claimed'; deviceToken: string }
  | { type: 'hello_error'; reason: string }
  | { type: 'ping' }
  | { type: 'fs_call'; id: string; op: HostFsOp; args: unknown[] }
  | {
      type: 'spawn'
      spawnId: string
      command: string
      args?: string[]
      cwd?: string
      env?: Record<string, string | undefined>
      killProcessGroup?: boolean
    }
  | { type: 'spawn_stdin'; spawnId: string; bytes: Uint8Array }
  | { type: 'spawn_stdin_end'; spawnId: string }
  | { type: 'spawn_kill'; spawnId: string; signal?: string }

export type RunnerProtocolMessage = RunnerToBackendMessage | BackendToRunnerMessage

export function encodeRunnerMessage(message: RunnerProtocolMessage): string {
  return stringifyPortableJson(message)
}

/** Decodes a wire frame; throws on frames that are not runner-protocol messages. */
export function decodeRunnerMessage(frame: string): RunnerProtocolMessage {
  const value = parsePortableJson<unknown>(frame)
  if (!isRecord(value) || typeof value.type !== 'string') {
    throw new Error('Malformed runner-protocol frame')
  }
  return value as RunnerProtocolMessage
}

export function isHostFsOp(value: unknown): value is HostFsOp {
  return typeof value === 'string' && (HOST_FS_OPS as readonly string[]).includes(value)
}
