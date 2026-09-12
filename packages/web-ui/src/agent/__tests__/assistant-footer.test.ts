import { expect, test } from 'bun:test'
import { assistantFooterIds } from '../assistant-footer'

test('tool and subagent continuations leave only the final reply footer', () => {
  expect([...assistantFooterIds([
    { id: 'request', type: 'user' },
    { id: 'delegated', type: 'text' },
    { id: 'check-environment', type: 'tool_call' },
    { id: 'waiting', type: 'text' },
    { id: 'child-result', type: 'agent_message' },
    { id: 'verify', type: 'tool_call' },
    { id: 'final', type: 'text' },
  ], 'idle')]).toEqual(['final'])
})

test('earlier final replies keep footers while the next request runs', () => {
  expect([...assistantFooterIds([
    { id: 'first', type: 'text' },
    { id: 'next-request', type: 'user' },
    { id: 'streaming', type: 'text' },
  ], 'running')]).toEqual(['first'])
})

test('thinking, steer and failed tool continuations do not finalize progress text', () => {
  for (const type of ['thinking', 'steer', 'error', 'abort'] as const) {
    expect(assistantFooterIds([
      { id: 'progress', type: 'text' },
      { id: 'continuation', type },
    ], 'idle').size).toBe(0)
  }
})

test('only the last text in a consecutive sequence gets a footer', () => {
  expect([...assistantFooterIds([
    { id: 'progress', type: 'text' },
    { id: 'final', type: 'text' },
  ], 'idle')]).toEqual(['final'])
})
