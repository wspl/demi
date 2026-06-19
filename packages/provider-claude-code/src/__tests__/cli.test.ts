import { expect, test } from 'bun:test'
import {
  buildClaudeArgs,
  buildClaudeArgsForRequest,
  buildClaudeEnv,
} from '../index'

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
