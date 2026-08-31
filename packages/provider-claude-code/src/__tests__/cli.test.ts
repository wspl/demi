import { expect, test } from 'bun:test'
import { buildClaudeArgs, buildClaudeEnv } from '../cli'
import { buildClaudeArgsForRequest } from '../transport'

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

  const env = buildClaudeEnv({ CLAUDECODE: '1', PATH: '/bin' })
  expect(env.DISABLE_AUTO_COMPACT).toBe('1')
  expect(env.MAX_MCP_OUTPUT_TOKENS).toBe('1000000')
  expect(env.CLAUDECODE).toBeUndefined()
})

test('buildClaudeEnv public overlay is applied last and wins over the OAuth token', () => {
  const env = buildClaudeEnv(
    { PATH: '/bin' },
    {
      oauthAccessToken: 'vault-token',
      overlay: {
        ANTHROPIC_BASE_URL: 'http://backend/passthrough',
        CLAUDE_CODE_OAUTH_TOKEN: 'runner-token',
        MAX_MCP_OUTPUT_TOKENS: '5',
      },
    },
  )
  expect(env.ANTHROPIC_BASE_URL).toBe('http://backend/passthrough')
  expect(env.CLAUDE_CODE_OAUTH_TOKEN).toBe('runner-token')
  expect(env.MAX_MCP_OUTPUT_TOKENS).toBe('5')
  expect(env.PATH).toBe('/bin')
})

test('buildClaudeArgsForRequest maps summary thinking effort to CLI args', () => {
  const args = buildClaudeArgsForRequest({
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
  })

  expect(args).toContain('--effort')
  expect(args.slice(args.indexOf('--effort'), args.indexOf('--effort') + 2)).toEqual(['--effort', 'medium'])
})
