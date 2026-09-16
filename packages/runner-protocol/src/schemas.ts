// The runner wire declared as zod schemas — the single source of truth for
// both message directions (`messages.ts` derives the TS types via `z.infer`).
// Frames are MessagePack, so `Uint8Array` and `Date` arrive as instances and
// the schemas validate instances, never envelopes. Shell-owned shapes
// (`HostIdentity`, `HostSpawnError`, `HostFileStat`, `HostDirent`) keep their
// hand-written types; their validators carry a `z.ZodType<T>` annotation so
// drift is a compile error.
import { z } from 'zod'
import {
  artifactDigestSchema, artifactLocationSchema, nativeTargetSchema,
  editFileSchema, EDIT_JOB_FILES
} from '@demicodes/command-protocol'
import type {
  HostDirent,
  HostFileStat,
  HostIdentity,
  HostSpawnError
} from '@demicodes/shell'

// z.instanceof(Uint8Array) infers the constructor's ArrayBuffer-bound
// generic; the wire carries plain Uint8Array views.
export const bytesSchema = z.custom<Uint8Array>((value) => value instanceof Uint8Array)
const cwd = z.string().optional()

export const jobFileChangeSchema = editFileSchema.extend({
  added: z.number().int().nonnegative(),
  removed: z.number().int().nonnegative(),
}).strict()

const hostIdentitySchema: z.ZodType<HostIdentity> = z.strictObject({
  uid: z.number(),
  gid: z.number(),
  hostname: z.string(),
  homeDir: z.string(),
})

const hostSpawnErrorSchema: z.ZodType<HostSpawnError> = z.strictObject({
  kind: z.enum([
    'executable_not_found',
    'permission_denied',
    'cwd_unusable',
    'is_directory',
    'other'
  ]),
  detail: z.string().optional(),
})

const hostFileStatSchema: z.ZodType<HostFileStat> = z.strictObject({
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

const hostDirentSchema: z.ZodType<HostDirent> = z.strictObject({
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
  readFile: { params: z.strictObject({ path: z.string(), cwd }), result: bytesSchema },
  writeFile: {
    params: z.strictObject({
      path: z.string(),
      data: bytesSchema,
      cwd,
      createParents: z.boolean().optional()
    }),
    result: z.null()
  },
  appendFile: {
    params: z.strictObject({
      path: z.string(),
      data: bytesSchema,
      cwd,
      createParents: z.boolean().optional()
    }),
    result: z.null()
  },
  exists: { params: z.strictObject({ path: z.string(), cwd }), result: z.boolean() },
  stat: {
    params: z.strictObject({ path: z.string(), cwd }),
    result: hostFileStatSchema
  },
  lstat: {
    params: z.strictObject({ path: z.string(), cwd }),
    result: hostFileStatSchema
  },
  readdir: {
    params: z.strictObject({
      path: z.string(),
      cwd,
      withFileTypes: z.boolean().optional()
    }),
    result: z.union([z.array(z.string()), z.array(hostDirentSchema)])
  },
  mkdir: {
    params: z.strictObject({
      path: z.string(),
      cwd,
      recursive: z.boolean().optional()
    }),
    result: z.null()
  },
  rm: {
    params: z.strictObject({
      path: z.string(),
      cwd,
      recursive: z.boolean().optional(),
      force: z.boolean().optional()
    }),
    result: z.null()
  },
  cp: {
    params: z.strictObject({
      path: z.string(),
      destination: z.string(),
      cwd,
      recursive: z.boolean().optional()
    }),
    result: z.null()
  },
  mv: {
    params: z.strictObject({ path: z.string(), destination: z.string(), cwd }),
    result: z.null()
  },
  chmod: {
    params: z.strictObject({ path: z.string(), mode: z.number(), cwd }),
    result: z.null()
  },
  symlink: {
    params: z.strictObject({ target: z.string(), path: z.string(), cwd }),
    result: z.null()
  },
  link: {
    params: z.strictObject({ existingPath: z.string(), path: z.string(), cwd }),
    result: z.null()
  },
  readlink: { params: z.strictObject({ path: z.string(), cwd }), result: z.string() },
  realpath: { params: z.strictObject({ path: z.string(), cwd }), result: z.string() },
  utimes: {
    params: z.strictObject({ path: z.string(), atime: z.date(), mtime: z.date(), cwd }),
    result: z.null()
  },
} as const

export type FsOp = keyof typeof fsOps
export const FS_OPS = Object.keys(fsOps) as FsOp[]
export type FsParams<Op extends FsOp> = z.infer<(typeof fsOps)[Op]['params']>
export type FsResult<Op extends FsOp> = z.infer<(typeof fsOps)[Op]['result']>

/**
 * `fs_stat { id, path, cwd? }` and its siblings: the request of one operation.
 */
export type FsCallMessage = { [Op in FsOp]: {
  type: `fs_${Op}`;
  id: string
} & FsParams<Op> }[FsOp]
/** `fs_ok { id, op, result }` with the result typed by `op`. */
export type FsOkMessage = { [Op in FsOp]: {
  type: 'fs_ok';
  id: string;
  op: Op;
  result: FsResult<Op>
} }[FsOp]

function fsCallSchema<Op extends FsOp>(op: Op) {
  return z.strictObject({ type: z.literal(`fs_${op}`), id: z.string() })
    .extend(fsOps[op].params.shape)
}

function fsOkSchema<Op extends FsOp>(op: Op) {
  return z.strictObject({
    type: z.literal('fs_ok'),
    id: z.string(),
    op: z.literal(op),
    result: fsOps[op].result
  })
}

const fsCallMessageSchema = z.union(
  FS_OPS.map(fsCallSchema) as unknown as [z.ZodType<FsCallMessage>, ...z.ZodType<FsCallMessage>[]]
)
const fsOkMessageSchema = z.union(
  FS_OPS.map(fsOkSchema) as unknown as [z.ZodType<FsOkMessage>, ...z.ZodType<FsOkMessage>[]]
)

/** One changed file of a working tree; the shape `ChangeFile` in `@demicodes/web-ui` renders. */
export const gitChangeSchema = z.strictObject({
  path: z.string(),
  kind: z.enum(['added', 'modified', 'deleted', 'renamed']),
  /** The path before a rename. */
  from: z.string().optional(),
  added: z.number().int().nonnegative(),
  removed: z.number().int().nonnegative(),
})
export type GitChange = z.infer<typeof gitChangeSchema>

/**
 * The working-tree requests the runner answers in process (`runner.md`
 * § Working tree): `git_<op>` requests, `git_ok { id, op, result }` /
 * `git_error { id, code, message }` replies, in the shape of the `fsOps`
 * table. `root` is the directory the request is about; `path` is relative
 * to it.
 */
export const gitOps = {
  changes: {
    params: z.strictObject({ root: z.string() }),
    result: z.strictObject({
      repository: z.boolean(),
      head: z.string().nullable(),
      files: z.array(gitChangeSchema),
      truncated: z.boolean(),
      watched: z.boolean(),
    }),
  },
  show: {
    params: z.strictObject({ root: z.string(), path: z.string() }),
    result: bytesSchema,
  },
} as const

export type GitOp = keyof typeof gitOps
export const GIT_OPS = Object.keys(gitOps) as GitOp[]
export type GitParams<Op extends GitOp> = z.infer<(typeof gitOps)[Op]['params']>
export type GitResult<Op extends GitOp> = z.infer<(typeof gitOps)[Op]['result']>
export type GitChanges = GitResult<'changes'>

export type GitCallMessage = { [Op in GitOp]: {
  type: `git_${Op}`;
  id: string
} & GitParams<Op> }[GitOp]
export type GitOkMessage = { [Op in GitOp]: {
  type: 'git_ok';
  id: string;
  op: Op;
  result: GitResult<Op>
} }[GitOp]

function gitCallSchema<Op extends GitOp>(op: Op) {
  return z.strictObject({ type: z.literal(`git_${op}`), id: z.string() })
    .extend(gitOps[op].params.shape)
}

function gitOkSchema<Op extends GitOp>(op: Op) {
  return z.strictObject({
    type: z.literal('git_ok'),
    id: z.string(),
    op: z.literal(op),
    result: gitOps[op].result
  })
}

const gitCallMessageSchema = z.union(
  GIT_OPS.map(gitCallSchema) as unknown as [z.ZodType<GitCallMessage>, ...z.ZodType<GitCallMessage>[]]
)
const gitOkMessageSchema = z.union(
  GIT_OPS.map(gitOkSchema) as unknown as [z.ZodType<GitOkMessage>, ...z.ZodType<GitOkMessage>[]]
)

/**
 * Why a working-tree request failed: the runner's own outcomes, or an
 * errno-style code from the repository's files (`ENOENT` for a path the last
 * commit does not have).
 */
export type GitErrorCode =
  | 'not_repository'
  | 'busy'
  | 'timeout'
  | 'too_large'
  | 'cancelled'
  | 'internal'
  | (string & {})

const runnerInfoSchema = z.strictObject({
  name: z.string(),
  platform: z.string(),
  version: z.string(),
  nativeTarget: nativeTargetSchema.optional(),
  /**
   * Read synchronously at shell creation, so it must arrive before any Host
   * use.
   */
  identity: hostIdentitySchema,
  /**
   * A runner booted as a managed host's init: it presents its token or is
   * refused, never paired (`managed-hosts.md` § Joining).
   */
  managed: z.boolean().optional(),
})

const streamSchema = z.enum(['stdout', 'stderr'])

/**
 * One end of a pipe as the wire names it (`runner.md` § Pipes): the id the
 * runner reports `pipe_done` under, and the origin-relative URL its end
 * `PUT`s to or `GET`s from with its device token.
 */
export const pipeRefSchema = z.strictObject({ id: z.string(), url: z.string() })
export type PipeRef = z.infer<typeof pipeRefSchema>

/**
 * Where a job's full output lives on the target, and the last bytes of each
 * stream.
 */
const jobOutputSchema = z.strictObject({
  stdoutPath: z.string(),
  stderrPath: z.string(),
  stdoutBytes: z.number(),
  stderrBytes: z.number(),
  stdoutTail: bytesSchema,
  stderrTail: bytesSchema,
})

export const runnerToBackendMessageSchema = z.union([
  z.strictObject({
    type: z.literal('conversation_released'),
    id: z.string(),
    error: z.string().optional(),
  }),
  z.strictObject({
    type: z.literal('artifact_resolve'),
    id: z.string(),
    jobId: z.string(),
    manifestHash: artifactDigestSchema,
    sha256: artifactDigestSchema,
    target: nativeTargetSchema,
  }).strict(),
  z.strictObject({
    type: z.literal('hello'),
    protocol: z.number(),
    /** Absent on an unclaimed first start. */
    deviceToken: z.string().optional(),
    runner: runnerInfoSchema,
  }),
  /** Liveness plus the count of running jobs, which the idle rule reads. */
  z.strictObject({ type: z.literal('pong'), jobs: z.number().int().nonnegative() }),
  /** Every writable volume was synced, or the runner reports the failure. */
  z.strictObject({
    type: z.literal('sync_done'),
    id: z.string(),
    error: z.string().optional()
  }),
  /**
   * A writable volume is nearly full: the runner asks for this total size;
   * `volume_grown` answers.
   */
  z.strictObject({
    type: z.literal('volume_grow'),
    id: z.string(),
    volume: z.enum(['system', 'home']),
    bytes: z.number().int().positive()
  }),
  fsOkMessageSchema,
  /**
   * A failed fs call; `code` carries the errno-style code (ENOENT, …) when
   * there is one.
   */
  z.strictObject({
    type: z.literal('fs_error'),
    id: z.string(),
    code: z.string().optional(),
    message: z.string()
  }),
  gitOkMessageSchema,
  /** A failed working-tree call; `code` is a `GitErrorCode`. */
  z.strictObject({
    type: z.literal('git_error'),
    id: z.string(),
    code: z.string(),
    message: z.string()
  }),
  z.strictObject({
    type: z.literal('spawn_output'),
    spawnId: z.string(),
    stream: z.enum(['stdout', 'stderr']),
    bytes: bytesSchema,
  }),
  z.strictObject({
    type: z.literal('spawn_exit'),
    spawnId: z.string(),
    exitCode: z.number().nullable(),
    signal: z.string().optional(),
    spawnError: hostSpawnErrorSchema.optional(),
  }),
  /** Live output while the job runs, up to the view budget per stream. */
  z.strictObject({
    type: z.literal('job_output'),
    jobId: z.string(),
    stream: streamSchema,
    bytes: bytesSchema
  }),
  /**
   * A registered leaf's guidance while its invocation is active; null clears
   * that invocation.
   */
  z.strictObject({
    type: z.literal('job_running_hint'),
    jobId: z.string(),
    invocationId: z.string(),
    hint: z.string().nullable()
  }),
  z.strictObject({
    type: z.literal('job_exit'),
    jobId: z.string(),
    exitCode: z.number().nullable(),
    signal: z.string().optional(),
    spawnError: hostSpawnErrorSchema.optional(),
    /**
     * The directory the script ended in; absent when bash never ran the script.
     */
    cwd: z.string().optional(),
    output: jobOutputSchema.optional(),
    files: z.array(jobFileChangeSchema).max(EDIT_JOB_FILES),
    filesTruncated: z.boolean(),
  }),
  /**
   * An `rpc` command invoked on the target. `stdin` says whether the process
   * has a pipe on fd 0; the pipe itself travels as an HTTP stream once
   * `rpc_pipes` names it (`runner.md` § Pipes). The live stdin follows as
   * `rpc_stdin` frames.
   */
  z.strictObject({
    type: z.literal('rpc_call'),
    jobId: z.string(),
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
  z.strictObject({
    type: z.literal('rpc_stdin'),
    callId: z.string(),
    bytes: bytesSchema
  }),
  z.strictObject({ type: z.literal('rpc_stdin_end'), callId: z.string() }),
  z.strictObject({ type: z.literal('rpc_cancel'), callId: z.string() }),
  /**
   * This runner's end of a pipe closed: its HTTP exchange completed, or why it
   * did not.
   */
  z.strictObject({
    type: z.literal('pipe_done'),
    pipeId: z.string(),
    ok: z.boolean(),
    error: z.string().optional()
  }),
])

/**
 * Why a hello was refused. `already_connected` is the one outcome a runner
 * retries: the token's live connection may be a half-open socket the
 * backend has not timed out yet.
 */
export const helloErrorCodeSchema = z.enum([
  'unsupported_protocol',
  'unknown_device',
  'already_connected',
  'revoked',
  'internal'
])

export const backendToRunnerMessageSchema = z.union([
  z.discriminatedUnion('type', [
    z.strictObject({
      type: z.literal('conversation_release'),
      id: z.string(),
      conversationId: z.string().min(1),
    }),
    z.strictObject({ type: z.literal('hello_ok'), deviceId: z.string() }),
    z.strictObject({ type: z.literal('claim_pending'), claimToken: z.string() }),
    z.strictObject({ type: z.literal('claimed'), deviceToken: z.string() }),
    z.strictObject({
      type: z.literal('hello_error'),
      code: helloErrorCodeSchema,
      reason: z.string()
    }),
    z.strictObject({ type: z.literal('ping') }),
    /**
     * Flush writable filesystems before the guest is stopped; `sync_done`
     * answers.
     */
    z.strictObject({ type: z.literal('sync'), id: z.string() }),
    /**
     * The named backing image is now `bytes` large, or an error explains why
     * growth failed.
     */
    z.strictObject({
      type: z.literal('volume_grown'),
      id: z.string(),
      volume: z.enum(['system', 'home']),
      bytes: z.number().int().positive(),
      error: z.string().nullable()
    }),
    z.strictObject({
      type: z.literal('spawn'),
      spawnId: z.string(),
      command: z.string(),
      args: z.array(z.string()).optional(),
      cwd: z.string().optional(),
      env: z.record(z.string(), z.string().optional()).optional(),
      inheritEnv: z.boolean().optional(),
      killProcessGroup: z.boolean().optional(),
    }),
    z.strictObject({
      type: z.literal('spawn_stdin'),
      spawnId: z.string(),
      bytes: bytesSchema
    }),
    z.strictObject({ type: z.literal('spawn_stdin_end'), spawnId: z.string() }),
    z.strictObject({
      type: z.literal('spawn_kill'),
      spawnId: z.string(),
      signal: z.string().optional()
    }),
    /**
     * One job: `bash -c script` in `cwd` with exactly `env`; the shell ids
     * ride in `env`. `stdin` / `stdout` attach the job's fd 0 / fd 1 to pipes
     * whose other ends are elsewhere (`runner.md` § Pipes).
     */
    z.strictObject({
      type: z.literal('job_start'),
      jobId: z.string(),
      manifestHash: artifactDigestSchema.optional(),
      conversation: z.string().min(1),
      node: z.string().min(1),
      script: z.string(),
      cwd: z.string(),
      env: z.record(z.string(), z.string()),
      stdin: pipeRefSchema.optional(),
      stdout: pipeRefSchema.optional(),
    }),
    z.strictObject({
      type: z.literal('job_stdin'),
      jobId: z.string(),
      bytes: bytesSchema
    }),
    z.strictObject({ type: z.literal('job_stdin_end'), jobId: z.string() }),
    z.strictObject({
      type: z.literal('job_kill'),
      jobId: z.string(),
      signal: z.string().optional()
    }),
    /**
     * The call's pipe ends, sent before anything else for the call: the runner
     * `PUT`s the process's pipe into `stdin` (present when the call declared
     * one) and `GET`s `stdout` into the process (`runner.md` § Pipes).
     */
    z.strictObject({ type: z.literal('rpc_stdin_pull'), callId: z.string() }),
    z.strictObject({
      type: z.literal('rpc_pipes'),
      callId: z.string(),
      stdin: pipeRefSchema.optional(),
      stdout: pipeRefSchema
    }),
    /** The call's stderr view; stdout is the pipe. */
    z.strictObject({
      type: z.literal('rpc_output'),
      callId: z.string(),
      bytes: bytesSchema
    }),
    /**
     * Follows the stdout pipe's drain, so the process has written everything
     * before it exits with the code.
     */
    z.strictObject({
      type: z.literal('rpc_exit'),
      callId: z.string(),
      exitCode: z.number()
    }),
    /**
     * The command manifest for the runner's cache. Its shape is the loader's
     * (`parseManifest` in `@demicodes/command-loader`), which the runner
     * applies; the protocol carries it opaque so it owns no command types.
     */
    z.strictObject({ type: z.literal('manifest'), manifest: z.unknown() }),
    z.strictObject({
      type: z.literal('artifact_location'),
      id: z.string(),
      location: artifactLocationSchema.optional(),
      error: z.string().optional(),
    }).strict(),
  ]),
  fsCallMessageSchema,
  gitCallMessageSchema,
])
