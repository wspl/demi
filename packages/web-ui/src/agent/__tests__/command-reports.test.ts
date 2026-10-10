import { expect, test } from 'bun:test'
import type { CommandReport } from '@demicodes/protocol'
import { reportMark, reportNotice, reportTitle } from '../command-reports'

// Cost: pure, a few milliseconds for the file.

const report = (event: CommandReport['event'], title = 'Run the test suite'): CommandReport =>
  ({ commandId: '17', title, event, output: '' })

test('an end notice says what happened before the call it names; progress has no notice', () => {
  // A title is an imperative: as the subject it read wrongly, "Restart the
  // end-to-end suite is still running", so it stands as the object.
  const row = (r: CommandReport) => [reportNotice(r), reportTitle(r)]
  expect(reportNotice(report({ kind: 'running', runningMs: 300_000, idleMs: 0, intervalMs: 300_000 }))).toBeNull()
  expect(row(report({ kind: 'ended', exitCode: 0 }))).toEqual(['Command finished', 'Run the test suite'])
  expect(row(report({ kind: 'ended', exitCode: 1 }))).toEqual(['Command failed with exit code 1', 'Run the test suite'])
  expect(row(report({ kind: 'stopped', by: { kind: 'user' } }, 'Start the dev server')))
    .toEqual(['Command stopped by you', 'Start the dev server'])
  expect(reportNotice(report({ kind: 'stopped', by: { kind: 'agent', number: 2 } }))).toBe('Command stopped by another agent')
  expect(reportNotice(report({ kind: 'lost', reason: 'its Host’s connection ended' }))).toBe('Command lost')
  // A call whose title is not known: the command by its number.
  expect(reportTitle(report({ kind: 'ended', exitCode: 1 }, ''))).toBe('Command 17')
})

test('a failure, a stop and a loss carry the tag a shell row carries; progress and success carry none', () => {
  expect(reportMark(report({ kind: 'running', runningMs: 1, idleMs: 0, intervalMs: 15_000 }))).toBeNull()
  expect(reportMark(report({ kind: 'ended', exitCode: 0 }))).toBeNull()
  expect(reportMark(report({ kind: 'ended', exitCode: 2 }))).toEqual({ kind: 'failed', exitCode: 2 })
  expect(reportMark(report({ kind: 'stopped' }))).toEqual({ kind: 'stopped' })
  expect(reportMark(report({ kind: 'lost', reason: 'its Host’s connection ended' })))
    .toEqual({ kind: 'lost', reason: 'its Host’s connection ended' })
})
