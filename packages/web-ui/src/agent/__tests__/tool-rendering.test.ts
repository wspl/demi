import { expect, test } from 'bun:test'
import {
  isStandardToolName,
  shouldParsePartialToolInput,
  standardToolTitle,
  standardToolTitleParts,
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
  // A look or a wait names its command by the title of the call that
  // started it, never by a bare number.
  const titles = (commandId: string) => ({ '17': 'Run the login test', '18': 'Build' } as Record<string, string>)[commandId]
  expect(standardToolTitle('shell_status', { commandId: 17 }, titles)).toBe('Check Run the login test')
  expect(standardToolTitle('shell_status', { commandId: 9 }, titles)).toBe('Check command 9')
  expect(standardToolTitle('shell_status', { commandId: '17', stdin: 'y\n' }, titles)).toBe(
    'Send input to Run the login test'
  )
  expect(standardToolTitle('yield', { durationMs: 250 })).toBe('Wait 250ms')
  expect(standardToolTitle('yield', { durationMs: 900_000, commandIds: [17, 18] }, titles)).toBe(
    'Wait for Run the login test and 1 more'
  )
  expect(standardToolTitleParts('yield', { durationMs: 1, commandIds: [17, 18, 19] })).toEqual({
    kind: 'reference',
    lead: 'Wait for',
    commandId: '17',
    trail: 'and 2 more',
  })
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
