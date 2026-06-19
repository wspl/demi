import { expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import {
  buildClaudeArgs,
  buildClaudeArgsForRequest,
  buildClaudeEnv,
  claudeAuthState,
  claudeRuntimeState,
  detectClaudeCli,
  readClaudeStatsigState,
} from '../index'

test('detectClaudeCli reports path and version from shell runner', async () => {
  const status = await detectClaudeCli(async (command, args) => {
    if (command === 'which') return { stdout: '/bin/claude\n', stderr: '', exitCode: 0 }
    if (command === '/bin/claude' && args[0] === '--version') {
      return { stdout: '2.1.170 (Claude Code)\n', stderr: '', exitCode: 0 }
    }
    return { stdout: '', stderr: 'unexpected', exitCode: 1 }
  })

  expect(status).toEqual({ path: '/bin/claude', version: '2.1.170 (Claude Code)' })
})

test('claudeRuntimeState and claudeAuthState are derived without SDK dependencies', async () => {
  expect(
    await claudeRuntimeState(async () => ({ stdout: '', stderr: '', exitCode: 1 })),
  ).toEqual({ status: 'unavailable', message: 'claude CLI is not installed or not on PATH' })

  const auth = await claudeAuthState(async () => ({
    stdout: JSON.stringify({ loggedIn: true, email: 'dev@example.com' }),
    stderr: '',
    exitCode: 0,
  }))
  expect(auth).toEqual({ status: 'authenticated', accountLabel: 'dev@example.com' })
})

test('claudeAuthState falls back to statsig state when auth status is unavailable', async () => {
  const auth = await claudeAuthState(
    async () => ({ stdout: '', stderr: 'not available', exitCode: 1 }),
    async () => ({ exists: true, stableId: 'stable' }),
  )

  expect(auth).toEqual({ status: 'unknown', message: 'not available; Claude statsig state exists' })
})

test('readClaudeStatsigState reads stable and session ids conservatively', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-statsig-'))
  try {
    await writeFile(join(dir, 'statsig.stable_id.123'), 'stable-id\n')
    await writeFile(join(dir, 'statsig.session_id.123'), 'session-id\n')

    expect(await readClaudeStatsigState(dir)).toEqual({
      exists: true,
      stableId: 'stable-id',
      sessionId: 'session-id',
    })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('buildClaudeArgs and env match the planned CLI contract', () => {
  expect(buildClaudeArgs({ modelId: 'claude-test', systemPrompt: 'system', thinkingEffort: 'high' })).toEqual([
    '--print',
    '--output-format',
    'stream-json',
    '--verbose',
    '--input-format',
    'stream-json',
    '--include-partial-messages',
    '--no-session-persistence',
    '--safe-mode',
    '--disable-slash-commands',
    '--tools',
    '',
    '--permission-mode',
    'bypassPermissions',
    '--allow-dangerously-skip-permissions',
    '--model',
    'claude-test',
    '--system-prompt',
    'system',
    '--effort',
    'high',
  ])
  expect(buildClaudeArgs({ modelId: 'claude-test', systemPrompt: 'system', maxBudgetUsd: '0.01' })).toContain('--max-budget-usd')
  expect(buildClaudeArgs({ modelId: 'claude-test', systemPrompt: 'system', maxBudgetUsd: '0.01' }).slice(-2)).toEqual([
    '--max-budget-usd',
    '0.01',
  ])

  const env = buildClaudeEnv({ CLAUDECODE: '1', PATH: '/bin' })
  expect(env.DISABLE_AUTO_COMPACT).toBe('1')
  expect(env.MAX_MCP_OUTPUT_TOKENS).toBe('1000000')
  expect(env.CLAUDECODE).toBeUndefined()
})

test('buildClaudeArgsForRequest maps summary thinking effort and provider budget to CLI args', () => {
  const args = buildClaudeArgsForRequest(
    {
      sessionId: 'test-session',
      turnId: 'test-turn',
      requestId: 'test-request',
      modelId: 'claude-opus-4-8',
      systemPrompt: 'Summarize the previous conversation for continuation.',
      cwd: '/workspace',
      items: [],
      tools: [],
      thinking: { type: 'effort', effort: 'medium', summary: null },
      cancel: new AbortController().signal,
    },
    { maxBudgetUsd: '0.25' },
  )

  expect(args).toContain('--effort')
  expect(args.slice(args.indexOf('--effort'), args.indexOf('--effort') + 2)).toEqual(['--effort', 'medium'])
  expect(args).toContain('--max-budget-usd')
  expect(args.slice(args.indexOf('--max-budget-usd'), args.indexOf('--max-budget-usd') + 2)).toEqual([
    '--max-budget-usd',
    '0.25',
  ])
})
