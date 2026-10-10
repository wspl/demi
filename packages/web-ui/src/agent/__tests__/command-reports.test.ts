import { expect, test } from 'bun:test'
import type { CommandReport } from '@demicodes/protocol'
import { reportMark, reportOutcome, reportTitle } from '../command-reports'

// Cost: pure, a few milliseconds for the file.

const report = (event: CommandReport['event'], title = 'Run the test suite'): CommandReport =>
  ({ commandId: '17', title, event, output: '' })

test('a report row names the call and, set apart from it, what happened, in the user words', () => {
  // A title is an imperative: joined into one sentence it read wrongly,
  // "Restart the end-to-end suite is still running".
  const row = (r: CommandReport) => [reportTitle(r), reportOutcome(r)]
  expect(row(report({ kind: 'running', runningMs: 300_000, idleMs: 0, intervalMs: 300_000 }, 'Restart the end-to-end suite')))
    .toEqual(['Restart the end-to-end suite', 'still running'])
  expect(row(report({ kind: 'ended', exitCode: 1 }))).toEqual(['Run the test suite', 'ended with exit code 1'])
  expect(row(report({ kind: 'ended', exitCode: 0 }))).toEqual(['Run the test suite', 'ended'])
  expect(row(report({ kind: 'stopped', by: { kind: 'user' } }, 'Start the dev server')))
    .toEqual(['Start the dev server', 'stopped by you'])
  expect(reportOutcome(report({ kind: 'stopped', by: { kind: 'agent', number: 2 } }))).toBe('stopped by another agent')
  expect(reportOutcome(report({ kind: 'lost', reason: 'its Host’s connection ended' }))).toBe('lost')
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
