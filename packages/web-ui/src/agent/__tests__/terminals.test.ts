import { expect, test } from 'bun:test'
import { DEMO_BUN_TEST, DEMO_GIT_DIFF, DEMO_RG } from '../terminal-demo'
import {
  firstRunningTerminalId,
  runningChipLabel,
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

test('the dock chip names the running count', () => {
  expect(runningChipLabel(0)).toBe('0 Running')
  expect(runningChipLabel(1)).toBe('1 Running')
  expect(runningChipLabel(3)).toBe('3 Running')
})

test('demo jobs carry the CSI colors the xterm specimen shows', () => {
  expect(DEMO_BUN_TEST).toContain('\x1b[92m')
  expect(DEMO_BUN_TEST).toContain('\x1b[91m')
  expect(DEMO_RG).toContain('\x1b[35m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[32m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[31m')
  expect(DEMO_GIT_DIFF).toContain('\x1b[36m')
})
