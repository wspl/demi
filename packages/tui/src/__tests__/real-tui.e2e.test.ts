import { expect, test } from 'bun:test'
import { spawn } from 'node:child_process'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ProcessCapture } from './process-helpers'

const e2e = process.env.DEMI_TUI_REAL_E2E === '1' ? test : test.skip
const attempts = Math.max(1, Number.parseInt(process.env.DEMI_TUI_REAL_E2E_ATTEMPTS ?? '2', 10))

e2e(
  'TUI streams real Claude Code tool output and observes thinking across repeated opus medium runs',
  async () => {
    const outputs: string[] = []
    for (let attempt = 0; attempt < attempts; attempt++) outputs.push(await runRealTuiSmokeOnce())
    expect(outputs.some((stdout) => stdout.includes('thinking>'))).toBe(true)
  },
  attempts * 150_000,
)

async function runRealTuiSmokeOnce(): Promise<string> {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-tui-real-'))
  const child = spawn(
    process.execPath,
    [
      'run',
      'packages/tui/src/index.ts',
      '--cwd',
      workspace,
      '--model',
      'opus',
      '--thinking',
      'medium',
      '--budget',
      process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.25',
      '--yield-after-ms',
      '250',
      '--timeout-ms',
      '120000',
    ],
    { cwd: process.cwd(), env: process.env, stdio: ['pipe', 'pipe', 'pipe'] },
  )
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('Session opened', 20_000)
    child.stdin?.write(
      [
        'First solve this internally: let a_1=7 and a_{n+1}=(a_n*37+19) mod 1009 for n=1..18.',
        'Then use the shell tool to run exactly: printf DEMI_REAL_TUI_TOOL_OK',
        'After the tool result, reply with exactly DEMI_REAL_TUI_TEXT_OK and no extra words.',
      ].join(' ') + '\n',
    )

    await capture.waitForStdout('DEMI_REAL_TUI_TEXT_OK', 120_000)
    await capture.waitForStdout('usage:', 120_000)
    const stdout = capture.stdout()
    expect(stdout).toContain('model: claude-opus-4-8 (opus)')
    expect(stdout).toContain('thinking: medium')
    expect(stdout).toContain('tool: shell_exec')
    expect(stdout).toContain('DEMI_REAL_TUI_TOOL_OK')
    expect(stdout).toContain('assistant>')
    expect(stdout).toContain('usage:')

    child.stdin?.write('/exit\n')
    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
    return capture.stdout()
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
}
