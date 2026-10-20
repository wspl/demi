import { expect, test } from 'bun:test'
import type { ShellToolView } from '../block-types'
import { commandEndMark, commandStatusLine, runningTime } from '../command-end'
import { reportMark } from '../command-reports'

function view(parts: Partial<ShellToolView>): ShellToolView {
  return {
    kind: 'shell',
    status: 'exited',
    commandId: '17',
    runningMs: 0,
    idleMs: 0,
    chunks: [],
    viewTruncated: false,
    ...parts,
  }
}

test('a row marks only a command that failed or was stopped', () => {
  expect(commandEndMark(view({ status: 'exited', exitCode: 1 }))).toEqual({ kind: 'failed', exitCode: 1 })
  expect(commandEndMark(view({ status: 'aborted' }))).toEqual({ kind: 'stopped' })
  expect(commandEndMark(view({ status: 'exited', exitCode: 0 }))).toBeNull()
  expect(commandEndMark(view({ status: 'running' }))).toBeNull()
  expect(commandEndMark(null)).toBeNull()
})

// An open shell row says above its output how long the command has run or
// ran, however it ended.
test('an open row says how long its command has run, or how it ended and after how long', () => {
  expect(commandStatusLine(null, 88_000)).toBe('Running for 1m 28s')
  expect(commandStatusLine(view({ status: 'exited', exitCode: 0 }), 133_000)).toBe('Exited with 0 after 2m 13s')
  expect(commandStatusLine(view({ status: 'exited', exitCode: 1 }), 47_000)).toBe('Failed with exit code 1 after 47s')
  expect(commandStatusLine(view({ status: 'aborted' }), 5_000)).toBe('Stopped after 5s')
  // A stop waits for the runner's word of the end
  // (`crates/runner-shell/src/job.rs`): the row says so in place of the time it
  // has run.
  expect(commandStatusLine({ status: 'running', stopping: true }, 88_000)).toBe('Stopping…')
  expect(commandEndMark({ status: 'running', stopping: true })).toEqual({ kind: 'stopping' })
  // A command whose time the page does not know, as one that ran on after
  // its call returned and ended while no page watched, says nothing.
  expect(commandStatusLine(null, null)).toBeNull()
})

// A running command wears its time as a tag only past 3 seconds, so a quick
// one does not flash one; a running run's title says it from the same point.
test('a running row shows its time only once it has run more than 3 seconds', () => {
  expect(runningTime(null)).toBeNull()
  expect(runningTime(2_900)).toBeNull()
  expect(runningTime(3_000)).toBeNull()
  expect(runningTime(3_400)).toBe('3s')
  expect(runningTime(92_000)).toBe('1m 32s')
  expect(runningTime(3_720_000)).toBe('1h 2m')
})

test('a lost command\'s call row carries its notice\'s tag, Lost with why, not a failure', () => {
  const reason = 'the Host\'s runner started anew after it ended'
  const lost = view({ status: 'lost', lostReason: reason })
  const notice = reportMark({ commandId: '17', title: 'Run the dev server', event: { kind: 'lost', reason }, output: '' })
  expect(commandEndMark(lost)).toEqual(notice)
  expect(notice).toEqual({ kind: 'lost', reason })
  expect(commandStatusLine(lost, 12_000)).toBe(`Lost: ${reason}`)
})
