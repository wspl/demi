import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { createInterface } from 'node:readline'
import type { InferenceRequest } from '@demicodes/provider'
import { buildClaudeArgs, buildClaudeEnv } from './cli'
import { createClaudeWireLog, type ClaudeWireLog } from './wire-log'

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
}

export class ClaudeCliTransportFactory implements ClaudeTransportFactory {
  private readonly claudePath: string

  constructor(options: ClaudeCliTransportFactoryOptions | string = {}) {
    if (typeof options === 'string') {
      this.claudePath = options
    } else {
      this.claudePath = options.claudePath ?? 'claude'
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
    const child = spawn(this.claudePath, args, {
      cwd: request.cwd,
      env: buildClaudeEnv(),
      stdio: ['pipe', 'pipe', 'pipe'],
    }) as ChildProcessWithoutNullStreams

    return new ChildProcessClaudeTransport(child, wireLog)
  }
}

export function buildClaudeArgsForRequest(request: InferenceRequest): string[] {
  return buildClaudeArgs({
    modelId: request.modelId,
    systemPrompt: request.systemPrompt,
    thinkingEffort: thinkingEffort(request.thinking),
  })
}

class ChildProcessClaudeTransport implements ClaudeTransport {
  private stderr = ''
  private readonly waitPromise: Promise<{ exitCode: number | null; signal?: string }>

  constructor(
    private readonly child: ChildProcessWithoutNullStreams,
    private readonly wireLog: ClaudeWireLog,
  ) {
    this.waitPromise = new Promise((resolve) => {
      child.once('close', (exitCode, signal) => {
        this.wireLog.record('exit', { exitCode, signal: signal ?? null })
        resolve({ exitCode, signal: signal ?? undefined })
      })
      child.once('error', (error) => {
        this.wireLog.record('exit', { error: error.message })
        resolve({ exitCode: null })
      })
    })
    void this.collectStderr()
  }

  async writeJson(value: unknown): Promise<void> {
    this.wireLog.record('in', value)
    const line = `${JSON.stringify(value)}\n`
    await new Promise<void>((resolve, reject) => {
      this.child.stdin.write(line, (error) => {
        if (error) reject(error)
        else resolve()
      })
    })
  }

  async *messages(): AsyncIterable<unknown> {
    const rl = createInterface({ input: this.child.stdout })
    for await (const line of rl) {
      if (String(line).trim() === '') continue
      const parsed = JSON.parse(String(line))
      this.wireLog.record('out', parsed)
      yield parsed
    }
  }

  async kill(): Promise<void> {
    if (!this.child.killed) this.child.kill('SIGTERM')
  }

  wait(): Promise<{ exitCode: number | null; signal?: string }> {
    return this.waitPromise
  }

  stderrText(): string {
    return this.stderr
  }

  private async collectStderr(): Promise<void> {
    for await (const chunk of this.child.stderr) {
      const text = Buffer.from(chunk).toString('utf8')
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
