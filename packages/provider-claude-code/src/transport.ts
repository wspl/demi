import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { statSync } from 'node:fs'
import process from 'node:process'
import { encodeUtf8, utf8Lines } from '@demicodes/utils'
import type { InferenceRequest } from '@demicodes/provider'
import { buildClaudeArgs, buildClaudeEnv } from './cli'
import type { ClaudeSpawn, ClaudeSpawnExit, ClaudeSpawnHandle, ClaudeSpawnParams } from './spawn'
import { createClaudeWireLog, type ClaudeWireLog } from './wire-log'

// The session cwd is a logical workspace id for some hosts (e.g. a virtual
// filesystem), not a real directory. The CLI's tools come from the agent, not
// the process's working directory, so spawn in a real directory rather than
// letting posix_spawn fail with ENOENT on a non-existent cwd.
function resolveSpawnCwd(cwd: string): string {
  try {
    if (statSync(cwd).isDirectory()) return cwd
  } catch {
    // not a real directory — fall through
  }
  return process.cwd()
}

export interface ClaudeTransport {
  writeJson(value: unknown): Promise<void>
  messages(): AsyncIterable<unknown>
  kill(): Promise<void>
  wait(): Promise<{ exitCode: number | null; signal?: string }>
  stderrText(): string
}

export interface ClaudeTransportFactory {
  start(request: InferenceRequest): Promise<ClaudeTransport>
}

export interface ClaudeCliTransportFactoryOptions {
  claudePath?: string
  /** Resolve OAuth token for CLAUDE_CODE_OAUTH_TOKEN env overlay (multi-cred). */
  resolveOAuthAccessToken?: () => Promise<string | null>
  /**
   * `Host.process`-shaped spawn the CLI runs through. When set, the child env
   * is built from an empty base (never the local `process.env` — the spawn
   * implementation owns merging with its machine's environment) and the
   * request cwd is passed through untranslated.
   */
  spawn?: ClaudeSpawn
  /**
   * Public env overlay applied last (wins over everything, including the
   * resolved OAuth token) — e.g. `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN`.
   */
  env?: Record<string, string>
}

export class ClaudeCliTransportFactory implements ClaudeTransportFactory {
  private readonly claudePath: string
  private readonly resolveOAuthAccessToken: (() => Promise<string | null>) | null
  private readonly spawnFn: ClaudeSpawn | null
  private readonly envOverlay: Record<string, string> | null

  constructor(options: ClaudeCliTransportFactoryOptions | string = {}) {
    if (typeof options === 'string') {
      this.claudePath = options
      this.resolveOAuthAccessToken = null
      this.spawnFn = null
      this.envOverlay = null
    } else {
      this.claudePath = options.claudePath ?? 'claude'
      this.resolveOAuthAccessToken = options.resolveOAuthAccessToken ?? null
      this.spawnFn = options.spawn ?? null
      this.envOverlay = options.env ?? null
    }
  }

  async start(request: InferenceRequest): Promise<ClaudeTransport> {
    const args = buildClaudeArgsForRequest(request)
    const wireLog = createClaudeWireLog(request.sessionId)
    wireLog.record('spawn', {
      requestId: request.requestId,
      turnId: request.turnId,
      model: request.modelId,
      cwd: request.cwd,
      args,
    })
    const oauthAccessToken = this.resolveOAuthAccessToken ? await this.resolveOAuthAccessToken() : null
    const overlay = this.envOverlay ?? undefined
    const handle = this.spawnFn
      ? await this.spawnFn({
          command: this.claudePath,
          args,
          cwd: request.cwd,
          // Injected-spawn targets are managed devices: the CLI must consume
          // zero device-local configuration (settings, hooks, sessions), so
          // its config home is pinned inside the workspace's artifacts dir.
          env: buildClaudeEnv({}, {
            oauthAccessToken,
            overlay: { CLAUDE_CONFIG_DIR: `${request.cwd}/.demi-artifacts/claude-config`, ...overlay },
          }),
        })
      : await localClaudeSpawn({
          command: this.claudePath,
          args,
          cwd: resolveSpawnCwd(request.cwd),
          env: buildClaudeEnv(process.env, { oauthAccessToken, overlay }),
        })

    return new SpawnHandleClaudeTransport(handle, wireLog)
  }
}

export function buildClaudeArgsForRequest(request: InferenceRequest): string[] {
  return buildClaudeArgs({
    modelId: request.modelId,
    systemPrompt: request.systemPrompt,
    thinkingEffort: thinkingEffort(request.thinking),
  })
}

/** Local default: wraps `child_process.spawn` into the `ClaudeSpawnHandle` shape. */
async function localClaudeSpawn(params: ClaudeSpawnParams): Promise<ClaudeSpawnHandle> {
  const child = spawn(params.command, params.args ?? [], {
    cwd: params.cwd,
    env: params.env as NodeJS.ProcessEnv,
    stdio: ['pipe', 'pipe', 'pipe'],
  }) as ChildProcessWithoutNullStreams

  const exit = new Promise<ClaudeSpawnExit>((resolve) => {
    child.once('close', (exitCode, signal) => {
      resolve({ exitCode, signal: signal ?? undefined })
    })
    child.once('error', () => {
      resolve({ exitCode: null, spawnError: { kind: 'other' } })
    })
  })

  return {
    stdout: child.stdout,
    stderr: child.stderr,
    writeStdin: (data) =>
      new Promise<void>((resolve, reject) => {
        child.stdin.write(data, (error) => {
          if (error) reject(error)
          else resolve()
        })
      }),
    closeStdin: async () => {
      child.stdin.end()
    },
    kill: async () => {
      if (!child.killed) child.kill('SIGTERM')
    },
    wait: () => exit,
  }
}

class SpawnHandleClaudeTransport implements ClaudeTransport {
  private stderr = ''
  private readonly waitPromise: Promise<{ exitCode: number | null; signal?: string }>

  constructor(
    private readonly handle: ClaudeSpawnHandle,
    private readonly wireLog: ClaudeWireLog,
  ) {
    this.waitPromise = handle.wait().then((exit) => {
      this.wireLog.record('exit', {
        exitCode: exit.exitCode,
        signal: exit.signal ?? null,
        ...(exit.spawnError ? { spawnError: exit.spawnError.kind } : {}),
      })
      return { exitCode: exit.exitCode, signal: exit.signal }
    })
    void this.collectStderr()
  }

  async writeJson(value: unknown): Promise<void> {
    this.wireLog.record('in', value)
    await this.handle.writeStdin(encodeUtf8(`${JSON.stringify(value)}\n`))
  }

  async *messages(): AsyncIterable<unknown> {
    for await (const line of utf8Lines(this.handle.stdout)) {
      if (line.trim() === '') continue
      const parsed = JSON.parse(line)
      this.wireLog.record('out', parsed)
      yield parsed
    }
  }

  async kill(): Promise<void> {
    await this.handle.kill('SIGTERM')
  }

  wait(): Promise<{ exitCode: number | null; signal?: string }> {
    return this.waitPromise
  }

  stderrText(): string {
    return this.stderr
  }

  private async collectStderr(): Promise<void> {
    const decoder = new TextDecoder()
    for await (const chunk of this.handle.stderr) {
      const text = decoder.decode(chunk, { stream: true })
      if (text.length === 0) continue
      this.stderr += text
      this.wireLog.record('err', text)
    }
  }
}

function thinkingEffort(thinking: InferenceRequest['thinking']): string | null {
  if (!thinking) return null
  if (thinking.type === 'adaptive') return thinking.effort
  if (thinking.type === 'effort') return thinking.effort
  return null
}
