import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { createInterface } from 'node:readline'
import type { InferenceRequest } from '@demi/provider'
import { buildClaudeArgs, buildClaudeEnv } from './cli'

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
  maxBudgetUsd?: number | string | null
}

export class ClaudeCliTransportFactory implements ClaudeTransportFactory {
  private readonly claudePath: string
  private readonly maxBudgetUsd: number | string | null

  constructor(options: ClaudeCliTransportFactoryOptions | string = {}) {
    if (typeof options === 'string') {
      this.claudePath = options
      this.maxBudgetUsd = null
    } else {
      this.claudePath = options.claudePath ?? 'claude'
      this.maxBudgetUsd = options.maxBudgetUsd ?? null
    }
  }

  async start(request: InferenceRequest): Promise<ClaudeTransport> {
    const child = spawn(
      this.claudePath,
      buildClaudeArgs({
        modelId: request.modelId,
        systemPrompt: request.systemPrompt,
        thinkingEffort: thinkingEffort(request.thinking),
        maxBudgetUsd: this.maxBudgetUsd,
      }),
      {
        cwd: request.cwd,
        env: buildClaudeEnv(),
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    ) as ChildProcessWithoutNullStreams

    return new ChildProcessClaudeTransport(child)
  }
}

class ChildProcessClaudeTransport implements ClaudeTransport {
  private stderr = ''
  private readonly waitPromise: Promise<{ exitCode: number | null; signal?: string }>

  constructor(private readonly child: ChildProcessWithoutNullStreams) {
    this.waitPromise = new Promise((resolve) => {
      child.once('close', (exitCode, signal) => resolve({ exitCode, signal: signal ?? undefined }))
      child.once('error', () => resolve({ exitCode: null }))
    })
    void this.collectStderr()
  }

  async writeJson(value: unknown): Promise<void> {
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
      yield JSON.parse(String(line))
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
    for await (const chunk of this.child.stderr) this.stderr += Buffer.from(chunk).toString('utf8')
  }
}

function thinkingEffort(thinking: InferenceRequest['thinking']): string | null {
  if (!thinking) return null
  if (thinking.type === 'adaptive') return thinking.effort
  if (thinking.type === 'effort') return thinking.effort
  return null
}
