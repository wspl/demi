import { expect, test } from 'bun:test'
import type { Model } from '@demicodes/core'
import {
  TITLE_MAX_LENGTH,
  lowestThinking,
  titleFromMessage,
  titleFromResponse,
} from '../conversation/title'

function modelWith(thinking: Model['thinking']): Model {
  return {
    id: 'm',
    name: 'M',
    contextWindow: 100_000,
    inputLimit: null,
    outputLimit: null,
    thinking,
    acceptedExtensions: null,
  }
}

test('the message-derived title is the start of the message on one line', () => {
  expect(titleFromMessage('  fix\n\tthe   build  ')).toBe('fix the build')
  expect(titleFromMessage('x'.repeat(200))).toHaveLength(TITLE_MAX_LENGTH)
  expect(titleFromMessage(' \n ')).toBe('')
})

test('the generated title is the first non-empty line, unquoted and cut to length', () => {
  expect(titleFromResponse('\n  "Parser bug fix"  \nmore')).toBe('Parser bug fix')
  expect(titleFromResponse('「修复解析器」')).toBe('修复解析器')
  expect(titleFromResponse(`${'y'.repeat(120)}`)).toHaveLength(TITLE_MAX_LENGTH)
  expect(titleFromResponse('')).toBeNull()
  expect(titleFromResponse(' \n ""\n')).toBeNull()
})

test('the lowest thinking is the least effort an effort model names, and none for the rest', () => {
  expect(lowestThinking(modelWith([{
    type: 'effort',
    efforts: ['high', 'minimal', 'low'],
    defaultEffort: 'high',
    summaries: [],
    defaultSummary: null,
  }]))).toEqual({ type: 'effort', effort: 'minimal', summary: null })
  expect(lowestThinking(modelWith([{
    type: 'adaptive',
    efforts: ['medium', 'custom', 'low'],
    defaultEffort: null,
  }]))).toEqual({ type: 'adaptive', effort: 'low' })
  expect(lowestThinking(modelWith([{
    type: 'budget',
    minBudgetTokens: 1024,
    maxBudgetTokens: 32_000,
    defaultBudgetTokens: 8000,
  }]))).toBeNull()
  expect(lowestThinking(modelWith([{ type: 'disabled' }]))).toBeNull()
  expect(lowestThinking(modelWith([]))).toBeNull()
})
