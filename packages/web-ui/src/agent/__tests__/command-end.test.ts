import { expect, test } from 'bun:test'
import type { ShellToolView } from '../block-types'
import { commandEndMark, commandEndWords } from '../command-end'
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

test('a row marks only a command that failed or was stopped, and an open row says only those ends', () => {
  expect(commandEndMark(view({ status: 'exited', exitCode: 1 }))).toEqual({ kind: 'failed', exitCode: 1 })
  expect(commandEndMark(view({ status: 'aborted' }))).toEqual({ kind: 'stopped' })
  expect(commandEndMark(view({ status: 'exited', exitCode: 0 }))).toBeNull()
  expect(commandEndMark(view({ status: 'running' }))).toBeNull()
  expect(commandEndMark(null)).toBeNull()
  expect(commandEndWords(view({ status: 'exited', exitCode: 1 }))).toBe('Exited with code 1')
  expect(commandEndWords(view({ status: 'aborted' }))).toBe('Stopped')
  expect(commandEndWords(view({ status: 'exited', exitCode: 0 }))).toBeNull()
  expect(commandEndWords(view({ status: 'running' }))).toBeNull()
})

test('a lost command\'s call row carries its notice\'s tag, Lost with why, not a failure', () => {
  const reason = 'the Host\'s runner started anew after it ended'
  const lost = view({ status: 'lost', lostReason: reason })
  const notice = reportMark({ commandId: '17', title: 'Run the dev server', event: { kind: 'lost', reason }, output: '' })
  expect(commandEndMark(lost)).toEqual(notice)
  expect(notice).toEqual({ kind: 'lost', reason })
  expect(commandEndWords(lost)).toBe(`Lost: ${reason}`)
})
