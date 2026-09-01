// The runner wire declared as zod schemas — the single source of truth for
// both message directions (`messages.ts` derives the TS types via `z.infer`).
// Each end validates its inbound direction after portable-codec decode; the
// codec produces real `Uint8Array` values, so schemas validate instances,
// never base64 envelopes. Shell-owned shapes (`HostIdentity`,
// `HostSpawnError`) keep their hand-written types; their validators carry a
// `z.ZodType<T>` annotation so drift is a compile error.
import { z } from 'zod'
import type { HostIdentity, HostSpawnError } from '@demicodes/shell'

// z.instanceof(Uint8Array) infers the constructor's ArrayBuffer-bound
// generic; the wire carries plain Uint8Array views.
const bytesSchema = z.custom<Uint8Array>((value) => value instanceof Uint8Array)

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

const hostIdentitySchema: z.ZodType<HostIdentity> = z.object({
  uid: z.number(),
  gid: z.number(),
  hostname: z.string(),
})

const hostSpawnErrorSchema: z.ZodType<HostSpawnError> = z.object({
  kind: z.enum(['executable_not_found', 'permission_denied', 'cwd_unusable', 'is_directory', 'other']),
  detail: z.string().optional(),
})

const runnerInfoSchema = z.object({
  name: z.string(),
  platform: z.string(),
  version: z.string(),
  /** Read synchronously at shell creation, so it must arrive before any Host use. */
  identity: hostIdentitySchema,
})

/** Error shape carried for a failed fs call; `code` preserves errno-style codes (ENOENT, …). */
const wireCallErrorSchema = z.object({
  message: z.string(),
  code: z.string().optional(),
})

// A plain union: the two fs_result branches share the `type` discriminator
// (they discriminate on `ok`), which discriminatedUnion refuses.
export const runnerToBackendMessageSchema = z.union([
  z.object({
    type: z.literal('hello'),
    protocol: z.number(),
    /** Absent on an unclaimed first start. */
    deviceToken: z.string().optional(),
    runner: runnerInfoSchema,
  }),
  z.object({ type: z.literal('pong') }),
  z.object({ type: z.literal('fs_result'), id: z.string(), ok: z.literal(true), result: z.unknown() }),
  z.object({ type: z.literal('fs_result'), id: z.string(), ok: z.literal(false), error: wireCallErrorSchema }),
  z.object({
    type: z.literal('spawn_output'),
    spawnId: z.string(),
    stream: z.enum(['stdout', 'stderr']),
    bytes: bytesSchema,
  }),
  z.object({
    type: z.literal('spawn_exit'),
    spawnId: z.string(),
    exitCode: z.number().nullable(),
    signal: z.string().optional(),
    spawnError: hostSpawnErrorSchema.optional(),
  }),
])

export const backendToRunnerMessageSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('hello_ok'), deviceId: z.string() }),
  z.object({ type: z.literal('claim_pending'), claimToken: z.string() }),
  z.object({ type: z.literal('claimed'), deviceToken: z.string() }),
  z.object({ type: z.literal('hello_error'), reason: z.string() }),
  z.object({ type: z.literal('ping') }),
  z.object({ type: z.literal('fs_call'), id: z.string(), op: z.enum(HOST_FS_OPS), args: z.array(z.unknown()) }),
  z.object({
    type: z.literal('spawn'),
    spawnId: z.string(),
    command: z.string(),
    args: z.array(z.string()).optional(),
    cwd: z.string().optional(),
    env: z.record(z.string(), z.string().optional()).optional(),
    killProcessGroup: z.boolean().optional(),
  }),
  z.object({ type: z.literal('spawn_stdin'), spawnId: z.string(), bytes: bytesSchema }),
  z.object({ type: z.literal('spawn_stdin_end'), spawnId: z.string() }),
  z.object({ type: z.literal('spawn_kill'), spawnId: z.string(), signal: z.string().optional() }),
])
