// The runner wire declared as zod schemas — the single source of truth for
// both message directions (`messages.ts` derives the TS types via `z.infer`).
// Frames are MessagePack, so `Uint8Array` and `Date` arrive as instances and
// the schemas validate instances, never envelopes. Shell-owned shapes
// (`HostIdentity`, `HostSpawnError`, `HostFileStat`, `HostDirent`) keep their
// hand-written types; their validators carry a `z.ZodType<T>` annotation so
// drift is a compile error.
import { z } from 'zod'
import type { HostDirent, HostFileStat, HostIdentity, HostSpawnError } from '@demicodes/shell'

// z.instanceof(Uint8Array) infers the constructor's ArrayBuffer-bound
// generic; the wire carries plain Uint8Array views.
const bytesSchema = z.custom<Uint8Array>((value) => value instanceof Uint8Array)
const cwd = z.string().optional()

const hostIdentitySchema: z.ZodType<HostIdentity> = z.object({
  uid: z.number(),
  gid: z.number(),
  hostname: z.string(),
  homeDir: z.string(),
})

const hostSpawnErrorSchema: z.ZodType<HostSpawnError> = z.object({
  kind: z.enum(['executable_not_found', 'permission_denied', 'cwd_unusable', 'is_directory', 'other']),
  detail: z.string().optional(),
})

const hostFileStatSchema: z.ZodType<HostFileStat> = z.object({
  isFile: z.boolean(),
  isDirectory: z.boolean(),
  isSymbolicLink: z.boolean(),
  mode: z.number(),
  size: z.number(),
  mtime: z.date(),
  uid: z.number().optional(),
  gid: z.number().optional(),
  ino: z.number().optional(),
  dev: z.number().optional(),
  nlink: z.number().optional(),
  isCharacterDevice: z.boolean().optional(),
  isFIFO: z.boolean().optional(),
})

const hostDirentSchema: z.ZodType<HostDirent> = z.object({
  name: z.string(),
  isFile: z.boolean(),
  isDirectory: z.boolean(),
  isSymbolicLink: z.boolean(),
})

/**
 * The `HostFileSystem` method set as the wire sees it: one message per
 * operation with its parameters typed, one result shape per operation.
 * `fs_<op>` requests, `fs_ok { id, op, result }` / `fs_error` replies.
 */
export const fsOps = {
  readFile: { params: z.object({ path: z.string(), cwd }), result: bytesSchema },
  writeFile: { params: z.object({ path: z.string(), data: bytesSchema, cwd, createParents: z.boolean().optional() }), result: z.null() },
  appendFile: { params: z.object({ path: z.string(), data: bytesSchema, cwd, createParents: z.boolean().optional() }), result: z.null() },
  exists: { params: z.object({ path: z.string(), cwd }), result: z.boolean() },
  stat: { params: z.object({ path: z.string(), cwd }), result: hostFileStatSchema },
  lstat: { params: z.object({ path: z.string(), cwd }), result: hostFileStatSchema },
  readdir: { params: z.object({ path: z.string(), cwd, withFileTypes: z.boolean().optional() }), result: z.union([z.array(z.string()), z.array(hostDirentSchema)]) },
  mkdir: { params: z.object({ path: z.string(), cwd, recursive: z.boolean().optional() }), result: z.null() },
  rm: { params: z.object({ path: z.string(), cwd, recursive: z.boolean().optional(), force: z.boolean().optional() }), result: z.null() },
  cp: { params: z.object({ path: z.string(), destination: z.string(), cwd, recursive: z.boolean().optional() }), result: z.null() },
  mv: { params: z.object({ path: z.string(), destination: z.string(), cwd }), result: z.null() },
  chmod: { params: z.object({ path: z.string(), mode: z.number(), cwd }), result: z.null() },
  symlink: { params: z.object({ target: z.string(), path: z.string(), cwd }), result: z.null() },
  link: { params: z.object({ existingPath: z.string(), path: z.string(), cwd }), result: z.null() },
  readlink: { params: z.object({ path: z.string(), cwd }), result: z.string() },
  realpath: { params: z.object({ path: z.string(), cwd }), result: z.string() },
  utimes: { params: z.object({ path: z.string(), atime: z.date(), mtime: z.date(), cwd }), result: z.null() },
} as const

export type FsOp = keyof typeof fsOps
export const FS_OPS = Object.keys(fsOps) as FsOp[]
export type FsParams<Op extends FsOp> = z.infer<(typeof fsOps)[Op]['params']>
export type FsResult<Op extends FsOp> = z.infer<(typeof fsOps)[Op]['result']>

/** `fs_stat { id, path, cwd? }` and its siblings: the request of one operation. */
export type FsCallMessage = { [Op in FsOp]: { type: `fs_${Op}`; id: string } & FsParams<Op> }[FsOp]
/** `fs_ok { id, op, result }` with the result typed by `op`. */
export type FsOkMessage = { [Op in FsOp]: { type: 'fs_ok'; id: string; op: Op; result: FsResult<Op> } }[FsOp]

function fsCallSchema<Op extends FsOp>(op: Op) {
  return z.object({ type: z.literal(`fs_${op}`), id: z.string() }).extend(fsOps[op].params.shape)
}

function fsOkSchema<Op extends FsOp>(op: Op) {
  return z.object({ type: z.literal('fs_ok'), id: z.string(), op: z.literal(op), result: fsOps[op].result })
}

const fsCallMessageSchema = z.union(FS_OPS.map(fsCallSchema) as unknown as [z.ZodType<FsCallMessage>, ...z.ZodType<FsCallMessage>[]])
const fsOkMessageSchema = z.union(FS_OPS.map(fsOkSchema) as unknown as [z.ZodType<FsOkMessage>, ...z.ZodType<FsOkMessage>[]])

const runnerInfoSchema = z.object({
  name: z.string(),
  platform: z.string(),
  version: z.string(),
  /** Read synchronously at shell creation, so it must arrive before any Host use. */
  identity: hostIdentitySchema,
  /** A runner booted as a managed host's init: it presents its token or is refused, never paired (`managed-hosts.md` § Joining). */
  managed: z.boolean().optional(),
})

const streamSchema = z.enum(['stdout', 'stderr'])

/**
 * One end of a pipe as the wire names it (`runner.md` § Pipes): the id the
 * runner reports `pipe_done` under, and the origin-relative URL its end
 * `PUT`s to or `GET`s from with its device token.
 */
export const pipeRefSchema = z.object({ id: z.string(), url: z.string() })
export type PipeRef = z.infer<typeof pipeRefSchema>

/** Where a job's full output lives on the target, and the last bytes of each stream. */
const jobOutputSchema = z.object({
  stdoutPath: z.string(),
  stderrPath: z.string(),
  stdoutBytes: z.number(),
  stderrBytes: z.number(),
  stdoutTail: bytesSchema,
  stderrTail: bytesSchema,
})

export const runnerToBackendMessageSchema = z.union([
  z.object({
    type: z.literal('hello'),
    protocol: z.number(),
    /** Absent on an unclaimed first start. */
    deviceToken: z.string().optional(),
    runner: runnerInfoSchema,
  }),
  /** Liveness plus the count of running jobs, which the idle rule reads. */
  z.object({ type: z.literal('pong'), jobs: z.number().int().nonnegative() }),
  /** The home is on disk; `untouched` when nothing wrote to it since this boot (`managed-hosts.md` § Home persistence). */
  z.object({ type: z.literal('sync_done'), id: z.string(), untouched: z.boolean() }),
  /** The home is nearly full: the runner asks for this total size; `home_grown` answers. */
  z.object({ type: z.literal('home_grow'), bytes: z.number().int().positive() }),
  fsOkMessageSchema,
  /** A failed fs call; `code` carries the errno-style code (ENOENT, …) when there is one. */
  z.object({ type: z.literal('fs_error'), id: z.string(), code: z.string().optional(), message: z.string() }),
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
  /** Live output while the job runs, up to the view budget per stream. */
  z.object({ type: z.literal('job_output'), jobId: z.string(), stream: streamSchema, bytes: bytesSchema }),
  z.object({
    type: z.literal('job_exit'),
    jobId: z.string(),
    exitCode: z.number().nullable(),
    signal: z.string().optional(),
    spawnError: hostSpawnErrorSchema.optional(),
    /** The directory the script ended in; absent when bash never ran the script. */
    cwd: z.string().optional(),
    output: jobOutputSchema.optional(),
  }),
  /**
   * An `rpc` command invoked on the target. `stdin` says whether the process
   * has a pipe on fd 0; the pipe itself travels as an HTTP stream once
   * `rpc_pipes` names it (`runner.md` § Pipes). The live stdin follows as
   * `rpc_stdin` frames.
   */
  z.object({
    type: z.literal('rpc_call'),
    callId: z.string(),
    agentSessionId: z.string(),
    shellId: z.string(),
    root: z.string(),
    path: z.array(z.string()),
    argv: z.array(z.string()),
    args: z.record(z.string(), z.unknown()),
    json: z.boolean(),
    cwd: z.string(),
    env: z.record(z.string(), z.string()),
    stdin: z.boolean(),
  }),
  z.object({ type: z.literal('rpc_stdin'), callId: z.string(), bytes: bytesSchema }),
  z.object({ type: z.literal('rpc_stdin_end'), callId: z.string() }),
  z.object({ type: z.literal('rpc_cancel'), callId: z.string() }),
  /** This runner's end of a pipe closed: its HTTP exchange completed, or why it did not. */
  z.object({ type: z.literal('pipe_done'), pipeId: z.string(), ok: z.boolean(), error: z.string().optional() }),
])

/**
 * Why a hello was refused. `already_connected` is the one outcome a runner
 * retries: the token's live connection may be a half-open socket the
 * backend has not timed out yet.
 */
export const helloErrorCodeSchema = z.enum(['unsupported_protocol', 'unknown_device', 'already_connected', 'revoked', 'internal'])

export const backendToRunnerMessageSchema = z.union([
  z.discriminatedUnion('type', [
    z.object({ type: z.literal('hello_ok'), deviceId: z.string() }),
    z.object({ type: z.literal('claim_pending'), claimToken: z.string() }),
    z.object({ type: z.literal('claimed'), deviceToken: z.string() }),
    z.object({ type: z.literal('hello_error'), code: helloErrorCodeSchema, reason: z.string() }),
    z.object({ type: z.literal('ping') }),
    /** Flush the home to disk before the guest is killed; `sync_done` answers. */
    z.object({ type: z.literal('sync'), id: z.string() }),
    /** The home's backing image is now `bytes` large; the runner grows the filesystem into it. */
    z.object({ type: z.literal('home_grown'), bytes: z.number().int().positive() }),
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
    /**
     * One job: `bash -c script` in `cwd` with exactly `env`; the shell ids
     * ride in `env`. `stdin` / `stdout` attach the job's fd 0 / fd 1 to pipes
     * whose other ends are elsewhere (`runner.md` § Pipes).
     */
    z.object({
      type: z.literal('job_start'),
      jobId: z.string(),
      script: z.string(),
      cwd: z.string(),
      env: z.record(z.string(), z.string()),
      stdin: pipeRefSchema.optional(),
      stdout: pipeRefSchema.optional(),
    }),
    z.object({ type: z.literal('job_stdin'), jobId: z.string(), bytes: bytesSchema }),
    z.object({ type: z.literal('job_stdin_end'), jobId: z.string() }),
    z.object({ type: z.literal('job_kill'), jobId: z.string(), signal: z.string().optional() }),
    /**
     * The call's pipe ends, sent before anything else for the call: the runner
     * `PUT`s the process's pipe into `stdin` (present when the call declared
     * one) and `GET`s `stdout` into the process (`runner.md` § Pipes).
     */
    z.object({ type: z.literal('rpc_pipes'), callId: z.string(), stdin: pipeRefSchema.optional(), stdout: pipeRefSchema }),
    /** The call's stderr view; stdout is the pipe. */
    z.object({ type: z.literal('rpc_output'), callId: z.string(), bytes: bytesSchema }),
    /** Follows the stdout pipe's drain, so the process has written everything before it exits with the code. */
    z.object({ type: z.literal('rpc_exit'), callId: z.string(), exitCode: z.number() }),
    /**
     * The command manifest for the runner's cache. Its shape is the loader's
     * (`parseManifest` in `@demicodes/command-loader`), which the runner
     * applies; the protocol carries it opaque so it owns no command types.
     */
    z.object({ type: z.literal('manifest'), manifest: z.unknown() }),
  ]),
  fsCallMessageSchema,
])
