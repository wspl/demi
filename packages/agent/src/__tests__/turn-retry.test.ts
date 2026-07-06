import { expect, test } from 'bun:test'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { SessionEvent } from '../types'
import { DEFAULT_TURN_RETRY_POLICY, isRetryableCode, retryDelayMs, resolveRetryPolicy } from '../retry-policy'
import { createSession, createRuntime, text } from './helpers'

const fastRetry = { baseDelayMs: 1, maxDelayMs: 2 }

test('transient provider errors before any content are retried silently', async () => {
  const provider = new StubProvider([
    [events.error('throttled', 'rate_limit')],
    [events.error('busy', 'overloaded')],
    [events.text('recovered'), events.response()],
  ])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: fastRetry })
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await session.send(text('hello'))

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(session.transcript().blocks.some((block) => block.type === 'error')).toBe(false)
  const retries = emitted.filter((event) => event.type === 'retry_scheduled')
  expect(retries).toHaveLength(2)
  expect(retries[0]).toMatchObject({ attempt: 1, code: 'rate_limit' })
  expect(retries[1]).toMatchObject({ attempt: 2, code: 'overloaded' })
  expect(provider.consumedTurns).toBe(3)
})

test('non-retryable error codes surface immediately without retry', async () => {
  const provider = new StubProvider([[events.error('bad key', 'auth_expired')]])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: fastRetry })
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await expect(session.send(text('hello'))).rejects.toThrow('bad key')

  expect(emitted.some((event) => event.type === 'retry_scheduled')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'error')).toBe(true)
  expect(provider.consumedTurns).toBe(1)
})

test('errors after streamed content are not retried (no duplicate output)', async () => {
  const provider = new StubProvider([[events.text('partial'), events.error('dropped', 'rate_limit')]])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: fastRetry })
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await expect(session.send(text('hello'))).rejects.toThrow('dropped')

  expect(emitted.some((event) => event.type === 'retry_scheduled')).toBe(false)
  const textBlocks = session.transcript().blocks.filter((block) => block.type === 'text')
  expect(textBlocks).toHaveLength(1)
  expect(provider.consumedTurns).toBe(1)
})

test('retries stop at maxAttempts and the final error surfaces', async () => {
  const provider = new StubProvider([
    [events.error('throttled 1', 'rate_limit')],
    [events.error('throttled 2', 'rate_limit')],
    [events.error('throttled 3', 'rate_limit')],
  ])
  const session = createSession(provider, createRuntime(), undefined, undefined, {
    retry: { ...fastRetry, maxAttempts: 3 },
  })
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await expect(session.send(text('hello'))).rejects.toThrow('throttled 3')

  expect(emitted.filter((event) => event.type === 'retry_scheduled')).toHaveLength(2)
  expect(provider.consumedTurns).toBe(3)
})

test('retry policy helpers honor Retry-After and cap backoff', () => {
  const policy = resolveRetryPolicy({ baseDelayMs: 100, maxDelayMs: 500 })
  expect(policy.maxAttempts).toBe(DEFAULT_TURN_RETRY_POLICY.maxAttempts)
  expect(isRetryableCode(policy, 'rate_limit')).toBe(true)
  expect(isRetryableCode(policy, null)).toBe(false)
  expect(isRetryableCode(policy, 'auth_expired')).toBe(false)
  // Retry-After wins but is capped at maxDelayMs.
  expect(retryDelayMs(policy, 1, 60_000)).toBe(500)
  expect(retryDelayMs(policy, 1, 200)).toBe(200)
  // Backoff with full jitter stays under the per-attempt ceiling.
  for (let attempt = 1; attempt <= 6; attempt += 1) {
    const delayMs = retryDelayMs(policy, attempt, null)
    expect(delayMs).toBeGreaterThanOrEqual(0)
    expect(delayMs).toBeLessThanOrEqual(Math.min(500, 100 * 2 ** (attempt - 1)))
  }
})

test('tool-call turns retry transient continuation failures', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'noop', {}), events.response()],
    [events.error('throttled', 'rate_limit')],
    [events.text('done'), events.response()],
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'noop',
        description: 'does nothing',
        inputSchema: { type: 'object' },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, runtime, undefined, undefined, { retry: fastRetry })

  await session.send(text('run'))

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'text',
    'response',
  ])
  expect(provider.consumedTurns).toBe(3)
})
