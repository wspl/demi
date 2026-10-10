import { expect, test } from 'bun:test'
import type { CommandReport } from '@demicodes/protocol'
import { reportMark, reportSentence } from '../command-reports'

// Cost: pure, a few milliseconds for the file.

const report = (event: CommandReport['event'], title = 'Run the test suite'): CommandReport =>
  ({ commandId: '17', title, event, output: '' })

test('a report row reads the call title and what happened, in the user words', () => {
  expect(reportSentence(report({ kind: 'running', runningMs: 300_000, idleMs: 0, intervalMs: 300_000 })))
    .toBe('Run the test suite is still running')
  expect(reportSentence(report({ kind: 'ended', exitCode: 1 }))).toBe('Run the test suite ended with exit code 1')
  expect(reportSentence(report({ kind: 'ended', exitCode: 0 }))).toBe('Run the test suite ended')
  expect(reportSentence(report({ kind: 'stopped', by: { kind: 'user' } }, 'Start the dev server')))
    .toBe('Start the dev server was stopped by you')
  expect(reportSentence(report({ kind: 'stopped', by: { kind: 'agent', number: 2 } }))).toBe('Run the test suite was stopped by another agent')
  expect(reportSentence(report({ kind: 'lost', reason: 'its Host’s connection ended' }))).toBe('Run the test suite was lost')
  // A call whose title is not known: the command by its number.
  expect(reportSentence(report({ kind: 'ended', exitCode: 1 }, ''))).toBe('Command 17 ended with exit code 1')
})

test('a failure, a stop and a loss carry the tag a shell row carries; progress and success carry none', () => {
  expect(reportMark(report({ kind: 'running', runningMs: 1, idleMs: 0, intervalMs: 15_000 }))).toBeNull()
  expect(reportMark(report({ kind: 'ended', exitCode: 0 }))).toBeNull()
  expect(reportMark(report({ kind: 'ended', exitCode: 2 }))).toEqual({ kind: 'failed', exitCode: 2 })
  expect(reportMark(report({ kind: 'stopped' }))).toEqual({ kind: 'stopped' })
  expect(reportMark(report({ kind: 'lost', reason: 'its Host’s connection ended' })))
    .toEqual({ kind: 'lost', reason: 'its Host’s connection ended' })
})
