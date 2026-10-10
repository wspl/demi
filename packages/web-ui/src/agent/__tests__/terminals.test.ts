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
  promptLine,
  runningTerminals,
  shellRowEnd,
  shellRowRunning,
  terminalPanelTabs,
  terminalStatus,
  terminalWrite,
  type TerminalRecord,
} from '../terminals'
import { commandEndMark } from '../command-end'
import type { ToolCallBlock } from '../block-types'

function terminal(
  partial: Pick<TerminalRecord, 'id' | 'title' | 'phase'> & Partial<TerminalRecord>,
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
      title: 'A',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    terminal({
      id: 'b',
      title: 'B',
      phase: 'exited',
      startedAt: '2026-09-09T00:00:00.000Z',
    }),
    terminal({
      id: 'c',
      title: 'C',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
  ]
  expect(runningTerminals(terminals).map((item) => item.id)).toEqual(['c', 'a'])
  expect(firstRunningTerminalId(terminals)).toBe('c')
})

test('a command\'s tab shows it running, done, or stopped', () => {
  expect(terminalStatus('running')).toBe('active')
  expect(terminalStatus('exited')).toBe('done')
  // The user or the agent stopped it: it did not finish.
  expect(terminalStatus('aborted')).toBe('aborted')
})

test('a running inspect lists every running job; an exited inspect is that job only', () => {
  const terminals = [
    terminal({
      id: 'run-old',
      title: 'Old',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
    terminal({
      id: 'run-new',
      title: 'New',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    terminal({
      id: 'done',
      title: 'Done',
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
    toolName: 'shell',
    input: '{}',
    status,
    output: [],
    view: null,
  }
}

test('a command shows under its running call, and in the dock once the call returned', () => {
  const rootCommand = terminal({ id: 'root', title: 'R', phase: 'running', toolUseId: 'call-1' })
  const childCommand = terminal({ id: 'child', title: 'C', phase: 'running', toolUseId: 'call-1', subagentId: 'agent' })
  const stored = terminal({ id: 'stored', title: 'S', phase: 'exited' })
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
  const again = terminal({ id: 'again', title: 'R2', phase: 'running', toolUseId: 'call-1' })
  expect(callTerminal([rootCommand, again], undefined, 'call-1')?.id).toBe('again')
  expect(
    dockTerminals([again], () => [call('call-1', 'executing'), call('call-1', 'completed')]).map((item) => item.id),
  ).toEqual(['again'])
})

// `runtime.md` § Rendering boundary: a command's terminal opens with its
// script as a terminal shows what was typed, the page's own line.
const COLORS = { prompt: '<p>', script: '<s>' }
const R = '\x1b[0m'

test('the prompt line shows the script after $, and each further line after >', () => {
  expect(promptLine('cd web\nbun test \\\n  --watch\n', COLORS)).toBe(
    `<p>$${R} <s>cd web${R}\n<p>>${R} <s>bun test \\${R}\n<p>>${R} <s>  --watch${R}\n`,
  )
  expect(promptLine(undefined, COLORS)).toBe('')
})

test('the prompt line opens the terminal once, and again only when it starts anew', () => {
  const prompt = promptLine('npm test', COLORS)
  // The first frame opens with the prompt, then the output.
  const first = terminalWrite(null, 'one\n', 4, prompt)
  expect(first).toEqual({ reset: false, text: `${prompt}one\n` })
  // A frame that adds output writes only what it adds: the prompt is not part of the count.
  expect(terminalWrite({ output: 'one\n', chars: 4 }, 'one\ntwo\n', 8, prompt)).toEqual({ reset: false, text: 'two\n' })
  // After a gap the tail shows anew, under the prompt again.
  expect(terminalWrite({ output: 'one\ntwo\n', chars: 8 }, 'ninety\n', 100, prompt))
    .toEqual({ reset: true, text: `${prompt}ninety\n` })
  // Whole output, as after a reload, continues what was shown.
  expect(terminalWrite({ output: 'a\n', chars: undefined }, 'a\nb\n', undefined, prompt)).toEqual({ reset: false, text: 'b\n' })
})

// `runtime.md` § Rendering boundary: a shell row shimmers as long as its
// command runs, also after its call returned, until the command's end arrives.
test('a shell row runs while its call runs, and after it returned until its command ends', () => {
  // The call runs; its command's first frame has not come yet.
  expect(shellRowRunning('executing', undefined)).toBe(true)
  // The call returned and the command runs on as one of the running commands.
  const command = terminal({ id: 'cmd', title: 'Watch', phase: 'running', toolUseId: 'call-1' })
  expect(shellRowRunning('completed', command)).toBe(true)
  // The command's end arrives: exited, or stopped.
  expect(shellRowRunning('completed', { ...command, phase: 'exited' })).toBe(false)
  expect(shellRowRunning('completed', { ...command, phase: 'aborted' })).toBe(false)
  // A call that returned with no command the page knows of is done, failed or not.
  expect(shellRowRunning('completed', undefined)).toBe(false)
  expect(shellRowRunning('error', undefined)).toBe(false)
})

// `runtime.md` § Rendering boundary: every shell row marks how its command
// ended, also a command that ran on after its call returned.
test('a returned call marks its command\'s end once the end arrives, and not from its own running view', () => {
  const returned: ToolCallBlock = {
    type: 'tool_call', id: 'call', createdAt, model, toolUseId: 'call-1', toolName: 'shell',
    input: '{"script":"make watch"}', status: 'completed', output: [],
    view: {
      kind: 'shell', status: 'running', commandId: 'cmd', runningMs: 10, idleMs: 0,
      chunks: [], viewTruncated: false,
    },
  }
  const command = terminal({ id: 'cmd', title: 'Watch', phase: 'running', toolUseId: 'call-1' })
  // It runs on: no end yet, whatever the call stored.
  expect(commandEndMark(shellRowEnd(returned, command))).toBeNull()
  // Its end arrives in a frame: exit 1 is Failed, a stop is Stopped, exit 0 is nothing.
  expect(commandEndMark(shellRowEnd(returned, { ...command, phase: 'exited', exitCode: 1 })))
    .toEqual({ kind: 'failed', exitCode: 1 })
  expect(commandEndMark(shellRowEnd(returned, { ...command, phase: 'aborted' }))).toEqual({ kind: 'stopped' })
  expect(commandEndMark(shellRowEnd(returned, { ...command, phase: 'exited', exitCode: 0 }))).toBeNull()
  // A call that returned with its command ended keeps the end it stored.
  const ended = { ...returned, view: { ...returned.view!, status: 'exited' as const, exitCode: 2 } }
  expect(commandEndMark(shellRowEnd(ended, undefined))).toEqual({ kind: 'failed', exitCode: 2 })
  // While the call runs there is no end.
  expect(shellRowEnd({ ...ended, status: 'executing' }, undefined)).toBeNull()
})
