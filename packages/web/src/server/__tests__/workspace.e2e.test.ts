import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { UserContentBlock } from '@demi/core'
import { defineProvider, type AgentProvider, type InferenceRequest, type Provider, type ProviderEvent } from '@demi/provider'
import { events } from '@demi/provider/testing'
import { AgentWorkspace } from '@demi/web-ui/agent/workspace'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import { parseServerOptions } from '../server-options'
import { startWebServer } from '../serve'
import { createStubProvider } from '../stub-provider'

test('AgentWorkspace drives a conversation through the websocket stack', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-workspace-'))
  const handle = startWebServer([createStubProvider()], testServerOptions(['--cwd', cwd]))

  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)
    const workspace = new AgentWorkspace({ baseUrl: handle.url, control, cwd })

    await workspace.init()

    expect(workspace.order.value.length).toBe(1)
    const id = workspace.activeId.value
    expect(id).not.toBeNull()
    expect(workspace.sessions[id!]?.model.modelId).toBe('stub-model')

    await workspace.send(id!, [{ type: 'text', text: 'hi' }])

    const state = workspace.sessions[id!]!
    expect(state.phase).toBe('idle')
    expect(state.hasContent).toBe(true)
    expect(state.blocks.some((block) => block.type === 'text' && block.text.includes('Hello from the stub provider.'))).toBe(true)

    await workspace.dispose()
  } finally {
    await handle.stop()
  }
})

test('AgentWorkspace steers a queued message through the websocket stack', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-workspace-'))
  const provider = new GateProvider([
    [events.text('active output'), events.response()],
    [events.text('continued output'), events.response()],
  ])
  const handle = startWebServer([createGateProvider(provider)], testServerOptions(['--cwd', cwd]))

  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)
    const workspace = new AgentWorkspace({ baseUrl: handle.url, control, cwd })

    await workspace.init()

    const id = workspace.activeId.value
    expect(id).not.toBeNull()

    const activeSend = workspace.send(id!, text('active'))
    await provider.waitForRun(0)

    const queuedSend = workspace.send(id!, text('queued as steer'))
    await waitFor(() => workspace.sessions[id!]?.queue.length === 1)
    const queuedId = workspace.sessions[id!]!.queue[0]!.id

    await workspace.steerQueuedMessage(id!, queuedId)
    await queuedSend
    expect(workspace.sessions[id!]!.queue).toEqual([])
    expect(workspace.sessions[id!]!.pendingSteers).toHaveLength(1)

    provider.release(0)
    await provider.waitForRun(1)
    expect(provider.requests[1]!.items.map((item) => item.type)).toEqual([
      'user_message',
      'assistant_text',
      'user_steer',
    ])
    expect(provider.requests[1]!.items[2]).toEqual({
      type: 'user_steer',
      turnId: expect.any(String),
      content: text('queued as steer'),
    })

    provider.release(1)
    await activeSend
    await waitFor(() => workspace.sessions[id!]?.phase === 'idle')

    const state = workspace.sessions[id!]!
    expect(state.pendingSteers).toEqual([])
    expect(state.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response', 'steer', 'text', 'response'])

    await workspace.dispose()
  } finally {
    await handle.stop()
  }
})

const EPOCH = '1970-01-01T00:00:00.000Z'

function createGateProvider(provider: AgentProvider): Provider {
  return defineProvider({
    id: 'claude-code',
    displayName: 'Gate Provider',
    state: () => ({ status: 'ready' }),
    listModels: () => ({
      providerId: 'claude-code',
      models: [
        {
          providerId: 'claude-code',
          id: 'stub-model',
          displayName: 'Stub Model',
          contextWindow: 200_000,
          outputLimit: 8_192,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: false,
          supportedThinkingEfforts: null,
          defaultThinkingEffort: null,
          sourceFetchedAt: EPOCH,
          stale: false,
        },
      ],
      defaultModelId: 'stub-model',
      warnings: [],
      sourceFetchedAt: EPOCH,
      stale: false,
    }),
    createRuntime: () => provider,
  })
}

class GateProvider implements AgentProvider {
  private readonly gates: Array<Deferred<void>>
  private readonly started = new Map<number, Deferred<void>>()
  readonly requests: InferenceRequest[] = []

  constructor(private readonly turns: ProviderEvent[][]) {
    this.gates = turns.map(() => deferred<void>())
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const index = this.requests.length
    this.requests.push(request)
    this.started.get(index)?.resolve()
    await this.gates[index]!.promise
    for (const event of this.turns[index] ?? []) yield event
  }

  waitForRun(index: number): Promise<void> {
    if (this.requests.length > index) return Promise.resolve()
    const existing = this.started.get(index)
    if (existing) return existing.promise
    const next = deferred<void>()
    this.started.set(index, next)
    return next.promise
  }

  release(index: number): void {
    this.gates[index]?.resolve(undefined)
  }
}

function text(value: string): UserContentBlock[] {
  return [{ type: 'text', text: value }]
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) throw new Error('Timed out waiting for predicate')
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
}

function testServerOptions(args: string[]) {
  return { ...parseServerOptions(args), port: 0 }
}
