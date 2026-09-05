import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { canResumeFromDock } from '../dock-recovery'

const model: ModelSelection = {
  providerId: 'p',
  model: { id: 'm', name: 'M', contextWindow: 1000, inputLimit: 900, thinking: [], acceptedExtensions: [] },
  thinking: null,
}
const text: Block = { type: 'text', id: 't', createdAt: '', model, text: 'done' }
const abort: Block = { type: 'abort', id: 'a', createdAt: '', model, isResumed: false }
const resumedAbort: Block = { type: 'abort', id: 'a2', createdAt: '', model, isResumed: true }
const error: Block = { type: 'error', id: 'e', createdAt: '', model, message: 'boom', code: null }

test('an idle conversation ending in an abort or error can resume', () => {
  expect(canResumeFromDock('idle', [text, abort])).toBe(true)
  expect(canResumeFromDock('idle', [text, error])).toBe(true)
})

test('a finished tail, a running turn, or a resumed abort offers nothing', () => {
  expect(canResumeFromDock('idle', [abort, text])).toBe(false)
  expect(canResumeFromDock('running', [text, abort])).toBe(false)
  expect(canResumeFromDock('idle', [text, resumedAbort])).toBe(false)
  expect(canResumeFromDock('idle', [])).toBe(false)
})
