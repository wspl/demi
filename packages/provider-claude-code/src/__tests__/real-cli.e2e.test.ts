import { expect, test } from 'bun:test'
import type { InferenceRequest, ProviderEvent } from '@demi/provider'
import { ClaudeCodeProvider } from '../index'

const e2e = process.env.DEMI_CLAUDE_CODE_E2E === '1' ? test : test.skip

e2e('ClaudeCodeProvider can stream a minimal response from the real claude CLI', async () => {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 60_000)
  const provider = new ClaudeCodeProvider({
    maxBudgetUsd: process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.01',
  })
  const request: InferenceRequest = {
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
