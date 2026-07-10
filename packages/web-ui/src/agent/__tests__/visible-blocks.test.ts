import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { getVisibleBlocks } from '../visible-blocks'

test('visible blocks hide response blocks because web no longer renders response stats rows', () => {
  const blocks = [
    tool('yield-1', 'yield'),
    response('response-after-yield'),
    tool('status-1', 'shell_status'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['yield-1', 'status-1'])
})

test('visible blocks hide ordinary response blocks too', () => {
  const blocks = [
    tool('exec-1', 'shell_exec'),
    response('response-after-exec'),
    tool('status-1', 'shell_status'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['exec-1', 'status-1'])
})

test('visible blocks hide resume blocks because web has no resume row renderer', () => {
  const blocks = [
    tool('yield-1', 'yield'),
    resume('resume-after-yield'),
    tool('status-1', 'shell_status'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['yield-1', 'status-1'])
})

test('visible blocks hide hidden user/steer turns (internal yield wakeups) but keep real ones', () => {
  const blocks = [
    user('user-1', false),
    tool('exec-1', 'shell_exec'),
    user('wakeup-send', true),
    steer('wakeup-steer', true),
    steer('real-steer', false),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['user-1', 'exec-1', 'real-steer'])
})

function tool(id: string, toolName: string): Extract<Block, { type: 'tool_call' }> {
  return {
    type: 'tool_call',
    id,
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    toolUseId: id,
    toolName,
    input: '{}',
    status: 'completed',
    streamingOutput: [],
    output: [],
    view: null,
  }
}

function response(id: string): Extract<Block, { type: 'response' }> {
  return {
    type: 'response',
    id,
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    usage: {
      inputTokens: 1,
      outputTokens: 1,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
    },
  }
}

function resume(id: string): Extract<Block, { type: 'resume' }> {
  return {
    type: 'resume',
    id,
    turnId: id,
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
  }
}

function user(id: string, hidden: boolean): Extract<Block, { type: 'user' }> {
  return {
    type: 'user',
    id,
    turnId: id,
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    content: [{ type: 'text', text: id }],
    preamble: null,
    ...(hidden ? { hidden: true } : {}),
  }
}

function steer(id: string, hidden: boolean): Extract<Block, { type: 'steer' }> {
  return {
    type: 'steer',
    id,
    turnId: id,
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    content: [{ type: 'text', text: id }],
    ...(hidden ? { hidden: true } : {}),
  }
}

const model: ModelSelection = {
  providerId: 'test',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 1000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}
