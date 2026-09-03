import type { z } from 'zod'
import { backendToRunnerMessageSchema, helloErrorCodeSchema, runnerToBackendMessageSchema } from './schemas'

/**
 * Wire protocol between the backend and a runner: one multiplexed connection
 * carrying the claim/auth handshake, liveness pings, and the wire form of the
 * `Host` contract's `fs` and `process` facets. `Host.store` never crosses
 * this protocol — conversation state is backend-local.
 *
 * Frames are MessagePack (`Uint8Array` as bin, `Date` as the timestamp
 * extension, `undefined` as nil), so bytes and times are native wire types.
 * The codec is the carrier's: `@msgpack/msgpack` on Bun (`codec.ts`),
 * `tinyjs:bytes` on tinyjs. The message set is declared as zod schemas in
 * `schemas.ts` — the single source of truth these types derive from — and
 * each end validates the direction it receives.
 */

/**
 * Bumped only on incompatible wire changes. The runner binary on user devices
 * is the hardest component to update, so the backend must be able to tell an
 * incompatible runner apart from a broken one (`hello_error`).
 */
export const RUNNER_PROTOCOL_VERSION = 4

/**
 * The view budget per stream of a job: what crosses the wire is the model's
 * window and nothing more — the first bytes while the job runs, the last
 * bytes at exit (`runner.md` § Jobs and the tee).
 */
export const JOB_VIEW_BYTES = 32 * 1024

export type { FsCallMessage, FsOkMessage, FsOp, FsParams, FsResult } from './schemas'
export { FS_OPS } from './schemas'

export type RunnerToBackendMessage = z.infer<typeof runnerToBackendMessageSchema>
export type BackendToRunnerMessage = z.infer<typeof backendToRunnerMessageSchema>
export type RunnerProtocolMessage = RunnerToBackendMessage | BackendToRunnerMessage

export type RunnerInfo = Extract<RunnerToBackendMessage, { type: 'hello' }>['runner']
export type JobExitMessage = Extract<RunnerToBackendMessage, { type: 'job_exit' }>
export type JobOutput = NonNullable<JobExitMessage['output']>
export type RpcCallMessage = Extract<RunnerToBackendMessage, { type: 'rpc_call' }>
export type HelloErrorCode = z.infer<typeof helloErrorCodeSchema>

/** A MessagePack codec: the two ends bring their own (`msgpackCodec`, `tinyjs:bytes`). */
export interface MessagePackCodec {
  encode(value: unknown): Uint8Array
  decode(bytes: Uint8Array): unknown
}

/** One end's framing: encode any message, decode and validate the inbound direction. */
export interface RunnerWire {
  encode(message: RunnerProtocolMessage): Uint8Array
  /** A frame arriving at the backend (runner → backend). */
  decodeRunnerToBackend(frame: Uint8Array): RunnerToBackendMessage
  /** A frame arriving at the runner (backend → runner). */
  decodeBackendToRunner(frame: Uint8Array): BackendToRunnerMessage
}

export function createRunnerWire(codec: MessagePackCodec): RunnerWire {
  return {
    encode: (message) => codec.encode(message),
    decodeRunnerToBackend: (frame) => decodeWith(runnerToBackendMessageSchema, codec, frame),
    decodeBackendToRunner: (frame) => decodeWith(backendToRunnerMessageSchema, codec, frame),
  }
}

function decodeWith<Schema extends z.ZodType>(schema: Schema, codec: MessagePackCodec, frame: Uint8Array): z.infer<Schema> {
  let value: unknown
  try {
    value = codec.decode(frame)
  } catch (error) {
    throw new Error(`Malformed runner-protocol frame: ${error instanceof Error ? error.message : String(error)}`)
  }
  const parsed = schema.safeParse(value)
  if (!parsed.success) {
    const issue = parsed.error.issues[0]
    throw new Error(`Malformed runner-protocol frame${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}`)
  }
  return parsed.data as z.infer<Schema>
}
