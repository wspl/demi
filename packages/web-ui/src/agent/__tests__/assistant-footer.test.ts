import { expect, test } from 'bun:test'
import type { Block, SessionPhase } from '@demicodes/protocol'
import { assistantFooterIds as footerAmong, replyEndIds } from '../assistant-footer'

// Where a reply ends, after which its work is done: Copy and Fork wait for
// it. Pure.

function assistantFooterIds(blocks: readonly Pick<Block, 'id' | 'type'>[], phase: SessionPhase): Set<string> {
  return footerAmong(blocks, replyEndIds(blocks, phase))
}

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
  for (const type of ['thinking', 'steer', 'error'] as const) {
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

test('a compaction after the final reply leaves its footer', () => {
  expect([...assistantFooterIds([
    { id: 'request', type: 'user' },
    { id: 'final', type: 'text' },
    { id: 'marker', type: 'compaction_marker' },
  ], 'idle')]).toEqual(['final'])
})

test('a reply the user stopped keeps its footer, Copy among it', () => {
  expect([...assistantFooterIds([
    { id: 'request', type: 'user' },
    { id: 'cut-off', type: 'text' },
    { id: 'stopped', type: 'abort' },
  ], 'idle')]).toEqual(['cut-off'])
})
