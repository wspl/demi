import { memoryAgentStores } from '../testing'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { Model, ModelSelection } from '@demicodes/core'
import { hostlessShellFactory, LocalHost } from '@demicodes/host-virtual/testing'
import { defineProvider, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { ShellCommandStatus, ShellEnvironment } from '@demicodes/shell'
import { AgentServer } from '../server/server'
import type { AgentToolInvokeContext } from '../types'
import { createStandardAgentTools, shellCommandHandleRequired, shellPreviewBudgetTokens, toShellToolResult } from '../tools'

test('standard shell tool schemas do not expose model-controlled output budgets or offsets', () => {
  const tools = createStandardAgentTools({
    environment: {} as ShellEnvironment,
    scheduleYield: () => ({ output: [{ type: 'text', text: 'scheduled' }] }),
  })
  const byName = new Map(tools.map((tool) => [tool.name, tool]))

  expect([...byName.keys()]).toEqual(['shell_exec', 'shell_status', 'shell_write', 'shell_abort', 'yield'])
  for (const name of ['shell_exec', 'shell_status', 'shell_write', 'shell_abort']) {
    const schema = byName.get(name)?.inputSchema as { properties?: Record<string, unknown> }
    expect(schema.properties).not.toHaveProperty('maxOutputBytes')
    expect(schema.properties).not.toHaveProperty('stdoutOffset')
    expect(schema.properties).not.toHaveProperty('stderrOffset')
    expect(schema.properties).toHaveProperty('description')
  }
})

test('shell preview budget follows the 800k context threshold', () => {
  expect(shellPreviewBudgetTokens(0)).toBe(10_000)
  expect(shellPreviewBudgetTokens(799_999)).toBe(10_000)
  expect(shellPreviewBudgetTokens(800_000)).toBe(100_000)
  expect(shellPreviewBudgetTokens(2_000_000)).toBe(100_000)
})

test('shell tool result exposes output paths and a bounded preview without stdout body sections', () => {
  const longOutput = `${'x'.repeat(4_200)}tail`
  const result = toShellToolResult(shellSnapshot(longOutput), {
    includePreview: true,
    previewBudgetTokens: 1_000,
  })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('stdoutPath: /artifacts/session-1/cmd-1/stdout.txt')
  expect(text).toContain('stderrPath: /artifacts/session-1/cmd-1/stderr.txt')
  expect(text).toContain('previewBudgetTokens: 1000')
  expect(text).toContain('previewTruncated: true')
  expect(text).not.toContain('stdout:\n')
  expect(text).not.toContain('tail')
})

test('shell preview truncation never splits a surrogate pair', () => {
  // Budget of 1000 tokens → 4000 chars; the emoji straddles the cut point.
  const output = `${'x'.repeat(3_999)}🙂${'y'.repeat(100)}`
  const result = toShellToolResult(shellSnapshot(output), {
    includePreview: true,
    previewBudgetTokens: 1_000,
  })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('previewTruncated: true')
  expect(text).toMatch(/x{3999}\n/)
  expect(text).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/)
})

test('completed short shell_exec hides and releases the command handle', async () => {
  const released: string[] = []
  const tools = createStandardAgentTools({
    environment: {
      exec: async () => shellSnapshot('done\n'),
      releaseCommand: async (commandId: string) => {
        released.push(commandId)
        return true
      },
    } as unknown as ShellEnvironment,
    scheduleYield: () => ({ output: [{ type: 'text', text: 'scheduled' }] }),
  })
  const shellExec = tools.find((tool) => tool.name === 'shell_exec')
  if (!shellExec) throw new Error('missing shell_exec')

  const result = await shellExec.invoke(toolContext(), { script: 'printf done', timeoutMs: 1 })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('status: exited')
  expect(text).toContain('exitCode: 0')
  expect(text).toContain('preview:')
  expect(text).toContain('done')
  expect(text).not.toContain('commandId:')
  expect(text).not.toContain('stdoutPath:')
  expect(text).not.toContain('/artifacts/session-1/cmd-1')
  expect(released).toEqual(['cmd-1'])
})

test('completed truncated shell_exec keeps the command handle for artifacts', async () => {
  const released: string[] = []
  // Default budget for a small-context model is 10k tokens → 40k chars.
  const output = `${'x'.repeat(40_200)}tail`
  const tools = createStandardAgentTools({
    environment: {
      exec: async () => shellSnapshot(output),
      releaseCommand: async (commandId: string) => {
        released.push(commandId)
        return true
      },
    } as unknown as ShellEnvironment,
    scheduleYield: () => ({ output: [{ type: 'text', text: 'scheduled' }] }),
  })
  const shellExec = tools.find((tool) => tool.name === 'shell_exec')
  if (!shellExec) throw new Error('missing shell_exec')

  const result = await shellExec.invoke(toolContext(), { script: 'printf long', timeoutMs: 1 })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('commandId: cmd-1')
  expect(text).toContain('stdoutPath: /artifacts/session-1/cmd-1/stdout.txt')
  expect(text).toContain('previewTruncated: true')
  expect(released).toEqual([])
})

test.each([
  ['shell_exec', { script: 'printf long', timeoutMs: 1 }],
  ['shell_status', { commandId: 'cmd-1' }],
  ['shell_write', { commandId: 'cmd-1', stdin: 'input\n' }],
  ['shell_abort', { commandId: 'cmd-1' }],
])('%s evaluates the custom preview budget against the current model', async (name, input) => {
  const seen: number[] = []
  const released: string[] = []
  const snapshot = () => shellSnapshot(`${'x'.repeat(100)}tail`)
  const tools = createStandardAgentTools({
    environment: {
      exec: async () => snapshot(),
      status: async () => snapshot(),
      write: async () => snapshot(),
      abort: async () => snapshot(),
      releaseCommand: async (commandId: string) => {
        released.push(commandId)
        return true
      },
    } as unknown as ShellEnvironment,
    scheduleYield: () => ({ output: [{ type: 'text', text: 'scheduled' }] }),
    previewBudgetTokens: (contextWindow) => {
      seen.push(contextWindow)
      return contextWindow / 10_000
    },
  })
  const tool = tools.find((tool) => tool.name === name)
  if (!tool) throw new Error(`missing ${name}`)
  const context = toolContext()

  const result = await tool.invoke(context, input)
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(seen).toEqual([context.model.model.contextWindow])
  expect(text).toContain('previewBudgetTokens: 10')
  expect(text).toContain('previewTruncated: true')
  expect(text).toContain('commandId: cmd-1')
  expect(text).not.toContain('tail')
  expect(released).toEqual([])

  const largerContext = {
    ...context,
    model: { ...context.model, model: { ...context.model.model, contextWindow: 1_000_000 } },
  }
  const largerResult = await tool.invoke(largerContext, input)
  const largerText = largerResult.output[0]?.type === 'text' ? largerResult.output[0].text : ''

  expect(seen).toEqual([context.model.model.contextWindow, 1_000_000])
  expect(largerText).toContain('previewBudgetTokens: 100')
  expect(largerText).not.toContain('previewTruncated: true')
  expect(largerText).not.toContain('commandId:')
  expect(largerText).toContain('tail')
  expect(released).toEqual(['cmd-1'])
})

test('shell command handles are required only for running or over-budget output', () => {
  expect(shellCommandHandleRequired(runningShellSnapshot(''), 1_000)).toBe(true)
  expect(shellCommandHandleRequired(shellSnapshot('short\n'), 1_000)).toBe(false)
  expect(shellCommandHandleRequired(shellSnapshot('x'.repeat(4_001)), 1_000)).toBe(true)
})

test('AgentServer keeps the custom preview policy across model changes and live session takeover', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-preview-policy-'))
  const host = new LocalHost(root, { storeRoot: join(root, '.store') })
  const seenWindows: number[] = []
  const seenResults: string[] = []
  const turns: ConstructorParameters<typeof StubProvider>[0] = []
  for (let index = 0; index < 3; index += 1) {
    turns.push([events.toolCall(`preview-${index}`, 'shell_exec', {
      script: `echo ${'x'.repeat(100)}tail`, timeoutMs: 100,
    })])
    turns.push((request) => {
      const result = [...request.items].reverse().find((item) => item.type === 'tool_result')
      seenResults.push(result?.output[0]?.type === 'text' ? result.output[0].text : '')
      return [events.text('done'), events.response()]
    })
  }
  const runtime = new StubProvider(turns)
  const server = new AgentServer({ store: memoryAgentStores(),
    agent: { name: 'preview', initialState: () => ({}), host: () => host, systemPrompt: () => 'test' },
    providers: [defineProvider({ id: 'stub', displayName: 'Stub', createRuntime: () => runtime })],
    shellEnvironment: hostlessShellFactory,
    tools: {
      shellPreviewBudgetTokens: (contextWindow) => {
        seenWindows.push(contextWindow)
        return contextWindow / 10_000
      },
    },
  })
  const selection = (contextWindow: number): ProviderSelection => ({
    providerId: 'stub', model: { ...model, model: { ...model.model, contextWindow } },
  })
  try {
    const first = server.client()
    await first.open(selection(100_000), root, 'preview-session')
    await first.send([{ type: 'text', text: 'first preview' }])

    first.setProvider(selection(1_000_000))
    await first.send([{ type: 'text', text: 'larger preview' }])

    const adopted = server.client()
    await adopted.open(selection(200_000), root, 'preview-session')
    await adopted.send([{ type: 'text', text: 'preview after takeover' }])

    expect(seenWindows).toEqual([100_000, 1_000_000, 200_000])
    expect(seenResults).toHaveLength(3)
    expect(seenResults[0]).toContain('previewBudgetTokens: 10')
    expect(seenResults[0]).not.toContain('tail')
    expect(seenResults[1]).toContain('previewBudgetTokens: 100')
    expect(seenResults[1]).toContain('tail')
    expect(seenResults[2]).toContain('previewBudgetTokens: 20')
    expect(seenResults[2]).not.toContain('tail')
  } finally {
    await server.close()
    await rm(root, { recursive: true, force: true })
  }
})

const PNG_STREAM = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])
const MP4_STREAM = new Uint8Array([0, 0, 0, 0x20, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d, 0xff, 0xfe])
const OPAQUE_STREAM = new Uint8Array([0xde, 0xad, 0xbe, 0xef, 0xff, 0xfe, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05])

function imageModel(): Model {
  return {
    id: 'm',
    name: 'M',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'],
  }
}

test('a sniffed binary stream the model accepts is attached as a media block', () => {
  const snapshot = {
    ...shellSnapshot('<binary stdout: 12 bytes>\n'),
    binaryStdout: { data: PNG_STREAM, truncated: false, totalBytes: PNG_STREAM.length, limitBytes: 4 * 1024 * 1024 },
  }
  const result = toShellToolResult(snapshot, { includePreview: true, model: imageModel() })

  expect(result.output).toHaveLength(3)
  expect(result.output[1]).toEqual({
    type: 'image',
    source: { mediaType: 'image/png', data: Buffer.from(PNG_STREAM).toString('base64') },
  })
  const note = result.output[2]?.type === 'text' ? result.output[2].text : ''
  expect(note).toContain('Attached stdout as image/png')
})

test('a media stream the model does not accept explains why nothing was attached', () => {
  const snapshot = {
    ...shellSnapshot('<binary stdout: 14 bytes>\n'),
    binaryStdout: { data: MP4_STREAM, truncated: false, totalBytes: MP4_STREAM.length, limitBytes: 16 * 1024 * 1024 },
  }
  const result = toShellToolResult(snapshot, { includePreview: true, model: imageModel() })
  expect(result.output).toHaveLength(2)
  const note = result.output[1]?.type === 'text' ? result.output[1].text : ''
  expect(note).toContain('video/mp4')
  expect(note).toContain('does not accept')
})

test('unknown binary and truncated streams stay placeholder-only with a reason', () => {
  const opaque = toShellToolResult(
    { ...shellSnapshot('<binary stdout: 12 bytes>\n'), binaryStdout: { data: OPAQUE_STREAM, truncated: false, totalBytes: 12, limitBytes: 1024 * 1024 } },
    { includePreview: true, model: imageModel() },
  )
  const opaqueNote = opaque.output[1]?.type === 'text' ? opaque.output[1].text : ''
  expect(opaque.output).toHaveLength(2)
  expect(opaqueNote).toContain('does not match any model-viewable media type')

  const truncated = toShellToolResult(
    {
      ...shellSnapshot('<binary stdout: 999 bytes, exceeds the 12-byte output limit>\n'),
      binaryStdout: { data: PNG_STREAM, truncated: true, totalBytes: 999, limitBytes: 12 },
    },
    { includePreview: true, model: imageModel() },
  )
  const truncNote = truncated.output[1]?.type === 'text' ? truncated.output[1].text : ''
  expect(truncated.output).toHaveLength(2)
  expect(truncNote).toContain("exceeded the shell's 12-byte binary limit (maxBinaryBytes)")
  expect(truncNote).toContain('image/png')
})

test('media over its modality cap is withheld and points at a smaller version', () => {
  // The shell only bounds raw size; whether these bytes are worth the context
  // is decided here, where both the modality and the model are known.
  const result = toShellToolResult(
    {
      ...shellSnapshot('<binary stdout: 12 bytes>\n'),
      binaryStdout: { data: PNG_STREAM, truncated: false, totalBytes: PNG_STREAM.length, limitBytes: 16 * 1024 * 1024 },
    },
    { includePreview: true, model: imageModel(), maxMediaBytes: { image: 4 } },
  )
  const note = result.output[1]?.type === 'text' ? result.output[1].text : ''
  expect(result.output).toHaveLength(2)
  expect(note).toContain('over the 4-byte image cap')
  expect(note).toContain('smaller version')
})

test('shell tool result without binary stdout stays text-only', () => {
  const result = toShellToolResult(shellSnapshot('done\n'), { includePreview: true })
  expect(result.output).toHaveLength(1)
  expect(result.output[0]?.type).toBe('text')
})

function shellSnapshot(output: string): Extract<ShellCommandStatus, { status: 'exited' }> {
  const bytes = new TextEncoder().encode(output).byteLength
  return {
    status: 'exited',
    shellId: 'shell-1',
    commandId: 'cmd-1',
    outputDir: '/artifacts/session-1/cmd-1',
    exitCode: 0,
    stdout: {
      path: '/artifacts/session-1/cmd-1/stdout.txt',
      offset: bytes,
      delta: output,
      tail: output.slice(-128),
      bytes,
      truncated: false,
    },
    stderr: {
      path: '/artifacts/session-1/cmd-1/stderr.txt',
      offset: 0,
      delta: '',
      tail: '',
      bytes: 0,
      truncated: false,
    },
    output: {
      path: '/artifacts/session-1/cmd-1',
      offset: bytes,
      text: output,
      tail: output.slice(-128),
      chunks: [{ stream: 'stdout', text: output }],
      bytes,
      truncated: false,
    },
    runningMs: 1,
    idleMs: 0,
  }
}

function runningShellSnapshot(output: string): Extract<ShellCommandStatus, { status: 'running' }> {
  const bytes = new TextEncoder().encode(output).byteLength
  return {
    status: 'running',
    shellId: 'shell-1',
    commandId: 'cmd-1',
    outputDir: '/artifacts/session-1/cmd-1',
    stdout: {
      path: '/artifacts/session-1/cmd-1/stdout.txt',
      offset: bytes,
      delta: output,
      tail: output.slice(-128),
      bytes,
      truncated: false,
    },
    stderr: {
      path: '/artifacts/session-1/cmd-1/stderr.txt',
      offset: 0,
      delta: '',
      tail: '',
      bytes: 0,
      truncated: false,
    },
    output: {
      path: '/artifacts/session-1/cmd-1',
      offset: bytes,
      text: output,
      tail: output.slice(-128),
      chunks: output ? [{ stream: 'stdout', text: output }] : [],
      bytes,
      truncated: false,
    },
    runningMs: 1,
    idleMs: 0,
  }
}

function toolContext(): AgentToolInvokeContext<unknown> {
  return {
    agentSessionId: 'agent-1',
    state: null,
    cwd: '/workspace',
    model,
    toolCallId: 'tool-1',
    signal: new AbortController().signal,
    metadata: null,
    emitProgress: () => {},
  }
}

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'stub-model',
    name: 'Stub Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}
