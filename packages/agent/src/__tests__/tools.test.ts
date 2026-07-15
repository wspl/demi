import { expect, test } from 'bun:test'
import type { Model, ModelSelection } from '@demicodes/core'
import type { BashEnvironment, ShellCommandStatus } from '@demicodes/shell'
import type { AgentToolInvokeContext } from '../types'
import { createStandardAgentTools, shellCommandHandleRequired, shellPreviewBudgetTokens, toShellToolResult } from '../tools'

test('standard shell tool schemas do not expose model-controlled output budgets or offsets', () => {
  const tools = createStandardAgentTools({
    environment: {} as BashEnvironment,
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
  expect(shellPreviewBudgetTokens(0)).toBe(1_000)
  expect(shellPreviewBudgetTokens(799_999)).toBe(1_000)
  expect(shellPreviewBudgetTokens(800_000)).toBe(10_000)
  expect(shellPreviewBudgetTokens(2_000_000)).toBe(10_000)
})

test('shell tool result exposes artifact refs and bounded preview without stdout body sections', () => {
  const longOutput = `${'x'.repeat(4_200)}tail`
  const result = toShellToolResult(shellSnapshot(longOutput), {
    includePreview: true,
    previewBudgetTokens: 1_000,
  })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('stdoutPath: /@/commands/cmd-1/stdout.txt')
  expect(text).toContain('stderrPath: /@/commands/cmd-1/stderr.txt')
  expect(text).toContain('metaPath: /@/commands/cmd-1/meta.json')
  expect(text).toContain('previewBudgetTokens: 1000')
  expect(text).toContain('previewTruncated: true')
  expect(text).not.toContain('stdout:\n')
  expect(text).not.toContain('tail')
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
    } as unknown as BashEnvironment,
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
  expect(text).not.toContain('/@/commands/')
  expect(released).toEqual(['cmd-1'])
})

test('completed truncated shell_exec keeps the command handle for artifacts', async () => {
  const released: string[] = []
  const output = `${'x'.repeat(4_200)}tail`
  const tools = createStandardAgentTools({
    environment: {
      exec: async () => shellSnapshot(output),
      releaseCommand: async (commandId: string) => {
        released.push(commandId)
        return true
      },
    } as unknown as BashEnvironment,
    scheduleYield: () => ({ output: [{ type: 'text', text: 'scheduled' }] }),
  })
  const shellExec = tools.find((tool) => tool.name === 'shell_exec')
  if (!shellExec) throw new Error('missing shell_exec')

  const result = await shellExec.invoke(toolContext(), { script: 'printf long', timeoutMs: 1 })
  const text = result.output[0]?.type === 'text' ? result.output[0].text : ''

  expect(text).toContain('commandId: cmd-1')
  expect(text).toContain('stdoutPath: /@/commands/cmd-1/stdout.txt')
  expect(text).toContain('previewTruncated: true')
  expect(released).toEqual([])
})

test('shell command handles are required only for running or over-budget output', () => {
  expect(shellCommandHandleRequired(runningShellSnapshot(''), 1_000)).toBe(true)
  expect(shellCommandHandleRequired(shellSnapshot('short\n'), 1_000)).toBe(false)
  expect(shellCommandHandleRequired(shellSnapshot('x'.repeat(4_001)), 1_000)).toBe(true)
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
    binaryStdout: { data: PNG_STREAM, truncated: false, totalBytes: PNG_STREAM.length },
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
    binaryStdout: { data: MP4_STREAM, truncated: false, totalBytes: MP4_STREAM.length },
  }
  const result = toShellToolResult(snapshot, { includePreview: true, model: imageModel() })
  expect(result.output).toHaveLength(2)
  const note = result.output[1]?.type === 'text' ? result.output[1].text : ''
  expect(note).toContain('video/mp4')
  expect(note).toContain('does not accept')
})

test('unknown binary and truncated streams stay placeholder-only with a reason', () => {
  const opaque = toShellToolResult(
    { ...shellSnapshot('<binary stdout: 12 bytes>\n'), binaryStdout: { data: OPAQUE_STREAM, truncated: false, totalBytes: 12 } },
    { includePreview: true, model: imageModel() },
  )
  const opaqueNote = opaque.output[1]?.type === 'text' ? opaque.output[1].text : ''
  expect(opaque.output).toHaveLength(2)
  expect(opaqueNote).toContain('does not match any model-viewable media type')

  const truncated = toShellToolResult(
    {
      ...shellSnapshot('<binary stdout: 999 bytes, exceeds the 12-byte output limit>\n'),
      binaryStdout: { data: PNG_STREAM, truncated: true, totalBytes: 999 },
    },
    { includePreview: true, model: imageModel() },
  )
  const truncNote = truncated.output[1]?.type === 'text' ? truncated.output[1].text : ''
  expect(truncated.output).toHaveLength(2)
  expect(truncNote).toContain('truncated at the output limit')
  expect(truncNote).toContain('image/png')
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
    exitCode: 0,
    stdout: {
      path: '/@/commands/cmd-1/stdout.txt',
      offset: bytes,
      delta: output,
      tail: output.slice(-128),
      bytes,
      truncated: false,
    },
    stderr: {
      path: '/@/commands/cmd-1/stderr.txt',
      offset: 0,
      delta: '',
      tail: '',
      bytes: 0,
      truncated: false,
    },
    output: {
      path: 'demi://shell/shell-1/commands/cmd-1/output',
      offset: bytes,
      text: output,
      tail: output.slice(-128),
      chunks: [{ stream: 'stdout', text: output }],
      bytes,
      truncated: false,
    },
    runningMs: 1,
    idleMs: 0,
    audit: [],
  }
}

function runningShellSnapshot(output: string): Extract<ShellCommandStatus, { status: 'running' }> {
  const bytes = new TextEncoder().encode(output).byteLength
  return {
    status: 'running',
    shellId: 'shell-1',
    commandId: 'cmd-1',
    stdout: {
      path: '/@/commands/cmd-1/stdout.txt',
      offset: bytes,
      delta: output,
      tail: output.slice(-128),
      bytes,
      truncated: false,
    },
    stderr: {
      path: '/@/commands/cmd-1/stderr.txt',
      offset: 0,
      delta: '',
      tail: '',
      bytes: 0,
      truncated: false,
    },
    output: {
      path: 'demi://shell/shell-1/commands/cmd-1/output',
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
