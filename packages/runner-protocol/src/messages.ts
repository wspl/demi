import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { z } from 'zod'
import { HOST_FS_OPS, backendToRunnerMessageSchema, runnerToBackendMessageSchema } from './schemas'

/**
 * Wire protocol between the backend and a runner: one multiplexed connection
 * carrying the claim/auth handshake, liveness pings, and the wire form of the
 * `Host` contract's `fs` and `process` facets. `Host.store` never crosses
 * this protocol — conversation state is backend-local.
 *
 * Messages are text frames of portable JSON (`Uint8Array`/`bigint`/`Date`
 * round-trip via `@demicodes/utils`). The message set is declared as zod
 * schemas in `schemas.ts` — the single source of truth these types derive
 * from — and each end validates the direction it receives.
 */

/**
 * Bumped only on incompatible wire changes. The runner binary on user devices
 * is the hardest component to update, so the backend must be able to tell an
 * incompatible runner apart from a broken one (`hello_error`).
 */
export const RUNNER_PROTOCOL_VERSION = 1

export { HOST_FS_OPS }

export type HostFsOp = (typeof HOST_FS_OPS)[number]

export type RunnerToBackendMessage = z.infer<typeof runnerToBackendMessageSchema>
export type BackendToRunnerMessage = z.infer<typeof backendToRunnerMessageSchema>
export type RunnerProtocolMessage = RunnerToBackendMessage | BackendToRunnerMessage

export type RunnerInfo = Extract<RunnerToBackendMessage, { type: 'hello' }>['runner']
export type WireCallError = Extract<RunnerToBackendMessage, { type: 'fs_result'; ok: false }>['error']

export function encodeRunnerMessage(message: RunnerProtocolMessage): string {
  return stringifyPortableJson(message)
}

/** Decodes and validates a frame arriving at the backend (runner → backend direction). */
export function decodeRunnerToBackendMessage(frame: string): RunnerToBackendMessage {
  return decodeWith(runnerToBackendMessageSchema, frame)
}

/** Decodes and validates a frame arriving at the runner (backend → runner direction). */
export function decodeBackendToRunnerMessage(frame: string): BackendToRunnerMessage {
  return decodeWith(backendToRunnerMessageSchema, frame)
}

function decodeWith<Schema extends z.ZodType>(schema: Schema, frame: string): z.infer<Schema> {
  const parsed = schema.safeParse(parsePortableJson<unknown>(frame))
  if (!parsed.success) {
    const issue = parsed.error.issues[0]
    throw new Error(`Malformed runner-protocol frame${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}`)
  }
  return parsed.data as z.infer<Schema>
}
