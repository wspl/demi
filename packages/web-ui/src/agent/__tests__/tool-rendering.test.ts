import { expect, test } from 'bun:test'
import {
  isStandardToolName,
  shouldParsePartialToolInput,
  standardToolRows,
  standardToolTitle,
  trimToolSummary,
  toolRenderKind,
} from '../tool-rendering'

test('standard tool titles prefer description and fall back by concrete tool', () => {
  expect(standardToolTitle('shell_exec', { description: 'Run unit tests', script: 'bun test' })).toBe('Run unit tests')
  expect(standardToolTitle('shell_exec', { script: 'bun test' })).toBe('bun test')
  expect(standardToolTitle('shell_status', { commandId: 'cmd-1' })).toBe('Check cmd-1')
  expect(standardToolTitle('shell_write', { commandId: 'cmd-1' })).toBe('Send input to cmd-1')
  expect(standardToolTitle('shell_abort', { commandId: 'cmd-1' })).toBe('Stop cmd-1')
  expect(standardToolTitle('yield', { durationMs: 250 })).toBe('Wait 250ms')
})

test('standard control tool rows are concrete and omit description-only display state', () => {
  expect(standardToolRows('shell_status', { description: 'Poll', commandId: 'cmd-1', stdoutOffset: 10 })).toEqual([
    { label: 'commandId', value: 'cmd-1', monospace: true },
    { label: 'stdoutOffset', value: '10', monospace: true },
  ])
  expect(standardToolRows('shell_write', { commandId: 'cmd-1', stdin: 'typed\n' })).toEqual([
    { label: 'commandId', value: 'cmd-1', monospace: true },
    { label: 'stdin', value: 'typed', monospace: true },
  ])
  expect(standardToolRows('shell_abort', { commandId: 'cmd-1' })).toEqual([
    { label: 'commandId', value: 'cmd-1', monospace: true },
  ])
  expect(standardToolRows('yield', { durationMs: 1000 })).toEqual([
    { label: 'durationMs', value: '1000', monospace: true },
  ])
})

test('standard tool helpers distinguish Demi tools from unknown generic tools', () => {
  expect(isStandardToolName('shell_exec')).toBe(true)
  expect(isStandardToolName('shell_status')).toBe(true)
  expect(isStandardToolName('shell_write')).toBe(true)
  expect(isStandardToolName('shell_abort')).toBe(true)
  expect(isStandardToolName('yield')).toBe(true)
  expect(isStandardToolName('unknown_tool')).toBe(false)
  expect(shouldParsePartialToolInput('shell_write')).toBe(true)
  expect(shouldParsePartialToolInput('unknown_tool')).toBe(false)
  expect(toolRenderKind('shell_exec')).toBe('shell_exec')
  expect(toolRenderKind('shell_status')).toBe('shell_status')
  expect(toolRenderKind('shell_write')).toBe('shell_write')
  expect(toolRenderKind('shell_abort')).toBe('shell_abort')
  expect(toolRenderKind('yield')).toBe('yield')
  expect(toolRenderKind('unknown_tool')).toBe('generic')
  expect(trimToolSummary(' a\n  b ')).toBe('a b')
})
