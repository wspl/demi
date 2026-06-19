import { expect, test } from 'bun:test'
import { spawn } from 'node:child_process'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const claudeFixturePath = join(fixtureDir, 'claude')

test('TUI process entry prints help without opening a provider session', async () => {
  const result = await runTuiProcess(['--help'])

  expect(result.exitCode).toBe(0)
  expect(result.stderr).toBe('')
  expect(result.stdout).toContain('Usage: bun run tui -- [workspace] [options]')
  expect(result.stdout).toContain('/input <shellId> <text>')
  expect(result.stdout).not.toContain('claude runtime:')
  expect(result.stdout).not.toContain('Session opened')
})

test('TUI process runs a Claude Code backed session through stdin and renders tool output', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-tui-process-'))
  const child = spawn(
    process.execPath,
    [
      'run',
      'packages/tui/src/index.ts',
      '--cwd',
      workspace,
      '--claude-path',
      claudeFixturePath,
      '--model',
      'opus',
      '--thinking',
      'medium',
      '--budget',
      '0.01',
      '--yield-after-ms',
      '5',
      '--timeout-ms',
      '5000',
    ],
    {
      cwd: process.cwd(),
      env: { ...process.env, PATH: `${fixtureDir}${delimiter}${process.env.PATH ?? ''}` },
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  )
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('Session opened', 5_000)
    child.stdin?.write('run the fixture workflow\n')

    await capture.waitForStdout('usage: in=12 out=3 cache_read=2 cache_write=1', 5_000)
    const stdout = capture.stdout()
    expect(stdout).toContain('claude runtime: ready (fixture claude 0.0.0)')
    expect(stdout).toContain('claude auth: authenticated (fixture@example.test)')
    expect(stdout).toContain('model: claude-opus-4-8 (opus)')
    expect(stdout).toContain('thinking: medium')
    expect(stdout).toContain('tool: shell_exec')
    expect(stdout).toContain('shell[')
    expect(stdout).toContain('fixture-shell')
    expect(stdout).toContain('thinking> fixture plan')
    expect(stdout).toContain('assistant> fixture response')

    child.stdin?.write('/exit\n')
    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
})

async function runTuiProcess(
  args: string[],
  options: { env?: NodeJS.ProcessEnv } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number | null }> {
  const child = spawn(process.execPath, ['run', 'packages/tui/src/index.ts', ...args], {
    cwd: process.cwd(),
    env: options.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const capture = new ProcessCapture(child)
  const exitCode = await capture.closed
  return { stdout: capture.stdout(), stderr: capture.stderr(), exitCode }
}

class ProcessCapture {
  readonly closed: Promise<number | null>
  private stdoutText = ''
  private stderrText = ''
  private readonly waiters: Waiter[] = []

  constructor(
    private readonly child: ReturnType<typeof spawn>,
  ) {
    child.stdout?.on('data', (chunk) => {
      this.stdoutText += Buffer.from(chunk).toString('utf8')
      this.resolveMatchingWaiters()
    })
    child.stderr?.on('data', (chunk) => {
      this.stderrText += Buffer.from(chunk).toString('utf8')
    })
    this.closed = new Promise<number | null>((resolve) => {
      child.once('close', (code) => {
        this.rejectPendingWaiters()
        resolve(code)
      })
      child.once('error', () => {
        this.rejectPendingWaiters()
        resolve(null)
      })
    })
  }

  stdout(): string {
    return this.stdoutText
  }

  stderr(): string {
    return this.stderrText
  }

  waitForStdout(expected: string, timeoutMs: number): Promise<void> {
    if (this.stdoutText.includes(expected)) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const waiter: Waiter = {
        expected,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.removeWaiter(waiter)
          reject(new Error(`Timed out waiting for stdout ${JSON.stringify(expected)}\nstdout:\n${this.stdoutText}\nstderr:\n${this.stderrText}`))
        }, timeoutMs),
      }
      this.waiters.push(waiter)
    })
  }

  private resolveMatchingWaiters(): void {
    for (const waiter of [...this.waiters]) {
      if (!this.stdoutText.includes(waiter.expected)) continue
      this.removeWaiter(waiter)
      waiter.resolve()
    }
  }

  private rejectPendingWaiters(): void {
    for (const waiter of [...this.waiters]) {
      if (this.stdoutText.includes(waiter.expected)) {
        this.removeWaiter(waiter)
        waiter.resolve()
        continue
      }
      this.removeWaiter(waiter)
      waiter.reject(new Error(`Process closed before stdout ${JSON.stringify(waiter.expected)}\nstdout:\n${this.stdoutText}\nstderr:\n${this.stderrText}`))
    }
  }

  private removeWaiter(waiter: Waiter): void {
    clearTimeout(waiter.timer)
    const index = this.waiters.indexOf(waiter)
    if (index !== -1) this.waiters.splice(index, 1)
  }
}

interface Waiter {
  expected: string
  resolve(): void
  reject(error: Error): void
  timer: ReturnType<typeof setTimeout>
}
