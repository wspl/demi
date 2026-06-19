import { expect, test } from 'bun:test'
import { spawn } from 'node:child_process'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ProcessCapture } from './process-helpers'

const e2e = process.env.DEMI_TUI_REAL_E2E === '1' ? test : test.skip
const attempts = Math.max(1, Number.parseInt(process.env.DEMI_TUI_REAL_E2E_ATTEMPTS ?? '2', 10))
const fuzzyShellE2e = process.env.DEMI_TUI_FUZZY_SHELL_E2E === '1' ? test : test.skip
const fuzzyShellAttempts = Math.max(1, Number.parseInt(process.env.DEMI_TUI_FUZZY_SHELL_E2E_ATTEMPTS ?? '3', 10))

e2e(
  'TUI streams real Claude Code tool output and observes thinking across repeated opus medium runs',
  async () => {
    const outputs: string[] = []
    for (let attempt = 0; attempt < attempts; attempt++) outputs.push(await runRealTuiSmokeOnce())
    expect(outputs.some((stdout) => stdout.includes('thinking>'))).toBe(true)
  },
  attempts * 150_000,
)

fuzzyShellE2e(
  'TUI real model smoke chooses wait, input, and abort for a fuzzy shell-control task',
  async () => {
    for (let attempt = 0; attempt < fuzzyShellAttempts; attempt++) {
      const stdout = await runRealTuiFuzzyShellOnce()
      expect(stdout).toContain('tool: shell_exec')
      expect(stdout).toContain('tool: shell_wait')
      expect(stdout).toContain('tool: shell_input')
      expect(stdout).toContain('tool: shell_abort')
      expect(stdout).toContain('DEMI_FUZZY_INPUT:')
      expect(stdout).toContain('DEMI_FUZZY_SHELL_OK')
      expect(stdout).toContain('usage:')
    }
  },
  fuzzyShellAttempts * 180_000,
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

async function runRealTuiFuzzyShellOnce(): Promise<string> {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-tui-fuzzy-shell-'))
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
      process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.35',
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
        'Use only local shell work in this scratch workspace.',
        'Do not ask me for clarification and do not install packages.',
        'I need you to handle two small foreground process checks.',
        'First, run a local command of your choice that waits for one line on stdin and then prints DEMI_FUZZY_INPUT:<that line>.',
        'Before sending stdin, make a separate shell_wait call to observe that it is still running; do not count the initial shell_exec running result as this observation.',
        'Then send a non-empty line using the shell control tool.',
        'Second, run a local command of your choice that would keep running for a while.',
        'Make a separate shell_wait call to observe it once, then stop it intentionally using the shell control tool.',
        'When both checks are complete, reply with exactly DEMI_FUZZY_SHELL_OK and no extra words.',
      ].join(' ') + '\n',
    )

    await capture.waitForStdout('DEMI_FUZZY_SHELL_OK', 150_000)
    await capture.waitForStdout('usage:', 150_000)
    const stdout = capture.stdout()
    expect(stdout).toContain('model: claude-opus-4-8 (opus)')
    expect(stdout).toContain('thinking: medium')

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
