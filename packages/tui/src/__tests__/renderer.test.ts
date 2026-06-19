import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demi/core'
import { createRenderer, handleCommand, renderEvent, type TuiOutput } from '../index'

const model: ModelSelection = {
  providerId: 'claude-code',
  model: {
    id: 'claude-opus-4-8',
    name: 'Claude Opus 4.8',
    contextWindow: 200_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: { type: 'effort', effort: 'medium', summary: null },
}

test('TUI renderer prints transcript deltas, tool state, and cache usage without duplicates', () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  const thinking = block({
    type: 'thinking',
    id: 'thinking-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    text: 'plan',
    signature: null,
  })
  const text = block({
    type: 'text',
    id: 'text-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    text: 'hello',
  })
  const tool = block({
    type: 'tool_call',
    id: 'tool-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    toolUseId: 'tool-use-1',
    toolName: 'shell_exec',
    input: JSON.stringify({ script: 'bun test' }),
    status: 'executing',
    streamingOutput: [],
    output: [],
    metadata: null,
  })
  const response = block({
    type: 'response',
    id: 'response-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    usage: { inputTokens: 10, outputTokens: 2, cacheReadTokens: 3, cacheWriteTokens: 4 },
  })

  renderEvent(renderer, { type: 'transcript_snapshot', blocks: [thinking, text, tool, response] })

  expect(output.text()).toContain('thinking> plan')
  expect(output.text()).toContain('assistant> hello')
  expect(output.text()).toContain('tool: shell_exec executing bun test')
  expect(output.text()).toContain('usage: in=10 out=2 cache_read=3 cache_write=4')

  const offset = output.text().length
  renderEvent(renderer, {
    type: 'transcript_patch',
    patches: [],
    blocks: [
      { ...thinking, text: 'plan more' },
      { ...text, text: 'hello world' },
      { ...tool, status: 'completed', output: [{ type: 'text', text: 'ok' }] },
      response,
    ],
  })
  const delta = output.text().slice(offset)

  expect(delta).toContain(' more')
  expect(delta).toContain(' world')
  expect(delta).toContain('tool: shell_exec completed bun test')
  expect(delta).not.toContain('hello world')
  expect(delta).not.toContain('usage:')
})

test('TUI renderer prints phase, queue, shell output, audit, and progress frames', () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)

  renderEvent(renderer, { type: 'phase', phase: 'running' })
  renderEvent(renderer, {
    type: 'queue',
    queue: [{ id: 'queued-1', text: 'next task', content: [{ type: 'text', text: 'next task' }] }],
  })
  renderEvent(renderer, {
    type: 'shell_output',
    shellId: 'shell-1',
    snapshot: {
      stdoutDelta: 'out\n',
      stderrDelta: 'err\n',
      stdoutTail: 'out\n',
      stderrTail: 'err\n',
      totalStdoutBytes: 4,
      totalStderrBytes: 4,
      truncated: false,
    },
  })
  renderEvent(renderer, {
    type: 'audit',
    events: [
      { kind: 'registered-command', name: 'editor', args: ['list'], exitCode: 0 },
      { kind: 'system-command', name: 'bun', args: ['test'], cwd: '/tmp/project', exitCode: 1 },
    ],
  })
  renderEvent(renderer, {
    type: 'tool_progress',
    toolUseId: 'tool-1',
    output: [{ type: 'text', text: JSON.stringify({ shellId: 'shell-1', status: 'running', reason: 'yield' }) }],
  })
  renderEvent(renderer, {
    type: 'tool_progress',
    toolUseId: 'tool-2',
    output: [{ type: 'text', text: 'plain progress' }],
  })
  renderEvent(renderer, { type: 'error', message: 'provider failed', code: 'rate_limit' })
  renderEvent(renderer, { type: 'rejected', command: 'retry', reason: 'busy' })
  renderEvent(renderer, { type: 'closed' })

  const text = output.text()
  expect(text).toContain('status: running')
  expect(text).toContain('queue: 1 message(s)')
  expect(text).toContain('shell[shell-1] stdout> out')
  expect(text).toContain('shell[shell-1] stderr> err')
  expect(text).toContain('audit: registered editor list -> 0')
  expect(text).toContain('audit: system bun test -> 1')
  expect(text).toContain('progress: shell[shell-1] running (yield)')
  expect(text).toContain('progress> plain progress')
  expect(text).toContain('error: provider failed')
  expect(text).toContain('rejected retry: busy')
  expect(text).toContain('closed')
})

test('TUI commands dispatch to the RPC client and validate input usage', async () => {
  const output = new CaptureOutput()
  const client = new FakeCommandClient()

  await expect(handleCommand('/help', client, output)).resolves.toBe(false)
  await expect(handleCommand('/abort', client, output)).resolves.toBe(false)
  await expect(handleCommand('/retry', client, output)).resolves.toBe(false)
  await expect(handleCommand('/resume', client, output)).resolves.toBe(false)
  await expect(handleCommand('/compact', client, output)).resolves.toBe(false)
  await expect(handleCommand('/input shell-1 typed words', client, output)).resolves.toBe(false)
  await expect(handleCommand('/input', client, output)).resolves.toBe(false)
  await expect(handleCommand('/bogus', client, output)).resolves.toBe(false)
  await expect(handleCommand('/exit', client, output)).resolves.toBe(true)

  expect(client.calls).toEqual([
    ['abort'],
    ['retry'],
    ['resume'],
    ['compact'],
    ['shellInput', 'shell-1', 'typed words\n'],
  ])
  expect(output.text()).toContain('Commands:')
  expect(output.text()).toContain('abort requested')
  expect(output.text()).toContain('usage: /input <shellId> <text>')
  expect(output.text()).toContain('Unknown command: /bogus')
})

function block<T extends Block>(value: T): T {
  return value
}

class CaptureOutput implements TuiOutput {
  readonly isTTY = false
  private readonly chunks: string[] = []

  write(text: string): void {
    this.chunks.push(text)
  }

  text(): string {
    return this.chunks.join('')
  }
}

class FakeCommandClient {
  readonly calls: Array<[string] | [string, string, string]> = []

  async abort(): Promise<boolean> {
    this.calls.push(['abort'])
    return true
  }

  async retry(): Promise<void> {
    this.calls.push(['retry'])
  }

  async resume(): Promise<void> {
    this.calls.push(['resume'])
  }

  async compact(): Promise<void> {
    this.calls.push(['compact'])
  }

  async shellInput(shellId: string, stdin: string): Promise<void> {
    this.calls.push(['shellInput', shellId, stdin])
  }
}
