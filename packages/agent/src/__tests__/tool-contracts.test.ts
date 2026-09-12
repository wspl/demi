import { expect, test } from 'bun:test'
import { z } from 'zod'
import type { AgentToolInvokeContext } from '../types'
import { createStandardAgentTools } from '../tools'
import { modelSelectionSchema } from '../protocol/schemas'

const context: AgentToolInvokeContext<unknown> = {
  agentSessionId: 'test-session',
  state: null,
  cwd: '/workspace',
  model: {
    providerId: 'fixture',
    model: {
      id: 'fixture',
      name: 'Fixture',
      contextWindow: 1000,
      outputLimit: null,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: null,
  },
  toolCallId: 'test-tool',
  signal: new AbortController().signal,
  metadata: null,
  emitProgress: () => {},
}

const cases: Array<{
  name: string;
  valid: Record<string, unknown>;
  invalid: unknown[]
}> = [
  {
    name: 'shell_exec',
    valid: { script: 'pwd', timeoutMs: 1 },
    invalid: [
      { script: 'pwd', timeoutMs: 1, shellId: 42 },
      { script: 'pwd', timeoutMs: 1, shellId: null },
      { script: 'pwd', timeoutMs: 1.5 },
      { script: 'pwd', timeoutMs: 0 },
      { script: 'pwd', timeoutMs: 600001 },
      { script: 'pwd', timeoutMs: '12' },
      { script: 42, timeoutMs: 1 },
    ],
  },
  {
    name: 'shell_status',
    valid: { commandId: 'cmd' },
    invalid: [{ commandId: 42 }],
  },
  {
    name: 'shell_write',
    valid: { commandId: 'cmd', stdin: '\n' },
    invalid: [{ commandId: 'cmd', stdin: '' }, { commandId: 'cmd', stdin: 42 }],
  },
  {
    name: 'shell_abort',
    valid: { commandId: 'cmd' },
    invalid: [{ commandId: null }],
  },
  {
    name: 'yield',
    valid: { durationMs: 600000 },
    invalid: [{ durationMs: 1.5 }, { durationMs: 0 }, { durationMs: 600001 }],
  },
]

for (const fixture of cases) {
  test(`${fixture.name} declaration and execution enforce the same contract`, async () => {
    let reachedExecution = false
    const executionBoundary = new Error('execution boundary')
    const tools = createStandardAgentTools({
      environment: () => {
        reachedExecution = true
        throw executionBoundary
      },
      scheduleYield: () => {
        reachedExecution = true
        throw executionBoundary
      },
    })
    const tool = tools.find((candidate) => candidate.name === fixture.name)!
    const declaration = z.fromJSONSchema(tool.inputSchema)
    const examples = [
      { input: fixture.valid, accepted: true },
      { input: { ...fixture.valid, description: 'Visible result' }, accepted: true },
      ...[...fixture.invalid, null, [], {},
        { ...fixture.valid, extra: true },
        { ...fixture.valid, description: 42 },
      ].map((input) => ({ input, accepted: false })),
    ]
    for (const example of examples) {
      reachedExecution = false
      expect(declaration.safeParse(example.input).success).toBe(example.accepted)
      try {
        await tool.invoke(context, example.input)
        throw new Error('expected validation or execution sentinel')
      } catch (error) {
        if (example.accepted) {
          expect(error).toBe(executionBoundary)
        } else {
          expect(error).toBeInstanceOf(z.ZodError)
        }
      }
      expect(reachedExecution).toBe(example.accepted)
    }
  })
}

test('model selections reject extensions outside the core value set', () => {
  for (const extension of ['png', 'jpg', 'jpeg', 'gif', 'webp', 'mp4', 'mov', 'webm', 'm4v', 'pdf']) {
    expect(modelSelectionSchema.safeParse({
      ...context.model,
      model: { ...context.model.model, acceptedExtensions: [extension] },
    }).success).toBe(true)
  }
  for (const extension of ['not-a-file-extension', '', 42, null]) {
    expect(modelSelectionSchema.safeParse({
      ...context.model,
      model: { ...context.model.model, acceptedExtensions: [extension] },
    }).success).toBe(false)
  }
})
