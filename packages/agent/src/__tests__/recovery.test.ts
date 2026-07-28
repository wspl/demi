import { expect, test } from 'bun:test'
import { StubProvider, events } from '@demicodes/provider/testing'
import { findResumePoint } from '../recovery'
import { createSession, createRuntime, makeTranscript, model, text } from './helpers'

type Leftover = 'text' | 'thinking' | 'toolCall' | 'toolCallPending' | 'response' | 'error' | 'abort'

function afterUserTurn(...parts: Leftover[]) {
  const log = makeTranscript()
  log.pushUserTurn('turn-1', model, text('hello'))
  for (const part of parts) {
    switch (part) {
      case 'text':
        log.applyProviderEvent(model, { type: 'text_delta', text: 'partial answer' })
        break
      case 'thinking':
        log.applyProviderEvent(model, { type: 'thinking_delta', text: 'reasoning' })
        break
      case 'toolCall':
        log.applyProviderEvent(model, { type: 'tool_call_requested', toolUseId: 'tool-1', toolName: 'noop', input: {} })
        log.completeToolCall('tool-1', [{ type: 'text', text: 'done' }])
        break
      case 'toolCallPending':
        log.applyProviderEvent(model, { type: 'tool_call_requested', toolUseId: 'tool-2', toolName: 'noop', input: {} })
        break
      case 'response':
        log.applyProviderEvent(model, events.response())
        break
      case 'error':
        log.applyProviderEvent(model, { type: 'error', message: 'boom', code: 'overloaded' })
        break
      case 'abort':
        log.pushAbort(model)
        break
    }
  }
  return log.blocks
}

test('a turn whose only trace is the failure unwinds completely', () => {
  expect(findResumePoint(afterUserTurn())).toEqual({ cut: 1, isFullRerun: true })
  expect(findResumePoint(afterUserTurn('error'))).toEqual({ cut: 1, isFullRerun: true })
  // Thinking is discardable: nothing keys off it and a rerun simply reasons again.
  expect(findResumePoint(afterUserTurn('thinking', 'error'))).toEqual({ cut: 1, isFullRerun: true })
})

test('emitted text stops the unwind — a product may already have sent it', () => {
  expect(findResumePoint(afterUserTurn('text', 'error'))).toEqual({ cut: 2, isFullRerun: false })
})

test('a tool call stops the unwind whether or not it completed', () => {
  expect(findResumePoint(afterUserTurn('toolCall', 'error'))).toEqual({ cut: 2, isFullRerun: false })
  // Still 'executing' means the process died mid-tool: whether the effect landed
  // is unknown, and unknown must be treated as landed.
  expect(findResumePoint(afterUserTurn('toolCallPending'))).toEqual({ cut: 2, isFullRerun: false })
})

test('a response stops the unwind — it records a request that did complete', () => {
  // Dropping it would also drop the usage that anchors the context estimate.
  expect(findResumePoint(afterUserTurn('toolCall', 'response', 'thinking', 'error'))).toEqual({
    cut: 3,
    isFullRerun: false,
  })
})

test('an abort stops the unwind — it is history the user created', () => {
  expect(findResumePoint(afterUserTurn('abort'))).toEqual({ cut: 2, isFullRerun: false })
})

test('whitespace-only text is a leftover, not output', () => {
  const log = makeTranscript()
  log.pushUserTurn('turn-1', model, text('hello'))
  log.applyProviderEvent(model, { type: 'text_delta', text: '   \n' })
  expect(findResumePoint(log.blocks).isFullRerun).toBe(true)
})

test('only the latest turn is in play — an earlier one is settled history', () => {
  const log = makeTranscript()
  log.pushUserTurn('turn-1', model, text('first'))
  log.applyProviderEvent(model, { type: 'text_delta', text: 'answered' })
  log.applyProviderEvent(model, events.response())
  log.pushUserTurn('turn-2', model, text('second'))
  log.applyProviderEvent(model, { type: 'error', message: 'boom', code: 'overloaded' })
  expect(findResumePoint(log.blocks)).toEqual({ cut: 4, isFullRerun: true })
})

test('an empty transcript keeps everything and continues', () => {
  expect(findResumePoint([])).toEqual({ cut: 0, isFullRerun: false })
})

test('resume reruns a turn that failed before producing anything', async () => {
  const provider = new StubProvider([
    [events.error('backend failed', 'overloaded')],
    [events.text('second attempt'), events.response()],
  ])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: { maxAttempts: 1 } })

  await expect(session.send(text('run'))).rejects.toThrow('backend failed')
  await session.resume()

  // No continuation boundary and no leftover error: the turn simply ran again.
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('resume keeps a tool result and continues after it', async () => {
  let toolCalls = 0
  let resumedItems = ''
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'noop', {}), events.response()],
    [events.error('backend failed', 'overloaded')],
    (request) => {
      resumedItems = JSON.stringify(request.items)
      return [events.text('continued'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'noop',
        description: 'does nothing',
        inputSchema: { type: 'object' },
        invoke: () => {
          toolCalls += 1
          return { output: [{ type: 'text', text: 'preserved result' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime, undefined, undefined, { retry: { maxAttempts: 1 } })

  await expect(session.send(text('run'))).rejects.toThrow('backend failed')
  await session.resume()

  expect(toolCalls).toBe(1)
  expect(resumedItems).toContain('preserved result')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'resume',
    'text',
    'response',
  ])
})

test('resume keeps already-emitted text exactly once', async () => {
  const provider = new StubProvider([
    [events.text('interim finding'), events.error('backend failed', 'overloaded')],
    [events.text('final answer'), events.response()],
  ])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: { maxAttempts: 1 } })

  await expect(session.send(text('run'))).rejects.toThrow('backend failed')
  await session.resume()

  const texts = session
    .transcript()
    .blocks.filter((block): block is Extract<typeof block, { type: 'text' }> => block.type === 'text')
    .map((block) => block.text)
  // Rewinding this would make any product that already sent it send a second copy.
  expect(texts).toEqual(['interim finding', 'final answer'])
})

test('resume drops stale thinking so the rerun does not replay it', async () => {
  const provider = new StubProvider([
    [{ type: 'thinking_start' }, { type: 'thinking_delta', text: 'first pass reasoning' }, events.error('backend failed', 'overloaded')],
    [events.text('answer'), events.response()],
  ])
  const session = createSession(provider, createRuntime(), undefined, undefined, { retry: { maxAttempts: 1 } })

  await expect(session.send(text('run'))).rejects.toThrow('backend failed')
  await session.resume()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})
