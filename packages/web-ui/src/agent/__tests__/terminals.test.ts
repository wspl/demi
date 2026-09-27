import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import { createdAt, model } from './agent-harness'
import {
  LIVE_OUTPUT_CHARS,
  callTerminal,
  dockTerminals,
  firstRunningTerminalId,
  followLiveOutput,
  liveOutputDelta,
  runningTerminals,
  terminalPanelTabs,
  terminalStatus,
  type TerminalRecord,
} from '../terminals'

function terminal(
  partial: Pick<TerminalRecord, 'id' | 'name' | 'phase'> & Partial<TerminalRecord>,
): TerminalRecord {
  return {
    startedAt: '2026-09-09T00:00:00.000Z',
    output: 'a\nb\nc\nd',
    ...partial,
  }
}

test('running terminals are oldest first', () => {
  const terminals = [
    terminal({
      id: 'a',
      name: 'A',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    terminal({
      id: 'b',
      name: 'B',
      phase: 'exited',
      startedAt: '2026-09-09T00:00:00.000Z',
    }),
    terminal({
      id: 'c',
      name: 'C',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
  ]
  expect(runningTerminals(terminals).map((item) => item.id)).toEqual(['c', 'a'])
  expect(firstRunningTerminalId(terminals)).toBe('c')
})

test('status follows the job phase', () => {
  expect(terminalStatus('running')).toBe('active')
  expect(terminalStatus('exited')).toBe('done')
})

test('a running inspect lists every running job; an exited inspect is that job only', () => {
  const terminals = [
    terminal({
      id: 'run-old',
      name: 'Old',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
    terminal({
      id: 'run-new',
      name: 'New',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    terminal({
      id: 'done',
      name: 'Done',
      phase: 'exited',
    }),
  ]
  expect(terminalPanelTabs(terminals, null)).toEqual([])
  expect(terminalPanelTabs(terminals, 'missing')).toEqual([])
  expect(terminalPanelTabs(terminals, 'run-new').map((item) => item.id)).toEqual([
    'run-old',
    'run-new',
  ])
  expect(terminalPanelTabs(terminals, 'done').map((item) => item.id)).toEqual([
    'done',
  ])
})

// `runtime.md` § Rendering boundary: a page adds only the characters beyond
// those it has shown, by the live view's count, and shows the tail anew
// after a gap; characters are code points, as the backend counts them.
test('a live view adds the characters beyond those shown, or its tail anew', () => {
  const cases: [shown: number | undefined, tail: string, chars: number, anew: boolean, text: string][] = [
    [undefined, 'one\ntwo\n', 8, true, 'one\ntwo\n'],
    [4, 'one\ntwo\n', 8, false, 'two\n'],
    [8, 'one\ntwo\n', 8, false, ''],
    // More came than the tail holds: a gap.
    [2, 'two\n', 8, true, 'two\n'],
    // A count behind the one shown is no continuation.
    [9, 'one\ntwo\n', 8, true, 'one\ntwo\n'],
    // An astral character is one: the new "😀!" is two characters and three UTF-16 units.
    [3, 'a😀b😀!', 5, false, '😀!'],
  ]
  for (const [shown, tail, chars, anew, text] of cases) {
    expect(liveOutputDelta(shown, tail, chars)).toEqual({ anew, text })
  }
  expect(followLiveOutput('one\n', 4, 'one\ntwo\n', 8)).toBe('one\ntwo\n')
  expect(followLiveOutput('old\n', 4, 'new\n', 100)).toBe('new\n')
  const long = followLiveOutput('x'.repeat(LIVE_OUTPUT_CHARS), LIVE_OUTPUT_CHARS, 'y', LIVE_OUTPUT_CHARS + 1)
  expect(long.length).toBe(LIVE_OUTPUT_CHARS)
  expect(long.endsWith('xy')).toBe(true)
})

function call(toolUseId: string, status: 'executing' | 'completed'): Block {
  return {
    type: 'tool_call',
    id: `block-${toolUseId}-${status}`,
    createdAt,
    model,
    toolUseId,
    toolName: 'shell_exec',
    input: '{}',
    status,
    output: [],
    view: null,
  }
}

test('a command shows under its running call, and in the dock once the call returned', () => {
  const rootCommand = terminal({ id: 'root', name: 'R', phase: 'running', toolUseId: 'call-1' })
  const childCommand = terminal({ id: 'child', name: 'C', phase: 'running', toolUseId: 'call-1', subagentId: 'agent' })
  const stored = terminal({ id: 'stored', name: 'S', phase: 'exited' })
  const terminals = [stored, rootCommand, childCommand]
  const root = [call('call-1', 'executing')]
  const child = [call('call-1', 'completed')]
  const blocksOf = (subagentId: string | undefined) => (subagentId === 'agent' ? child : root)
  expect(callTerminal(terminals, undefined, 'call-1')?.id).toBe('root')
  expect(callTerminal(terminals, 'agent', 'call-1')?.id).toBe('child')
  expect(callTerminal(terminals, 'other', 'call-1')).toBeUndefined()
  // The root's call still runs; the child's returned, and its command is the dock's.
  expect(dockTerminals(terminals, blocksOf).map((item) => item.id)).toEqual(['stored', 'child'])
  // A model that reuses a call's id: the latest call and the latest command are the ones that run.
  const again = terminal({ id: 'again', name: 'R2', phase: 'running', toolUseId: 'call-1' })
  expect(callTerminal([rootCommand, again], undefined, 'call-1')?.id).toBe('again')
  expect(
    dockTerminals([again], () => [call('call-1', 'executing'), call('call-1', 'completed')]).map((item) => item.id),
  ).toEqual(['again'])
})
