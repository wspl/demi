import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import { encodeUtf8, waitFor } from '@demicodes/utils'
import type { Block, ModelSelection } from '@demicodes/core'
import type { AgentHarness } from '@demicodes/agent'
import type { CommandSpec, Host } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type Provider, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer } from '../index'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

const selection: ProviderSelection = { providerId: 'stub', model }

function demoCommand(host: Host): CommandSpec {
  return {
    name: 'demo',
    summary: 'demo commands',
    subcommands: [
      {
        name: 'write',
        summary: 'Write a file from stdin.',
        effects: 'creates or overwrites one file',
        input: {
          path: z.string().describe('Target path'),
          body: z.string().describe('File body from stdin'),
          upper: z.boolean().optional().describe('Uppercase the body'),
        },
        positionals: ['path'],
        stdinField: 'body',
        examples: ["demo write out.txt <<'EOF'\nhello\nEOF"],
        run: async ({ parsed, cwd, io }) => {
          const path = String(parsed.values.path)
          let body = String(parsed.values.body)
          if (parsed.values.upper === true) body = body.toUpperCase()
          await host.fs.writeFile(path, encodeUtf8(body), { cwd, createParents: true })
          await io.stdout(`wrote ${path}\n`)
          return { exitCode: 0, metadata: { wrote: path } }
        },
      },
    ],
  }
}

function runtimeProvider(turnsFactory: () => ConstructorParameters<typeof StubProvider>[0]): Provider {
  return defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () => new StubProvider(turnsFactory()),
  })
}

function createHarness(cwd: string): AgentHarness<Record<string, never>> {
  const host = new LocalHost(cwd)
  return {
    name: 'projection-test',
    initialState: () => ({}),
    host: () => host,
    commands: () => [demoCommand(host)],
    systemPrompt: (ctx) => `test\n${ctx.commandsPrompt}`,
  }
}

test('native tool projection and shell heredoc produce identical results', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-projection-'))
  const body = "it's a\nmulti 'quoted' line"

  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [
    [
      events.toolCall('t1', 'demo_write', { path: "native it's.txt", body, upper: true }),
      events.response(),
    ],
    [
      events.toolCall('t2', 'shell_exec', {
        script: `demo write --path 'shell.txt' --upper true <<'EOF'\n${body}\nEOF`,
        timeoutMs: 30_000,
      }),
      events.response(),
    ],
    [events.text('done'), events.response()],
  ]

  const server = new AgentServer({
    agent: createHarness(cwd),
    providers: [runtimeProvider(turns)],
    commandTools: {},
  })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text'))

  const expected = `${body}\n`.toUpperCase()
  expect(await readFile(join(cwd, "native it's.txt"), 'utf8')).toBe(expected)
  expect(await readFile(join(cwd, 'shell.txt'), 'utf8')).toBe(expected)

  // Both projections leave the same registered-command audit trail.
  const toolCalls = client
    .transcript()
    .blocks.filter((block): block is Extract<Block, { type: 'tool_call' }> => block.type === 'tool_call')
  expect(toolCalls).toHaveLength(2)
  for (const call of toolCalls) {
    const metadata = call.metadata as { audit?: Array<{ kind: string; name: string; exitCode: number }> } | null
    const audit = metadata?.audit ?? []
    expect(audit.some((event) => event.kind === 'registered-command' && event.name === 'demo' && event.exitCode === 0)).toBe(
      true,
    )
  }

  await client.close()
  await server.close()
})

test('projection is off by default and validates name conflicts', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-projection-off-'))
  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [
    [events.toolCall('t1', 'demo_write', { path: 'x.txt', body: 'hi' }), events.response()],
    [events.text('done'), events.response()],
  ]
  const server = new AgentServer({ agent: createHarness(cwd), providers: [runtimeProvider(turns)] })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text'))

  // Without projection the tool does not exist.
  const call = client.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(call).toMatchObject({ status: 'error' })

  await client.close()
  await server.close()
})

test('projected tool schemas carry field descriptions and requiredness', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-projection-schema-'))
  const requests: Array<{ tools: Array<{ name: string; inputSchema: Record<string, unknown> }> }> = []
  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [
    (request) => {
      requests.push({ tools: request.tools })
      return [events.text('ok'), events.response()]
    },
  ]
  const server = new AgentServer({
    agent: createHarness(cwd),
    providers: [runtimeProvider(turns)],
    commandTools: {},
  })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => requests.length > 0)

  const tool = requests[0]!.tools.find((candidate) => candidate.name === 'demo_write')
  expect(tool).toBeDefined()
  const schema = tool!.inputSchema as {
    properties: Record<string, { type?: string; description?: string }>
    required?: string[]
  }
  expect(schema.properties.path).toMatchObject({ type: 'string', description: 'Target path' })
  expect(schema.properties.upper).toMatchObject({ type: 'boolean' })
  expect(schema.required).toContain('path')
  expect(schema.required).toContain('body')
  expect(schema.required).not.toContain('upper')

  await client.close()
  await server.close()
})
