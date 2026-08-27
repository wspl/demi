import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import type { ModelSelection } from '@demicodes/core'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type InferenceRequest, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import {
  AgentServer,
  MAX_LIVE_SUBAGENTS,
  createReadonlyHost,
  type AgentClient,
  type AgentHarness,
  type AgentMetadata,
  type ClientSessionEvent,
  type SubagentProfile,
} from '../index'

type TurnScript = ConstructorParameters<typeof StubProvider>[0][number]

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

async function openHarness(options: {
  turns: TurnScript[]
  agents?: SubagentProfile<Record<string, never>>[]
  notifyParentOnIdle?: boolean
  metadataLog?: (AgentMetadata | null)[]
  root?: string
  sessionId?: string
}): Promise<{ client: AgentClient; seen: ClientSessionEvent[]; root: string; sessionId: string }> {
  const root = options.root ?? (await mkdtemp(join(tmpdir(), 'demi-subagent-')))
  const hosts = new Map<string, LocalHost>()
  const harness: AgentHarness<Record<string, never>> = {
    name: 'subagent-test',
    initialState: () => ({}),
    host: (ctx) => {
      const existing = hosts.get(ctx.cwd)
      if (existing) return existing
      const host = new LocalHost(ctx.cwd)
      hosts.set(ctx.cwd, host)
      return host
    },
    systemPrompt: (ctx) => {
      options.metadataLog?.push(ctx.metadata)
      return 'parent-system-marker'
    },
    ...(options.agents ? { agents: () => options.agents! } : {}),
  }
  const server = new AgentServer({
    agent: harness,
    providers: [defineProvider({ id: 'stub', displayName: 'stub', createRuntime: () => new StubProvider(options.turns) })],
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
    ...(options.notifyParentOnIdle === undefined ? {} : { subagents: { notifyParentOnIdle: options.notifyParentOnIdle } }),
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))
  const sessionId = options.sessionId ?? globalThis.crypto.randomUUID()
  await client.open(selection, root, sessionId)
  return { client, seen, root, sessionId }
}

function itemsText(request: InferenceRequest): string {
  return JSON.stringify(request.items)
}

function spawnCall(toolUseId: string, script: string, timeoutMs: number): ReturnType<typeof events.toolCall> {
  return events.toolCall(toolUseId, 'shell_exec', { script, timeoutMs })
}

function subagentIdFrom(request: InferenceRequest): string {
  const match = itemsText(request).match(/subagentId: ([A-Za-z0-9_-]+)/)
  if (!match) throw new Error(`no subagentId in tool result: ${itemsText(request)}`)
  return match[1]!
}

test('spawn runs an isolated child whose last assistant text becomes the tool result', async () => {
  let childRequest: InferenceRequest | null = null
  let continuationText = ''
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'Summarize the config file layout' --description sum", 5_000)],
      (request) => {
        childRequest = request
        return [events.text('child result text'), events.response()]
      },
      (request) => {
        continuationText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  // Isolation: the child transcript starts at exactly the task brief.
  expect(childRequest).not.toBeNull()
  expect(childRequest!.items.map((item) => item.type)).toEqual(['user_message'])
  expect(itemsText(childRequest!)).toContain('Summarize the config file layout')
  expect(itemsText(childRequest!)).not.toContain('go')
  expect(itemsText(childRequest!)).toContain('You are a subagent')
  expect(childRequest!.systemPrompt).toContain('parent-system-marker')

  // The spawn tool result carries the id line and the child's last assistant text.
  expect(continuationText).toContain('subagentId:')
  expect(continuationText).toContain('child result text')

  // Protocol frames: started and closed, plus a child transcript stream.
  const lifecycle = seen.filter((event) => event.type === 'subagent')
  expect(lifecycle.map((event) => (event.type === 'subagent' ? event.event : ''))).toEqual(['started', 'closed'])
  const closedFrame = lifecycle[1]
  expect(closedFrame?.type === 'subagent' ? closedFrame.job : null).toMatchObject({
    phase: 'completed',
    result: 'child result text',
    description: 'sum',
  })
  const childId = closedFrame?.type === 'subagent' ? closedFrame.job.subagentId : ''
  expect(seen.some((event) => event.type === 'subagent_transcript_reset' && event.subagentId === childId)).toBe(true)
  expect(seen.some((event) => event.type === 'subagent_transcript_patch' && event.subagentId === childId)).toBe(true)
  await client.close()
})

test('an empty child last assistant text completes with empty output', async () => {
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'silent task'", 5_000)],
      [events.response()],
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  const closedFrame = seen.find((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closedFrame?.type === 'subagent' ? closedFrame.job : null).toMatchObject({ phase: 'completed', result: '' })
  const toolCall = client.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolCall?.type === 'tool_call' ? toolCall.status : '').toBe('completed')
  await client.close()
})

test('an empty prompt fails the spawn command without starting a child', async () => {
  let continuationText = ''
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent ''", 5_000)],
      (request) => {
        continuationText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  expect(continuationText).toContain('prompt must not be empty')
  expect(seen.some((event) => event.type === 'subagent')).toBe(false)
  await client.close()
})

test('a child has no spawn surface: its demi agent tree is send-parent only', async () => {
  let childContinuationText = ''
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'outer task'", 5_000)],
      [events.toolCall('c1', 'shell_exec', { script: "demi agent 'nested task'", timeoutMs: 5_000 })],
      (request) => {
        childContinuationText = itemsText(request)
        return [events.text('child done'), events.response()]
      },
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  // The nested spawn must fail: the child tree only carries send-parent.
  expect(childContinuationText).not.toContain('subagentId:')
  expect(childContinuationText.toLowerCase()).toContain('agent')
  await client.close()
})

test('send-parent reaches a parent blocked in the spawn as a steer at its continuation boundary', async () => {
  let parentContinuation: InferenceRequest | null = null
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'report halfway then finish' --description sp", 5_000)],
      [events.toolCall('c1', 'shell_exec', { script: "demi agent send-parent 'halfway there'", timeoutMs: 5_000 })],
      [events.text('final answer'), events.response()],
      (request) => {
        parentContinuation = request
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  expect(parentContinuation).not.toBeNull()
  const steer = parentContinuation!.items.find((item) => item.type === 'user_steer')
  expect(JSON.stringify(steer ?? '')).toContain('halfway there')
  expect(itemsText(parentContinuation!)).toContain('final answer')
  await client.close()
})

test('a child finishing after the parent went idle wakes it with a user message', async () => {
  let wakeText = ''
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 0.25', timeoutMs: 5_000 })],
      [events.text('spawned, going idle'), events.response()],
      [events.text('bg result'), events.response()],
      (request) => {
        wakeText = itemsText(request)
        return [events.text('acknowledged'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 3_000 },
  )

  expect(wakeText).toContain('completed')
  expect(wakeText).toContain('bg result')
  await client.close()
})

test('the idle wakeup carries the metadata of the round that spawned the child', async () => {
  const metadataLog: (AgentMetadata | null)[] = []
  const { client } = await openHarness({
    metadataLog,
    turns: [
      [spawnCall('t1', "demi agent 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 0.25', timeoutMs: 5_000 })],
      [events.text('spawned, going idle'), events.response()],
      [events.text('bg result'), events.response()],
      [events.text('acknowledged'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }], { metadata: { identityOpenId: 'u-spawner' } })
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 3_000 },
  )

  expect(metadataLog.at(-1)).toEqual({ identityOpenId: 'u-spawner' })
  await client.close()
})

test('notifyParentOnIdle: false leaves the idle parent untouched when a child closes', async () => {
  const { client, seen } = await openHarness({
    notifyParentOnIdle: false,
    turns: [
      [spawnCall('t1', "demi agent 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 0.25', timeoutMs: 5_000 })],
      [events.text('spawned, going idle'), events.response()],
      [events.text('bg result'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => seen.some((event) => event.type === 'subagent' && event.event === 'closed'),
    undefined,
    { timeoutMs: 3_000 },
  )
  await new Promise((resolve) => setTimeout(resolve, 200))

  const lastText = [...client.transcript().blocks].reverse().find((block) => block.type === 'text')
  expect(lastText?.type === 'text' ? lastText.text : '').toBe('spawned, going idle')
  const lastPhase = [...seen].reverse().find((event) => event.type === 'phase')
  expect(lastPhase?.type === 'phase' ? lastPhase.phase : '').toBe('idle')
  await client.close()
})

test('client abortSubagents aborts every live child without touching the parent turn', async () => {
  const { client, seen } = await openHarness({
    notifyParentOnIdle: false,
    turns: [
      [spawnCall('t1', "demi agent 'stuck task' --description stuck", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 5', timeoutMs: 10_000 })],
      [events.text('spawned, going idle'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => seen.some((event) => event.type === 'subagent' && event.event === 'started'),
    undefined,
    { timeoutMs: 3_000 },
  )
  client.abortSubagents()
  await waitFor(
    () => seen.some((event) => event.type === 'subagent' && event.event === 'closed'),
    undefined,
    { timeoutMs: 3_000 },
  )

  const closedFrame = seen.find((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closedFrame?.type === 'subagent' ? closedFrame.job.phase : '').toBe('aborted')
  await client.close()
})

test('demi agent abort tears the child down and fails the pending spawn command', async () => {
  let abortResultText = ''
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'stuck task' --description stuck", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 5', timeoutMs: 10_000 })],
      (request) => {
        const id = subagentIdFrom(request)
        return [events.toolCall('t2', 'shell_exec', { script: `demi agent abort ${id}`, timeoutMs: 5_000 })]
      },
      (request) => {
        abortResultText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 3_000 },
  )

  expect(abortResultText).toContain('aborted')
  const closedFrame = seen.find((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closedFrame?.type === 'subagent' ? closedFrame.job.phase : '').toBe('aborted')
  await client.close()
})

test('list and show expose bounded live snapshots with relative ages; a finished id misses', async () => {
  let inspectionText = ''
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'inspect me' --description insp", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 1', timeoutMs: 10_000 })],
      (request) => {
        const id = subagentIdFrom(request)
        return [
          events.toolCall('t2', 'shell_exec', { script: 'demi agent list', timeoutMs: 5_000 }),
          events.toolCall('t3', 'shell_exec', { script: `demi agent show ${id}`, timeoutMs: 5_000 }),
          events.toolCall('t4', 'shell_exec', { script: 'demi agent show gone-id', timeoutMs: 5_000 }),
          events.toolCall('t5', 'shell_exec', { script: `demi agent abort ${id}`, timeoutMs: 5_000 }),
        ]
      },
      (request) => {
        inspectionText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 3_000 },
  )

  expect(inspectionText).toContain('execution=tool_executing')
  expect(inspectionText).toContain('activity=shell_exec')
  expect(inspectionText).toContain('recent tool calls (last 1):')
  expect(inspectionText).toContain('[executing for ')
  expect(inspectionText).toContain('last assistant text: (none yet)')
  expect(inspectionText).toContain('no running subagent \\"gone-id\\"')
  await client.close()
})

test('a profile systemPrompt replaces the parent prompt; an unknown profile fails the spawn', async () => {
  let childRequest: InferenceRequest | null = null
  let failureText = ''
  const { client } = await openHarness({
    agents: [
      { name: 'explore', description: 'Read-only explorer.', systemPrompt: () => 'explore-system-marker' },
    ],
    turns: [
      [spawnCall('t1', "demi agent 'map the repo' --profile explore", 5_000)],
      (request) => {
        childRequest = request
        return [events.text('explored'), events.response()]
      },
      [events.toolCall('t2', 'shell_exec', { script: "demi agent 'x' --profile nope", timeoutMs: 5_000 })],
      (request) => {
        failureText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'))

  expect(childRequest).not.toBeNull()
  expect(childRequest!.systemPrompt).toContain('explore-system-marker')
  expect(childRequest!.systemPrompt).not.toContain('parent-system-marker')
  expect(failureText).toContain('unknown profile')
  expect(failureText).toContain('explore')
  await client.close()
})

test('closing the parent detaches live children; a reopened parent restores and finishes them', async () => {
  // Phase 1: spawn a child, let it get stuck mid-tool, then tear the connection down.
  const first = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'undying task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'sleep 5', timeoutMs: 10_000 })],
      [events.text('spawned, going idle'), events.response()],
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }], { metadata: { identityOpenId: 'u-spawner' } })
  await waitFor(
    () => first.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'spawned, going idle'),
    undefined,
    { timeoutMs: 3_000 },
  )
  expect(first.seen.some((event) => event.type === 'subagent' && event.event === 'started')).toBe(true)

  await first.client.close()
  // Detach, not close: the child is paused, so no closed frame is emitted.
  expect(first.seen.some((event) => event.type === 'subagent' && event.event === 'closed')).toBe(false)

  // Phase 2: a fresh server process reopens the same session; the child comes
  // back with its metadata, resumes its interrupted turn, and completes; the
  // idle parent gets the wakeup.
  let wakeText = ''
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    turns: [
      [events.text('recovered result'), events.response()],
      (request) => {
        wakeText = itemsText(request)
        return [events.text('acknowledged'), events.response()]
      },
    ],
  })
  // The restore frames arrive after the open ack, so wait for them.
  await waitFor(
    () => second.seen.some((event) => event.type === 'subagent' && event.event === 'started'),
    undefined,
    { timeoutMs: 3_000 },
  )
  const restoredStart = second.seen.find((event) => event.type === 'subagent' && event.event === 'started')
  expect(restoredStart?.type === 'subagent' ? restoredStart.job : null).toMatchObject({
    description: 'bg',
    metadata: { identityOpenId: 'u-spawner' },
  })
  await waitFor(
    () => second.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 3_000 },
  )
  const closedFrame = second.seen.find((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closedFrame?.type === 'subagent' ? closedFrame.job : null).toMatchObject({
    phase: 'completed',
    result: 'recovered result',
  })
  expect(wakeText).toContain('recovered result')
  await second.client.close()
})

test('a finished child is archived: list shows it and resume revives it on top of its old transcript', async () => {
  let childId = ''
  let listText = ''
  let revivedRequest: InferenceRequest | null = null
  let resumeToolText = ''
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'first task' --description arc", 5_000)],
      [events.text('first result'), events.response()],
      (request) => {
        childId = subagentIdFrom(request)
        return [spawnCall('t2', 'demi agent list', 5_000)]
      },
      (request) => {
        listText = itemsText(request)
        return [spawnCall('t3', `demi agent resume ${childId} 'continue the task'`, 5_000)]
      },
      (request) => {
        revivedRequest = request
        return [events.text('second result'), events.response()]
      },
      (request) => {
        resumeToolText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // The archive is visible after the child finished.
  expect(listText).toContain('archived')
  expect(listText).toContain(childId)
  expect(listText).toContain('completed')

  // The revived child continues its own transcript: old brief, old result, new message.
  expect(revivedRequest).not.toBeNull()
  expect(itemsText(revivedRequest!)).toContain('first task')
  expect(itemsText(revivedRequest!)).toContain('first result')
  expect(itemsText(revivedRequest!)).toContain('continue the task')

  // The resume command reports the new result, and the lifecycle ran twice.
  expect(resumeToolText).toContain('second result')
  const lifecycle = seen.filter((event) => event.type === 'subagent').map((event) => (event.type === 'subagent' ? event.event : ''))
  expect(lifecycle).toEqual(['started', 'closed', 'started', 'closed'])
  await client.close()
})

test('a parent restore skips archived children; the archive stays revivable', async () => {
  const first = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent 'finish fast'", 5_000)],
      [events.text('done already'), events.response()],
      [events.text('parent idle'), events.response()],
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => first.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent idle'),
    undefined,
    { timeoutMs: 5_000 },
  )
  expect(first.seen.some((event) => event.type === 'subagent' && event.event === 'closed')).toBe(true)
  await first.client.close()

  let listText = ''
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    turns: [
      [spawnCall('t2', 'demi agent list', 5_000)],
      (request) => {
        listText = itemsText(request)
        return [events.text('checked'), events.response()]
      },
    ],
  })
  await second.client.send([{ type: 'text', text: 'list them' }])
  await waitFor(
    () => second.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'checked'),
    undefined,
    { timeoutMs: 5_000 },
  )
  // No restore fired for the archived child, but it is still listed as revivable.
  expect(second.seen.some((event) => event.type === 'subagent')).toBe(false)
  expect(listText).toContain('archived')
  expect(listText).toContain('completed')
  await second.client.close()
})

test('a readonly Host wrapped over a class-instance Host still reads; mutation and spawn are denied', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-readonly-host-'))
  const inner = new LocalHost(root)
  await inner.fs.writeFile('note.txt', new TextEncoder().encode('readable'), { cwd: root })
  const host = createReadonlyHost(inner)

  // Class instances keep methods on the prototype; the wrapper must delegate, not spread.
  expect(new TextDecoder().decode(await host.fs.readFile('note.txt', { cwd: root }))).toBe('readable')
  expect(await host.fs.exists('note.txt', { cwd: root })).toBe(true)
  expect((await host.fs.stat('note.txt', { cwd: root })).isFile).toBe(true)
  expect(await host.fs.readdir(root)).toContain('note.txt')

  await expect(host.fs.writeFile('evil.txt', new Uint8Array(), { cwd: root })).rejects.toThrow('read-only subagent')
  await expect(host.fs.rm('note.txt', { cwd: root })).rejects.toThrow('read-only subagent')
  await expect(host.process.spawn({ command: 'true', args: [], cwd: root, env: {} })).rejects.toThrow('read-only subagent')

  const artifactPath = `${host.commandArtifactsDir}/probe.txt`
  await host.fs.mkdir(host.commandArtifactsDir, { recursive: true })
  await host.fs.writeFile(artifactPath, new TextEncoder().encode('ok'))
  expect(new TextDecoder().decode(await host.fs.readFile(artifactPath))).toBe('ok')
})

test('the live-children ceiling rejects the spawn beyond MAX_LIVE_SUBAGENTS', async () => {
  const spawns = Array.from({ length: MAX_LIVE_SUBAGENTS }, (_, index) =>
    spawnCall(`t${index + 1}`, `demi agent 'held task ${index + 1}'`, 30),
  )
  const childHold: TurnScript = [events.toolCall('c1', 'shell_exec', { script: 'sleep 5', timeoutMs: 10_000 })]
  let limitText = ''
  const { client } = await openHarness({
    turns: [
      spawns,
      ...Array.from({ length: MAX_LIVE_SUBAGENTS }, () => childHold),
      [events.toolCall('t9', 'shell_exec', { script: "demi agent 'one too many'", timeoutMs: 5_000 })],
      (request) => {
        limitText = itemsText(request)
        return [events.text('parent done'), events.response()]
      },
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  expect(limitText).toContain(`at most ${MAX_LIVE_SUBAGENTS} running subagents`)
  await client.close()
})
