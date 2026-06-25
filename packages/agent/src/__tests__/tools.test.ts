import { expect, test } from 'bun:test'
import type { BashEnvironment, ShellCommandSnapshot } from '@demi/shell'
import { createStandardAgentTools, shellPreviewBudgetTokens, toShellToolResult } from '../tools'

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
  const result = toShellToolResult(shellSnapshot(longOutput), 'tool-1', {
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

function shellSnapshot(output: string): ShellCommandSnapshot {
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
