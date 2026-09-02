import { mkdtemp, mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { hostlessShellFactory } from '@demicodes/command-loader/testing'
import type { Block, ModelSelection } from '@demicodes/core'
import { LocalHost } from '@demicodes/shell/node'
import { defineProvider, type InferenceRequest } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer, type AgentHarness, type ClientSessionEvent } from '../index'

// The migration primitive in miniature (M0): one AgentSession whose harness
// switches Host targets between turns, with the switch announced to the model
// as an injected context block (the harness preamble). The product's target
// switching (demi-next M4) builds on exactly this mechanism.

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
const selection = { providerId: 'stub', model }

test('switching Host between turns injects a context block and keeps one continuous transcript', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-switch-'))
  const pathA = join(root, 'target-a')
  const pathB = join(root, 'target-b')
  await Promise.all([mkdir(pathA), mkdir(pathB)])
  const hostA = new LocalHost(pathA)
  const hostB = new LocalHost(pathB)

  // Mimics the backend: resolve the Host from action metadata and announce a
  // target change with a context block on the first turn after the switch.
  let lastTarget: string | null = null
  const harness: AgentHarness<Record<string, never>> = {
    name: 'host-switch-migration-test',
    initialState: () => ({}),
    host: (ctx) => {
      if (!('metadata' in ctx)) return hostA
      const target = ctx.metadata?.target
      if (target === 'a') return hostA
      if (target === 'b') return hostB
      throw new Error('target metadata is required for shell access')
    },
    systemPrompt: () => 'test',
    preamble: (ctx) => {
      const target = typeof ctx.metadata?.target === 'string' ? ctx.metadata.target : null
      if (target === null) return null
      const previous = lastTarget
      lastTarget = target
      if (previous === null || previous === target) return null
      const from = previous === 'a' ? pathA : pathB
      const to = target === 'a' ? pathA : pathB
      return (
        `Execution target switched from ${from} to ${to}. ` +
        'Files and full command outputs from earlier turns stay on the previous target; ' +
        'paths you saw before the switch are not reachable on the new target.'
      )
    },
  }

  const requests: InferenceRequest[] = []
  const capture = (turn: ReturnType<(typeof events)['text']>[]) => {
    return (request: InferenceRequest) => {
      requests.push(request)
      return turn
    }
  }
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        // Turn 1 on A: create state in the shell (cwd) and on disk (marker file).
        capture([events.toolCall('a-1', 'shell_exec', { script: 'mkdir sub && cd sub && printf ready > marker.txt && pwd', timeoutMs: 5_000 })]),
        capture([events.text('done on a'), events.response()]),
        // Turn 2 on B: a fresh shell on the new target; A's files are absent.
        capture([events.toolCall('b-1', 'shell_exec', { script: 'pwd && ls', timeoutMs: 5_000 })]),
        capture([events.text('done on b'), events.response()]),
        // Turn 3 back on A: A's shell environment (and its cwd) survived the excursion.
        capture([events.toolCall('a-2', 'shell_exec', { script: 'pwd && cat marker.txt', timeoutMs: 5_000 })]),
        capture([events.text('done back on a'), events.response()]),
      ]),
  })

  const server = new AgentServer({ shellEnvironment: hostlessShellFactory, agent: harness, providers: [provider] })
  const client = server.client()
  const shellOutputs: ClientSessionEvent[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output') shellOutputs.push(event)
  })
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, pathA, sessionId)

  await client.send([{ type: 'text', text: 'work on a' }], { metadata: { target: 'a' } })
  await client.send([{ type: 'text', text: 'now on b' }], { metadata: { target: 'b' } })
  await client.send([{ type: 'text', text: 'back to a' }], { metadata: { target: 'a' } })

  // Tool execution followed the target, and each Host kept its own shell state.
  const output = shellOutputs
    .filter((event) => event.type === 'shell_output' && event.status.status === 'exited')
    .map((event) => (event.type === 'shell_output' ? event.status.stdout.delta.trim() : ''))
  expect(output).toEqual([
    join(pathA, 'sub'),
    pathB, // fresh shell at B's root — and `ls` printed nothing: no marker.txt
    `${join(pathA, 'sub')}\nready`,
  ])

  // The switch context block reached the model: turn 2's request replays the
  // user turn with the preamble text prepended; turn 1 and turn 3→A-again got none
  // (turn 3 switched back, so it carries its own announcement).
  const userTexts = (request: InferenceRequest): string[] =>
    request.items
      .filter((item): item is Extract<InferenceRequest['items'][number], { type: 'user_message' }> => item.type === 'user_message')
      .flatMap((item) => item.content)
      .flatMap((block) => (block.type === 'text' ? [block.text] : []))
  expect(userTexts(requests[0]).some((text) => text.includes('Execution target switched'))).toBe(false)
  expect(userTexts(requests[2]).some((text) => text.includes(`switched from ${pathA} to ${pathB}`))).toBe(true)
  expect(userTexts(requests[4]).some((text) => text.includes(`switched from ${pathB} to ${pathA}`))).toBe(true)

  // Transcript continuity: one session, all three rounds in order, the
  // context block recorded on the switched turns' user blocks.
  const blocks = client.transcript().blocks
  const userBlocks = blocks.filter((block): block is Extract<Block, { type: 'user' }> => block.type === 'user')
  expect(userBlocks.map((block) => block.content)).toEqual([
    [{ type: 'text', text: 'work on a' }],
    [{ type: 'text', text: 'now on b' }],
    [{ type: 'text', text: 'back to a' }],
  ])
  expect(userBlocks[0].preamble).toBeNull()
  expect(userBlocks[1].preamble).toContain('Execution target switched')
  expect(userBlocks[2].preamble).toContain('Execution target switched')
  const textBlocks = blocks.filter((block): block is Extract<Block, { type: 'text' }> => block.type === 'text')
  expect(textBlocks.map((block) => block.text)).toEqual(['done on a', 'done on b', 'done back on a'])

  await client.close()
  await server.close()
})
