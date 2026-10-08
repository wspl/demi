import { expect, test } from 'bun:test'
import {
  isStandardToolName,
  shouldParsePartialToolInput,
  standardToolTitle,
  trimToolSummary,
  toolRenderKind,
} from '../tool-rendering'

test('standard tool titles prefer description and fall back by concrete tool', () => {
  expect(standardToolTitle(
      'shell_exec',
      {
        description: 'Run unit tests',
        script: 'bun test'
      }
    )).toBe(
    'Run unit tests'
  )
  expect(standardToolTitle('shell_exec', { script: 'bun test' })).toBe('bun test')
  expect(standardToolTitle('shell_status', { commandId: 17 })).toBe('Check 17')
  expect(standardToolTitle('shell_status', { commandId: '17', stdin: 'y\n' })).toBe(
    'Send input to 17'
  )
  expect(standardToolTitle('yield', { durationMs: 250 })).toBe('Wait 250ms')
  expect(standardToolTitle('yield', { durationMs: 900_000, commandIds: [17, 18] })).toBe(
    'Wait for 17, 18'
  )
})

test('standard tool helpers distinguish Demi tools from unknown generic tools', () => {
  expect(isStandardToolName('shell_exec')).toBe(true)
  expect(isStandardToolName('shell_status')).toBe(true)
  expect(isStandardToolName('yield')).toBe(true)
  expect(isStandardToolName('unknown_tool')).toBe(false)
  expect(shouldParsePartialToolInput('shell_status')).toBe(true)
  expect(shouldParsePartialToolInput('unknown_tool')).toBe(false)
  expect(toolRenderKind('shell_exec')).toBe('shell_exec')
  expect(toolRenderKind('shell_status')).toBe('shell_status')
  // A call to a tool the runtime no longer has, as an older transcript
  // holds, renders as a generic tool card.
  expect(toolRenderKind('shell_write')).toBe('generic')
  expect(toolRenderKind('shell_abort')).toBe('generic')
  expect(toolRenderKind('yield')).toBe('yield')
  expect(toolRenderKind('unknown_tool')).toBe('generic')
  expect(trimToolSummary(' a\n  b ')).toBe('a b')
})
