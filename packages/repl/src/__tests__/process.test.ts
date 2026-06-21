import { expect, test } from 'bun:test'
import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { ProcessCapture } from './process-helpers'

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const claudeFixturePath = join(fixtureDir, 'claude')
const ttyE2e = process.env.DEMI_REPL_TTY_E2E === '1' ? test : test.skip

test('REPL process entry prints help without opening a provider session', async () => {
  const result = await runReplProcess(['--help'])

  expect(result.exitCode).toBe(0)
  expect(result.stderr).toBe('')
  expect(result.stdout).toContain('Usage: bun run repl -- [cwd] [options]')
  expect(result.stdout).toContain('/input <shellId> <text>')
  expect(result.stdout).not.toContain('claude runtime:')
  expect(result.stdout).not.toContain('session opened')
})

test('REPL process runs a Claude Code backed session through stdin and renders tool output', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-repl-process-'))
  const child = spawnReplFixture(workspace)
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('session opened', 5_000)
    child.stdin?.write('run the fixture workflow\n')

    await capture.waitForStdout('usage> in=12 out=3 cache_read=2 cache_write=1', 5_000)
    const stdout = capture.stdout()
    expect(stdout).not.toContain('claude runtime:')
    expect(stdout).not.toContain('claude auth:')
    expect(stdout).toContain('model     claude-opus-4-8')
    expect(stdout).toContain('thinking  medium')
    expect(stdout).toContain('tool> shell_exec')
    expect(stdout).toContain('shell[')
    expect(stdout).toContain('fixture-shell')
    expect(stdout).toContain('thinking> fixture plan')
    expect(stdout).toContain('assistant> fixture response')

    child.stdin?.write('/exit\n')
    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
    expect(capture.stdout()).not.toContain('demi>')
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
})

test('REPL process sends slash input to a running shell and renders the resulting stdout', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-repl-process-input-'))
  const child = spawnReplFixture(workspace)
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('session opened', 5_000)
    child.stdin?.write('run the interactive input fixture workflow\n')

    await capture.waitForStdout('assistant> fixture input ready', 5_000)
    const shellId = extractFirstShellId(capture.stdout())
    child.stdin?.write(`/input ${shellId} typed-from-repl\n`)

    await capture.waitForStdout('shell[', 5_000)
    await capture.waitForStdout('fixture-input:typed-from-repl', 5_000)
    expect(capture.stdout()).toContain(`shell[${shellId}] stdout> fixture-input:typed-from-repl`)

    child.stdin?.write('/exit\n')
    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
})

test('REPL process renders high-volume shell output without losing sentinel lines', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-repl-process-flood-'))
  const child = spawnReplFixture(workspace)
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('session opened', 5_000)
    child.stdin?.write('run the flood output fixture workflow\n')

    await capture.waitForStdout('DEMI_FLOOD_END', 10_000)
    await capture.waitForStdout('usage> in=14 out=5 cache_read=0 cache_write=0', 5_000)
    const stdout = capture.stdout()
    expect(stdout).toContain('shell[')
    expect(stdout).toContain('DEMI_FLOOD_START')
    expect(stdout).toContain('flood-0000')
    expect(stdout).toContain('flood-1499')
    expect(stdout).toContain('DEMI_FLOOD_END')
    expect(stdout).toContain('assistant> fixture flood complete')

    child.stdin?.write('/exit\n')
    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
})

ttyE2e('REPL process runs a fixture session under a real pseudo terminal', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-repl-process-tty-'))
  const child = spawnReplFixtureInPty(workspace)
  const capture = new ProcessCapture(child)

  try {
    await capture.waitForStdout('usage> in=12 out=3 cache_read=2 cache_write=1', 10_000)
    const stdout = capture.stdout()
    expect(stdout).toContain('\x1b[')
    expect(stdout).toContain('session opened')
    expect(stdout).toContain('tool>')
    expect(stdout).toContain('fixture-shell')
    expect(stdout).toContain('thinking>')
    expect(stdout).toContain('fixture plan')
    expect(stdout).toContain('assistant>')
    expect(stdout).toContain('fixture response')

    const exitCode = await capture.closed
    expect(exitCode).toBe(0)
    expect(capture.stderr()).toBe('')
    expect(capture.stdout()).toContain('closed')
  } finally {
    if (!child.killed) child.kill('SIGTERM')
  }
})

async function runReplProcess(
  args: string[],
  options: { env?: NodeJS.ProcessEnv } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number | null }> {
  const child = spawn(process.execPath, ['run', 'packages/repl/src/index.ts', ...args], {
    cwd: process.cwd(),
    env: options.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const capture = new ProcessCapture(child)
  const exitCode = await capture.closed
  return { stdout: capture.stdout(), stderr: capture.stderr(), exitCode }
}

function spawnReplFixture(workspace: string): ReturnType<typeof spawn> {
  return spawn(
    process.execPath,
    [
      'run',
      'packages/repl/src/index.ts',
      '--cwd',
      workspace,
      '--claude-path',
      claudeFixturePath,
      '--model',
      'claude-opus-4-8',
      '--thinking',
      'medium',
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
}

function spawnReplFixtureInPty(workspace: string): ReturnType<typeof spawn> {
  const scriptPath = '/usr/bin/script'
  if (!existsSync(scriptPath)) throw new Error('DEMI_REPL_TTY_E2E requires BSD script at /usr/bin/script')
  const command = [
    "{ printf 'run the fixture workflow\\n'; sleep 2; printf '/exit\\n'; }",
    '|',
    `${scriptPath} -q /dev/null "$DEMI_REPL_EXEC_PATH" run packages/repl/src/index.ts`,
    '--cwd "$DEMI_REPL_WORKSPACE"',
    '--claude-path "$DEMI_REPL_CLAUDE_PATH"',
    '--model claude-opus-4-8',
    '--thinking medium',
    '--yield-after-ms 5',
    '--timeout-ms 5000',
  ].join(' ')
  return spawn(
    'sh',
    ['-c', command],
    {
      cwd: process.cwd(),
      env: {
        ...process.env,
        PATH: `${fixtureDir}${delimiter}${process.env.PATH ?? ''}`,
        DEMI_REPL_CLAUDE_PATH: claudeFixturePath,
        DEMI_REPL_EXEC_PATH: process.execPath,
        DEMI_REPL_WORKSPACE: workspace,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )
}

function extractFirstShellId(stdout: string): string {
  const match = /shell\[([^\]]+)\]/.exec(stdout)
  if (!match) throw new Error(`No shell id found in stdout:\n${stdout}`)
  return match[1]
}
