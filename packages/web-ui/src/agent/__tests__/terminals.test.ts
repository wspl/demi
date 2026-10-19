import { expect, test } from 'bun:test'
import { computed, reactive } from 'vue'
import type { HeldTranscript } from '@demicodes/conversation-client'
import type { Block } from '@demicodes/protocol'
import { createdAt, model } from './agent-harness'
import {
  LIVE_OUTPUT_CHARS,
  firstRunningTerminalId,
  followFrame,
  followLiveOutput,
  liveOutputDelta,
  promptLine,
  runningTerminals,
  shellRowEnd,
  shellRowRunning,
  shellRowTime,
  terminalPanelTabs,
  terminalStatus,
  terminalWrite,
  useCallTerminals,
  useDockTerminals,
  type TerminalRecord,
} from '../terminals'
import { commandEndMark, runningTime } from '../command-end'
import type { ShellToolView, ToolCallBlock } from '../block-types'

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

// `rendering.md` § Rendering boundary: a page adds only the characters beyond
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
    endedAt: status === 'executing' ? null : createdAt, output: [],
    view: null,
  }
}

/** A transcript the page holds whole, its blocks one window. */
function held(blocks: Block[]): HeldTranscript {
  return { length: blocks.length, windows: [{ start: 0, blocks }] }
}

test('a command shows under its running call, and in the dock once the call returned', () => {
  const rootCommand = terminal({ id: 'root', title: 'R', phase: 'running', toolUseId: 'call-1' })
  const childCommand = terminal({ id: 'child', title: 'C', phase: 'running', toolUseId: 'call-1', subagentId: 'agent' })
  const stored = terminal({ id: 'stored', title: 'S', phase: 'exited' })
  const orphan = terminal({ id: 'orphan', title: 'O', phase: 'exited', toolUseId: 'call-1', subagentId: 'gone' })
  const state = reactive({
    terminals: [stored, rootCommand, childCommand, orphan],
    history: held([call('call-1', 'executing')]),
    subagents: [{ id: 'agent', history: held([call('call-1', 'completed')]) }],
  })
  const commandOf = useCallTerminals(() => state.terminals)
  const dock = useDockTerminals(() => state)
  expect(commandOf(undefined, 'call-1')?.id).toBe('root')
  expect(commandOf('agent', 'call-1')?.id).toBe('child')
  expect(commandOf('other', 'call-1')).toBeUndefined()
  // The root's call still runs; the child's returned, and its command is the
  // dock's, as are a command no call is known of and one of a child no
  // longer listed.
  expect(dock.value.map((item) => item.id)).toEqual(['stored', 'child', 'orphan'])
  // Once the root's call returns, its command joins the dock.
  state.history = held([call('call-1', 'completed')])
  expect(dock.value.map((item) => item.id)).toEqual(['stored', 'root', 'child', 'orphan'])
  // A model that reuses a call's id: the latest call and the latest command are the ones that run.
  const again = terminal({ id: 'again', title: 'R2', phase: 'running', toolUseId: 'call-1' })
  state.terminals = [rootCommand, again]
  expect(commandOf(undefined, 'call-1')?.id).toBe('again')
  state.history = held([call('call-1', 'completed'), call('call-1', 'executing')])
  expect(dock.value.map((item) => item.id)).toEqual([])
  state.history = held([call('call-1', 'executing'), call('call-1', 'completed')])
  expect(dock.value.map((item) => item.id)).toEqual(['root', 'again'])
})

// The dock and the rows keep what they found between changes; each change
// the store makes must reach them, never leave them showing what was.
test('the dock and the rows follow every change to a child, its transcript and the commands', () => {
  const first = terminal({ id: 'first', title: 'F', phase: 'running', toolUseId: 'call-1', subagentId: 'agent' })
  const second = terminal({ id: 'second', title: 'S', phase: 'running', toolUseId: 'call-1', subagentId: 'agent' })
  const state = reactive({
    terminals: [first, second],
    history: held([]),
    subagents: [{ id: 'agent', history: held([call('call-1', 'executing')]) }],
  })
  const commandOf = useCallTerminals(() => state.terminals)
  const dock = useDockTerminals(() => state)
  const row = computed(() => commandOf('agent', 'call-1')?.id)
  expect(dock.value.map((item) => item.id)).toEqual([])
  expect(row.value).toBe('second')
  // The child's transcript replaced whole: its call returned.
  state.subagents[0]!.history = held([call('call-1', 'completed')])
  expect(dock.value.map((item) => item.id)).toEqual(['first', 'second'])
  // The child's record replaced by one whose call runs again.
  state.subagents = [{ id: 'agent', history: held([call('call-1', 'executing')]) }]
  expect(dock.value.map((item) => item.id)).toEqual([])
  // The child's record removed: its commands are the dock's.
  state.subagents = []
  expect(dock.value.map((item) => item.id)).toEqual(['first', 'second'])
  state.subagents = [{ id: 'agent', history: held([call('call-1', 'executing')]) }]
  expect(dock.value.map((item) => item.id)).toEqual([])
  // A command's call id changed: the row finds the other one, and the
  // command, no longer under a running call, joins the dock.
  state.terminals[1]!.toolUseId = 'call-2'
  expect(row.value).toBe('first')
  expect(commandOf('agent', 'call-2')?.id).toBe('second')
  expect(dock.value.map((item) => item.id)).toEqual(['second'])
  // The latest command of a call removed: the row falls back to the one
  // before it, and to none once that goes too.
  state.terminals[1]!.toolUseId = 'call-1'
  expect(row.value).toBe('second')
  state.terminals.pop()
  expect(row.value).toBe('first')
  state.terminals.pop()
  expect(row.value).toBeUndefined()
})

/** `items`, counting each element a reader reads in `reads[label]`, under the store's reactive proxy. */
function counted<T>(items: T[], reads: Record<string, number>, label: string): T[] {
  reads[label] = 0
  return new Proxy(items, {
    get(target, key, receiver) {
      if (typeof key === 'string' && /^\d+$/.test(key)) {
        reads[label]! += 1
      }
      return Reflect.get(target, key, receiver)
    },
  })
}

// The trace of a long conversation (hundreds of commands, ~200 children)
// spent seconds rescanning every transcript once per command on each
// streamed patch. The dock and the rows read each command and each block a
// bounded number of times, and a patch to one transcript rescans only it.
// Costs a few milliseconds.
test('the dock and the rows read each command and block a bounded number of times, however many there are', () => {
  const agents = ['root', 'a1', 'a2', 'a3']
  const blocksPerAgent = 300
  const reads: Record<string, number> = {}
  // Every block is a call that started a command; every 50th still runs.
  const blocksOf = (agent: string) => Array.from({ length: blocksPerAgent }, (_, index) =>
    call(`${agent}-${index}`, index % 50 === 0 ? 'executing' : 'completed'))
  const commands = agents.flatMap((agent) => Array.from({ length: blocksPerAgent }, (_, index) => terminal({
    id: `${agent}-${index}`,
    title: 'T',
    phase: 'running',
    toolUseId: `${agent}-${index}`,
    ...(agent === 'root' ? {} : { subagentId: agent }),
  })))
  const state = reactive({
    terminals: counted(commands, reads, 'terminals'),
    history: held(counted(blocksOf('root'), reads, 'root')),
    subagents: agents.slice(1).map((agent) => ({ id: agent, history: held(counted(blocksOf(agent), reads, agent)) })),
  })
  const dock = useDockTerminals(() => state)
  expect(dock.value.length).toBe(commands.length - agents.length * (blocksPerAgent / 50))
  // Each block once, each command a bounded number of times.
  for (const agent of agents) {
    expect(reads[agent]).toBeLessThanOrEqual(blocksPerAgent)
  }
  expect(reads.terminals).toBeLessThanOrEqual(2 * commands.length)

  // A streamed patch to the root's transcript rescans the root's blocks only.
  for (const label of Object.keys(reads)) {
    reads[label] = 0
  }
  state.history = held(counted([...blocksOf('root'), call('root-new', 'executing')], reads, 'root'))
  expect(dock.value.length).toBe(commands.length - agents.length * (blocksPerAgent / 50))
  expect(reads.root).toBeLessThanOrEqual(blocksPerAgent + 1)
  for (const agent of agents.slice(1)) {
    expect(reads[agent]).toBe(0)
  }

  // Every row looks up its command: the list is read once for all of them,
  // and a frame that changes one command's output re-reads it for none.
  reads.terminals = 0
  const commandOf = useCallTerminals(() => state.terminals)
  const rows = commands.map((command) => computed(() => commandOf(command.subagentId, command.toolUseId!)?.phase))
  expect(rows.every((row) => row.value === 'running')).toBe(true)
  expect(reads.terminals).toBeLessThanOrEqual(2 * commands.length)
  reads.terminals = 0
  Object.assign(state.terminals[0]!, { phase: 'exited', output: 'done' })
  expect(rows[0]!.value).toBe('exited')
  expect(rows.slice(1).every((row) => row.value === 'running')).toBe(true)
  expect(reads.terminals).toBeLessThanOrEqual(1)
})

// `rendering.md` § Rendering boundary: a command's terminal opens with its
// script as a terminal shows what was typed, the page's own line.
const COLORS = { prompt: '<p>', script: '<s>' }
const R = '\x1b[0m'

test('the prompt line shows the script after $, and each further line after >', () => {
  expect(promptLine('cd web\nbun test \\\n  --watch\n', COLORS)).toBe(
    `<p>$${R} <s>cd web${R}\n<p>>${R} <s>bun test \\${R}\n<p>>${R} <s>  --watch${R}\n`,
  )
  expect(promptLine(undefined, COLORS)).toBe('')
})

test('the script is drawn in its highlighted runs, the prompt staying muted, each line cut where its runs start and end', () => {
  // `cat <<EOF` then `$HOME`: a run per word, one spanning the line break.
  const script = 'cat <<EOF\n$HOME\nEOF'
  const runs = [
    { from: 0, to: 3, ansi: '<cmd>' },
    { from: 4, to: 15, ansi: '<doc>' },
  ]
  expect(promptLine(script, COLORS, runs)).toBe(
    `<p>$${R} ${R}<cmd>cat${R}<s> ${R}<doc><<EOF${R}${R}\n`
      + `<p>>${R} ${R}<doc>$HOME${R}${R}\n`
      + `<p>>${R} <s>EOF${R}\n`,
  )
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

// `rendering.md` § Rendering boundary: a shell row shimmers as long as its
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

// `rendering.md` § Rendering boundary: every shell row marks how its command
// ended, also a command that ran on after its call returned.
test('a returned call marks its command\'s end once the end arrives, and not from its own running view', () => {
  const returned: ToolCallBlock = {
    type: 'tool_call', id: 'call', createdAt, model, toolUseId: 'call-1', toolName: 'shell',
    input: '{"script":"make watch"}', status: 'completed', endedAt: createdAt, output: [],
    view: {
      kind: 'shell', status: 'running', commandId: 'cmd', runningMs: 10, idleMs: 0,
      chunks: [], viewTruncated: false,
    },
  }
  const command = terminal({ id: 'cmd', title: 'Watch', phase: 'running', toolUseId: 'call-1' })
  // It runs on: no end yet, whatever the call stored.
  expect(commandEndMark(shellRowEnd(returned, command))).toBeNull()
  // A stop waits for its runner's word: the row says so, also while the call watches.
  expect(commandEndMark(shellRowEnd(returned, { ...command, stopping: true }))).toEqual({ kind: 'stopping' })
  expect(commandEndMark(shellRowEnd({ ...returned, status: 'executing' }, { ...command, stopping: true })))
    .toEqual({ kind: 'stopping' })
  // A start that waits for its offline Host says which, also while the call watches.
  expect(commandEndMark(shellRowEnd({ ...returned, status: 'executing' }, { ...command, waitingFor: 'MacBook Pro' })))
    .toEqual({ kind: 'waiting', host: 'MacBook Pro' })
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

// `rendering.md` § Rendering boundary and § Live output: a page that attaches
// after a reload receives each live command's view in its handshake, and
// its row times the command from the start the server reports, not from
// when the page loaded; the tag goes when the command's end arrives.
test('a reloaded page times a running command from the start the server reported, and its tag goes at the end', () => {
  const call: ToolCallBlock = {
    type: 'tool_call', id: 'call', createdAt, model, toolUseId: 'call-1', toolName: 'shell',
    input: '{"script":"sleep 600","description":"Wait"}', status: 'executing', endedAt: null, output: [], view: null,
  }
  const loadedAt = Date.parse('2026-09-24T12:05:00.000Z')
  const frame = { status: 'running', commandId: '17', toolUseId: 'call-1', title: 'Wait', script: 'sleep 600', tail: '', chars: 0, runningMs: 92_000 } as const
  const command = followFrame(undefined, frame, undefined, loadedAt)
  const time = shellRowTime(call, command)
  expect(time).toEqual({ kind: 'running', since: loadedAt - 92_000 })
  // The page's clock reads the time it has run since that start.
  expect(runningTime(loadedAt - (time?.kind === 'running' ? time.since : 0))).toBe('1m 32s')
  // A later frame keeps the start; the end frame says how long it ran in all.
  const later = followFrame(command, { ...frame, runningMs: 95_000 }, undefined, loadedAt + 3_500)
  expect(later.startedAt).toBe(command.startedAt)
  const ended = followFrame(later, { ...frame, status: 'exited', exitCode: 0, runningMs: 100_000 }, undefined, loadedAt + 8_200)
  expect(shellRowTime({ ...call, status: 'completed' }, ended)).toEqual({ kind: 'ran', ms: 100_000 })
  // A call that returned with its command ended, after a reload with no frame, times it by its stored view.
  const view: ShellToolView = { kind: 'shell', status: 'exited', exitCode: 1, commandId: '17', runningMs: 47_000, idleMs: 0, chunks: [], viewTruncated: false }
  expect(shellRowTime({ ...call, status: 'completed', view }, undefined)).toEqual({ kind: 'ran', ms: 47_000 })
  // One whose stored view saw it still running, and whose end the page never saw, has no time to tell.
  expect(shellRowTime({ ...call, status: 'completed', view: { ...view, status: 'running' } }, undefined)).toBeNull()
})
