import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import type { TokenUsage } from '@demi/core'
import type { InferenceRequest, ProviderEvent } from '@demi/provider'
import { ClaudeCodeProvider } from '../index'

const e2e = process.env.DEMI_CLAUDE_CODE_E2E === '1' ? test : test.skip
const cacheE2e = process.env.DEMI_CLAUDE_CODE_CACHE_E2E === '1' ? test : test.skip
const thinkingE2e = process.env.DEMI_CLAUDE_CODE_THINKING_E2E === '1' ? test : test.skip
const thinkingAttempts = Math.max(1, Number.parseInt(process.env.DEMI_CLAUDE_CODE_THINKING_E2E_ATTEMPTS ?? '2', 10))

e2e('ClaudeCodeProvider can stream a minimal response from the real claude CLI', async () => {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 60_000)
  const provider = new ClaudeCodeProvider({
    maxBudgetUsd: process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.01',
  })
  const request: InferenceRequest = {
    sessionId: 'claude-real-e2e-session',
    turnId: 'claude-real-e2e-turn',
    requestId: 'claude-real-e2e-request',
    modelId: process.env.DEMI_CLAUDE_CODE_MODEL ?? 'sonnet',
    systemPrompt: 'Reply tersely and follow exact-output requests.',
    cwd: process.cwd(),
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'Reply with exactly OK.' }] }],
    tools: [],
    thinking: null,
    cancel: controller.signal,
  }
  const events: ProviderEvent[] = []

  try {
    for await (const event of provider.run(request)) events.push(event)
  } finally {
    clearTimeout(timeout)
  }

  const errors = events.filter((event) => event.type === 'error')
  expect(errors).toEqual([])
  expect(events.some((event) => event.type === 'text_delta' && event.text.includes('OK'))).toBe(true)
  expect(events.some((event) => event.type === 'response')).toBe(true)
})

cacheE2e('ClaudeCodeProvider reports a real provider cache hit on repeated tool-enabled requests', async () => {
  const cacheKey = `demi-cache-e2e-${randomUUID()}`
  const systemPrompt = [
    `Cache smoke key: ${cacheKey}`,
    'You are a coding agent. '.repeat(200),
    'When asked for the cache smoke response, output the marker exactly.',
  ].join('\n')
  const first = await runCacheRequest(systemPrompt)
  const second = await runCacheRequest(systemPrompt)

  expect(first.cacheWriteTokens + first.cacheReadTokens).toBeGreaterThan(0)
  expect(second.cacheReadTokens).toBeGreaterThan(0)
})

thinkingE2e(
  'ClaudeCodeProvider streams real medium thinking for a budgeted summary request on opus',
  async () => {
    const runs: ProviderEvent[][] = []
    for (let attempt = 0; attempt < thinkingAttempts; attempt++) runs.push(await runThinkingBudgetRequest())

    expect(runs.some((events) => events.some(isVisibleThinkingEvent))).toBe(true)
  },
  thinkingAttempts * 120_000,
)

async function runCacheRequest(systemPrompt: string): Promise<TokenUsage> {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 90_000)
  const provider = new ClaudeCodeProvider({
    maxBudgetUsd: process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.15',
  })
  const request: InferenceRequest = {
    sessionId: 'claude-cache-e2e-session',
    turnId: 'claude-cache-e2e-turn',
    requestId: `claude-cache-e2e-${randomUUID()}`,
    modelId: process.env.DEMI_CLAUDE_CODE_MODEL ?? 'claude-opus-4-8',
    systemPrompt,
    cwd: process.cwd(),
    items: [
      {
        type: 'user_message',
        content: [{ type: 'text', text: 'Output DEMI_CACHE_TOOL_OK exactly. Do not use tools.' }],
      },
    ],
    tools: [
      {
        name: 'shell_exec',
        description: 'Execute shell commands',
        inputSchema: {
          type: 'object',
          properties: { script: { type: 'string' } },
          required: ['script'],
        },
      },
    ],
    thinking: { type: 'effort', effort: 'medium', summary: null },
    cancel: controller.signal,
  }
  const events: ProviderEvent[] = []

  try {
    for await (const event of provider.run(request)) events.push(event)
  } finally {
    clearTimeout(timeout)
  }

  const errors = events.filter((event) => event.type === 'error')
  expect(errors).toEqual([])
  expect(events.filter((event) => event.type === 'tool_call_requested')).toEqual([])
  const text = events.filter((event): event is Extract<ProviderEvent, { type: 'text_delta' }> => event.type === 'text_delta').map((event) => event.text).join('')
  expect(text).toContain('DEMI_CACHE_TOOL_OK')
  const response = events.find((event): event is Extract<ProviderEvent, { type: 'response' }> => event.type === 'response')
  expect(response).toBeDefined()
  return response!.usage
}

async function runThinkingBudgetRequest(): Promise<ProviderEvent[]> {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 90_000)
  const provider = new ClaudeCodeProvider({
    maxBudgetUsd: process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.25',
  })
  const request: InferenceRequest = {
    sessionId: 'claude-thinking-e2e-session',
    turnId: 'claude-thinking-e2e-turn',
    requestId: `claude-thinking-e2e-${randomUUID()}`,
    modelId: process.env.DEMI_CLAUDE_CODE_MODEL ?? 'claude-opus-4-8',
    systemPrompt: [
      'Summarize the previous conversation for continuation.',
      'For this smoke test, use the requested thinking effort and then output DEMI_THINKING_BUDGET_OK exactly.',
    ].join('\n'),
    cwd: process.cwd(),
    items: [
      {
        type: 'user_message',
        content: [
          {
            type: 'text',
            text: [
              'Conversation excerpt:',
              'The user asked the agent to initialize a Vue project, add Pinia, implement a todo list, run tests, and explain failures.',
              'The shell output included several transient prompts and a final successful test run.',
              'Return the continuation marker exactly.',
            ].join('\n'),
          },
        ],
      },
    ],
    tools: [],
    thinking: { type: 'effort', effort: 'medium', summary: null },
    cancel: controller.signal,
  }
  const events: ProviderEvent[] = []

  try {
    for await (const event of provider.run(request)) events.push(event)
  } finally {
    clearTimeout(timeout)
  }

  const errors = events.filter((event) => event.type === 'error')
  expect(errors).toEqual([])
  expect(events.filter((event) => event.type === 'tool_call_requested')).toEqual([])
  const text = events.filter((event): event is Extract<ProviderEvent, { type: 'text_delta' }> => event.type === 'text_delta').map((event) => event.text).join('')
  expect(text).toContain('DEMI_THINKING_BUDGET_OK')
  const response = events.find((event): event is Extract<ProviderEvent, { type: 'response' }> => event.type === 'response')
  expect(response).toBeDefined()
  expect(response!.usage.inputTokens).toBeGreaterThan(0)
  expect(response!.usage.outputTokens).toBeGreaterThan(0)
  return events
}

function isVisibleThinkingEvent(event: ProviderEvent): boolean {
  return (
    (event.type === 'thinking_delta' && event.text.trim().length > 0)
    || (event.type === 'thinking_signature' && event.signature.length > 0)
    || (event.type === 'redacted_thinking' && event.data.length > 0)
  )
}
