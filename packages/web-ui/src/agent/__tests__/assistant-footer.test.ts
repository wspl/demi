import { expect, test } from 'bun:test'
import type { Block, SessionPhase } from '@demicodes/protocol'
import type { TranscriptRequest } from '../../files/request-changes'
import { assistantFooterIds as footerAmong, replyEndIds, requestLineIds } from '../assistant-footer'

// Where a reply ends, after which its work is done: Copy and Fork, and the
// request's Files Changed line, wait for it (`edit-tracking.md` § What the
// conversation shows). Pure.

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

/** A request that changed one file, whose rows are `blocks` after its user block. */
function changedRequest(blocks: readonly Pick<Block, 'id' | 'type'>[]) {
  const request: TranscriptRequest = { id: 'request', files: [{ path: 'src/app.ts', kind: 'modified', edits: [] }] }
  const rows = [{ id: 'request', type: 'user' as const }, ...blocks]
  const requestOf = new Map(rows.map((row) => [row.id, request]))
  /** The row the line goes under, by id; none while it is hidden. */
  const line = (phase: SessionPhase) => [...requestLineIds(rows, requestOf, replyEndIds(rows, phase)).keys()]
  return { line }
}

test('a request’s Files Changed line waits for the request to end, as its Copy and Fork do', () => {
  const working = changedRequest([
    { id: 'edit', type: 'tool_call' },
    { id: 'reply', type: 'text' },
  ])
  expect(working.line('running')).toEqual([])
  expect(working.line('idle')).toEqual(['reply'])
})

test('a request a later turn continues has no line until that turn ends', () => {
  // The first turn's reply waited for a child; the child's result woke the request.
  const continued = changedRequest([
    { id: 'edit', type: 'tool_call' },
    { id: 'waiting', type: 'text' },
    { id: 'wakeup', type: 'agent_message' },
    { id: 'final', type: 'text' },
  ])
  expect(continued.line('running')).toEqual([])
  expect(continued.line('idle')).toEqual(['final'])
})

test('a request stopped during a call has its line under the stop', () => {
  const stopped = changedRequest([
    { id: 'edit', type: 'tool_call' },
    { id: 'stopped', type: 'abort' },
  ])
  expect(stopped.line('idle')).toEqual(['stopped'])
})

test('an earlier request keeps its line while the next one runs', () => {
  const first: TranscriptRequest = { id: 'first', files: [{ path: 'a.ts', kind: 'added', edits: [] }] }
  const second: TranscriptRequest = { id: 'second', files: [{ path: 'b.ts', kind: 'added', edits: [] }] }
  const rows = [
    { id: 'first', type: 'user' as const },
    { id: 'first-reply', type: 'text' as const },
    { id: 'second', type: 'user' as const },
    { id: 'second-edit', type: 'tool_call' as const },
  ]
  const requestOf = new Map<string, TranscriptRequest>([
    ['first', first], ['first-reply', first], ['second', second], ['second-edit', second],
  ])
  expect([...requestLineIds(rows, requestOf, replyEndIds(rows, 'running')).keys()]).toEqual(['first-reply'])
})
