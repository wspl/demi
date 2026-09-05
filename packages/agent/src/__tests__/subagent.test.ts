import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { hostlessShellFactory, probeCommand } from '@demicodes/host-virtual/testing'
import { waitFor } from '@demicodes/utils'
import type { ModelSelection } from '@demicodes/core'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { defineProvider, type InferenceRequest, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import {
  AgentServer,
  MAX_LIVE_SUBAGENTS,
  type AgentClient,
  type AgentHarness,
  type AgentMetadata,
  type ClientSessionEvent,
  type ShellPreviewBudget,
  type SubagentProfile,
} from '../index'
import { ChildSupervisor } from '../subagent/supervisor'
import { MemoryAgentStore } from '../testing'

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
  maxLiveSubagents?: number
  context?: AgentHarness<Record<string, never>>['context']
  metadataLog?: (AgentMetadata | null)[]
  root?: string
  sessionId?: string
  /** The tree store a "previous process" left; a fresh one otherwise. */
  store?: MemoryAgentStore
  shellPreviewBudgetTokens?: ShellPreviewBudget
}): Promise<{ server: AgentServer; client: AgentClient; seen: ClientSessionEvent[]; root: string; sessionId: string; store: MemoryAgentStore }> {
  const root = options.root ?? (await mkdtemp(join(tmpdir(), 'demi-subagent-')))
  const store = options.store ?? new MemoryAgentStore()
  const hosts = new Map<string, LocalHost>()
  const harness: AgentHarness<Record<string, never>> = {
    name: 'subagent-test',
    context: options.context,
    initialState: () => ({}),
    commands: () => [probeCommand()],
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
    store: () => store,
    shellEnvironment: hostlessShellFactory,
    agent: harness,
    providers: [defineProvider({ id: 'stub', displayName: 'stub', createRuntime: () => new StubProvider(options.turns) })],
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
    ...(options.shellPreviewBudgetTokens ? { tools: { shellPreviewBudgetTokens: options.shellPreviewBudgetTokens } } : {}),
    ...(options.notifyParentOnIdle === undefined && options.maxLiveSubagents === undefined
      ? {}
      : {
          subagents: {
            ...(options.notifyParentOnIdle === undefined ? {} : { notifyParentOnIdle: options.notifyParentOnIdle }),
            ...(options.maxLiveSubagents === undefined ? {} : { maxLiveSubagents: options.maxLiveSubagents }),
          },
        }),
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))
  const sessionId = options.sessionId ?? globalThis.crypto.randomUUID()
  await client.open(selection, root, sessionId)
  return { server, client, seen, root, sessionId, store }
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
      [spawnCall('t1', "demi agent spawn 'Summarize the config file layout' --description sum", 5_000)],
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
      [spawnCall('t1', "demi agent spawn 'silent task'", 5_000)],
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
      [spawnCall('t1', "demi agent spawn ''", 5_000)],
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

test('a child spawns a grandchild; the tree links and both close naturally', async () => {
  let grandchildRequest: InferenceRequest | null = null
  const { client, seen, sessionId } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'outer task' --description outer", 10_000)],
      [events.toolCall('c1', 'shell_exec', { script: "demi agent spawn 'inner task' --description inner", timeoutMs: 10_000 })],
      (request) => {
        grandchildRequest = request
        return [events.text('inner result'), events.response()]
      },
      [events.text('outer result'), events.response()],
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // The grandchild is a full subagent: empty transcript, preamble, own task brief.
  expect(grandchildRequest).not.toBeNull()
  expect(grandchildRequest!.items.map((item) => item.type)).toEqual(['user_message'])
  expect(itemsText(grandchildRequest!)).toContain('inner task')
  expect(itemsText(grandchildRequest!)).toContain('You are a subagent')

  // Frames from both depths on the same connection, linked by parentSessionId.
  const started = seen.filter((event) => event.type === 'subagent' && event.event === 'started')
  expect(started).toHaveLength(2)
  const outer = started.find((event) => event.type === 'subagent' && event.job.description === 'outer')
  const inner = started.find((event) => event.type === 'subagent' && event.job.description === 'inner')
  expect(outer?.type === 'subagent' ? outer.job.parentSessionId : '').toBe(sessionId)
  expect(inner?.type === 'subagent' ? inner.job.parentSessionId : '').toBe(
    outer?.type === 'subagent' ? outer.job.subagentId : '',
  )
  const closed = seen.filter((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closed.map((event) => (event.type === 'subagent' ? event.job.description : ''))).toEqual(['inner', 'outer'])
  expect(closed.every((event) => event.type === 'subagent' && event.job.phase === 'completed')).toBe(true)
  await client.close()
})

test('notifyParentOnIdle: false only silences the root level; a mid-tree parent still self-wakes', async () => {
  let childWakeRequest: InferenceRequest | null = null
  let parentContinuationText = ''
  const { client, seen } = await openHarness({
    notifyParentOnIdle: false,
    turns: [
      [spawnCall('t1', "demi agent spawn 'outer task' --description outer", 10_000)],
      [events.toolCall('c1', 'shell_exec', { script: "demi agent spawn 'inner task' --description inner", timeoutMs: 50 })],
      [events.toolCall('g1', 'shell_exec', { script: 'probe hold 300', timeoutMs: 10_000 })],
      [events.text('inner dispatched'), events.response()],
      [events.text('inner result'), events.response()],
      (request) => {
        childWakeRequest = request
        return [events.text('outer integrated'), events.response()]
      },
      (request) => {
        parentContinuationText = itemsText(request)
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

  // The grandchild's completion woke the idle mid-tree child, which integrated
  // the result before closing; the root got it as the spawn tool result.
  expect(childWakeRequest).not.toBeNull()
  expect(itemsText(childWakeRequest!)).toContain('completed')
  expect(itemsText(childWakeRequest!)).toContain('inner result')
  expect(parentContinuationText).toContain('outer integrated')
  const closed = seen.filter((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closed.map((event) => (event.type === 'subagent' ? event.job.description : ''))).toEqual(['inner', 'outer'])
  await client.close()
})

test('aborting a child tears its whole subtree down', async () => {
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'outer task' --description outer", 50)],
      [events.toolCall('c1', 'shell_exec', { script: "demi agent spawn 'inner task' --description inner", timeoutMs: 10_000 })],
      [events.toolCall('g1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })],
      (request) => {
        const id = subagentIdFrom(request)
        return [events.toolCall('t2', 'shell_exec', { script: `demi agent abort ${id}`, timeoutMs: 5_000 })]
      },
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  const closed = seen.filter((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closed).toHaveLength(2)
  expect(closed.every((event) => event.type === 'subagent' && event.job.phase === 'aborted')).toBe(true)
  await client.close()
})

test('steer chimes into a running child turn; the parent steer materializes at the continuation', async () => {
  let childContinuation: InferenceRequest | null = null
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'slow task' --description slow", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 400', timeoutMs: 10_000 })],
      (request) => {
        const id = subagentIdFrom(request)
        return [events.toolCall('t2', 'shell_exec', { script: `demi agent steer ${id} 'course correction'`, timeoutMs: 5_000 })]
      },
      [events.text('parent idle'), events.response()],
      (request) => {
        childContinuation = request
        return [events.text('steered fine'), events.response()]
      },
      [events.text('acknowledged'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // The message joined the child's running turn as a steer, not a new turn.
  expect(childContinuation).not.toBeNull()
  const steer = childContinuation!.items.find((item) => item.type === 'user_steer')
  expect(JSON.stringify(steer ?? '')).toContain('course correction')
  expect(JSON.stringify(steer ?? '')).toContain('[agent ')
  await client.close()
})

test('send is a mailbox: a queued message opens a new child turn instead of closing the session', async () => {
  let secondTurnRequest: InferenceRequest | null = null
  const { client, seen, sessionId } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'bg task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 400', timeoutMs: 10_000 })],
      (request) => {
        const id = subagentIdFrom(request)
        return [events.toolCall('t2', 'shell_exec', { script: `demi agent send ${id} 'extra instruction'`, timeoutMs: 5_000 })]
      },
      [events.text('parent idle'), events.response()],
      [events.text('first phase'), events.response()],
      (request) => {
        secondTurnRequest = request
        return [events.text('second phase'), events.response()]
      },
      [events.text('acknowledged'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // The mailbox message arrived as a fresh user turn after the first turn ended,
  // prefixed with the sender identity (the root session).
  expect(secondTurnRequest).not.toBeNull()
  expect(itemsText(secondTurnRequest!)).toContain('extra instruction')
  expect(itemsText(secondTurnRequest!)).toContain(`[agent ${sessionId}`)
  const closedFrame = seen.find((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closedFrame?.type === 'subagent' ? closedFrame.job.result : '').toBe('second phase')
  await client.close()
})

test('steering an idle root fails; send parent wakes it as a user turn', async () => {
  let steerFailureText = ''
  let wakeRequest: InferenceRequest | null = null
  // The parent wake and the child's own continuation race after the send, so
  // these two entries dispatch on request identity instead of arrival order.
  const afterSend: TurnScript = (request) => {
    if (itemsText(request).includes('You are a subagent')) return [events.text('child done'), events.response()]
    wakeRequest = request
    return [events.text('got it'), events.response()]
  }
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'report home' --description rep", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 300', timeoutMs: 10_000 })],
      [events.text('parent idle'), events.response()],
      [events.toolCall('c2', 'shell_exec', { script: "demi agent steer parent 'ping'", timeoutMs: 5_000 })],
      (request) => {
        steerFailureText = itemsText(request)
        return [events.toolCall('c3', 'shell_exec', { script: "demi agent send parent 'ping via mail'", timeoutMs: 5_000 })]
      },
      afterSend,
      afterSend,
      [events.text('acknowledged'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'acknowledged'),
    undefined,
    { timeoutMs: 5_000 },
  )

  expect(steerFailureText).toContain('no running turn to steer')
  expect(wakeRequest).not.toBeNull()
  expect(itemsText(wakeRequest!)).toContain('ping via mail')
  expect(itemsText(wakeRequest!)).toContain('[agent ')
  await client.close()
})

test('a sibling shows and messages another sibling through the directory', async () => {
  let showText = ''
  let siblingMessageRequest: InferenceRequest | null = null
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'hold the fort' --description holder", 50)],
      [events.toolCall('a1', 'shell_exec', { script: 'probe hold 500', timeoutMs: 10_000 })],
      (request) => {
        const holderId = subagentIdFrom(request)
        return [spawnCall('t2', `demi agent spawn 'message agent ${holderId} then finish' --description messenger`, 10_000)]
      },
      (request) => {
        const targetId = itemsText(request).match(/message agent ([A-Za-z0-9_-]+)/)![1]!
        return [
          events.toolCall('b1', 'shell_exec', { script: `demi agent show ${targetId}`, timeoutMs: 5_000 }),
          events.toolCall('b2', 'shell_exec', { script: `demi agent send ${targetId} 'hello sibling'`, timeoutMs: 5_000 }),
        ]
      },
      (request) => {
        showText = itemsText(request)
        return [events.text('messenger done'), events.response()]
      },
      [events.text('parent idle'), events.response()],
      [events.text('holder first'), events.response()],
      (request) => {
        siblingMessageRequest = request
        return [events.text('holder second'), events.response()]
      },
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // show on a sibling works and reports its live execution state.
  expect(showText).toContain('execution: tool_executing')
  expect(showText).toContain('sent to ')
  // The sibling's message arrived as a new turn with the sender identity.
  expect(siblingMessageRequest).not.toBeNull()
  expect(itemsText(siblingMessageRequest!)).toContain('hello sibling')
  expect(itemsText(siblingMessageRequest!)).toContain('messenger')
  await client.close()
})

test('lifecycle authority: send and abort reject an archived child; only resume revives it', async () => {
  let sendFailureText = ''
  let abortFailureText = ''
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'quick task' --description q", 5_000)],
      [events.text('done already'), events.response()],
      (request) => {
        const id = subagentIdFrom(request)
        return [events.toolCall('t2', 'shell_exec', { script: `demi agent send ${id} 'too late'`, timeoutMs: 5_000 })]
      },
      (request) => {
        sendFailureText = itemsText(request)
        const id = subagentIdFrom(request)
        return [events.toolCall('t3', 'shell_exec', { script: `demi agent abort ${id}`, timeoutMs: 5_000 })]
      },
      (request) => {
        abortFailureText = itemsText(request)
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

  expect(sendFailureText).toContain('no live agent')
  expect(sendFailureText).toContain('resume')
  expect(abortFailureText).toContain('not one of your running children')
  await client.close()
})

test('--no-subagents forbids the child from spawning while communication and reads remain', async () => {
  let nestedFailText = ''
  let listText = ''
  const { client, seen } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'restricted task' --no-subagents --description r", 10_000)],
      [events.toolCall('n1', 'shell_exec', { script: "demi agent spawn 'nested task'", timeoutMs: 5_000 })],
      (request) => {
        nestedFailText = itemsText(request)
        return [events.toolCall('n2', 'shell_exec', { script: 'demi agent list', timeoutMs: 5_000 })]
      },
      (request) => {
        listText = itemsText(request)
        return [events.text('restricted done'), events.response()]
      },
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  // The nested spawn never started a session; the tree still reads fine.
  expect(nestedFailText).not.toContain('subagentId:')
  expect(seen.filter((event) => event.type === 'subagent' && event.event === 'started')).toHaveLength(1)
  expect(listText).toContain('← you')
  await client.close()
})

test('a profile with canSpawnSubagents: false pins its children to communication only', async () => {
  let nestedFailText = ''
  const { client, seen } = await openHarness({
    agents: [{ name: 'worker', description: 'No delegation.', canSpawnSubagents: false }],
    turns: [
      [spawnCall('t1', "demi agent spawn 'leaf task' --profile worker", 10_000)],
      [events.toolCall('n1', 'shell_exec', { script: "demi agent spawn 'nested task'", timeoutMs: 5_000 })],
      (request) => {
        nestedFailText = itemsText(request)
        return [events.text('leaf done'), events.response()]
      },
      [events.text('parent done'), events.response()],
    ],
  })

  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent done'),
    undefined,
    { timeoutMs: 5_000 },
  )

  expect(nestedFailText).not.toContain('subagentId:')
  expect(seen.filter((event) => event.type === 'subagent' && event.event === 'started')).toHaveLength(1)
  await client.close()
})

test('a child finishing after the parent went idle wakes it with a user message', async () => {
  let wakeText = ''
  const { client } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 250', timeoutMs: 5_000 })],
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
      [spawnCall('t1', "demi agent spawn 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 250', timeoutMs: 5_000 })],
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
      [spawnCall('t1', "demi agent spawn 'long background task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 250', timeoutMs: 5_000 })],
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
      [spawnCall('t1', "demi agent spawn 'stuck task' --description stuck", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })],
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
      [spawnCall('t1', "demi agent spawn 'stuck task' --description stuck", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })],
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

test('list renders the tree with a self marker; show exposes a bounded snapshot; a finished id misses', async () => {
  let inspectionText = ''
  const { client, sessionId } = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'inspect me' --description insp", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 1000', timeoutMs: 10_000 })],
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

  // The tree: root marked as the caller, the live child rendered beneath it.
  expect(inspectionText).toContain(`${sessionId}  (root session) ← you`)
  expect(inspectionText).toContain('└─●')
  expect(inspectionText).toContain('execution=tool_executing')
  expect(inspectionText).toContain('activity=shell_exec')
  // show: bounded live snapshot with relative ages.
  expect(inspectionText).toContain('recent tool calls (last 1):')
  expect(inspectionText).toContain('[executing for ')
  expect(inspectionText).toContain('last assistant text: (none yet)')
  expect(inspectionText).toContain('no live agent \\"gone-id\\"')
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
      [spawnCall('t1', "demi agent spawn 'map the repo' --profile explore", 5_000)],
      (request) => {
        childRequest = request
        return [events.text('explored'), events.response()]
      },
      [events.toolCall('t2', 'shell_exec', { script: "demi agent spawn 'x' --profile nope", timeoutMs: 5_000 })],
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
      [spawnCall('t1', "demi agent spawn 'undying task' --description bg", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })],
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
    store: first.store,
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
      [spawnCall('t1', "demi agent spawn 'first task' --description arc", 5_000)],
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

  // The archive renders in the tree beneath its parent.
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
      [spawnCall('t1', "demi agent spawn 'finish fast'", 5_000)],
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
    store: first.store,
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

test('resuming an archived child whose profile is gone fails without orphaning the archive', async () => {
  let childId = ''
  const first = await openHarness({
    agents: [{ name: 'old', description: 'old profile' }],
    turns: [
      [spawnCall('t1', "demi agent spawn 'finish fast' --profile old", 5_000)],
      [events.text('done already'), events.response()],
      (request) => {
        childId = subagentIdFrom(request)
        return [events.text('parent idle'), events.response()]
      },
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => first.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent idle'),
    undefined,
    { timeoutMs: 5_000 },
  )
  await first.client.close()

  let resumeText = ''
  let listText = ''
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    store: first.store,
    agents: [{ name: 'new', description: 'replacement profile' }],
    turns: [
      [spawnCall('t2', `demi agent resume ${childId} 'again'`, 5_000)],
      (request) => {
        resumeText = itemsText(request)
        return [spawnCall('t3', 'demi agent list', 5_000)]
      },
      (request) => {
        listText = itemsText(request)
        return [events.text('checked'), events.response()]
      },
    ],
  })
  await second.client.send([{ type: 'text', text: 'revive it' }])
  await waitFor(
    () => second.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'checked'),
    undefined,
    { timeoutMs: 5_000 },
  )
  expect(resumeText).toContain('unknown profile')
  expect(second.seen.some((event) => event.type === 'subagent')).toBe(false)
  // The failed resume must not have rewritten the archive into a live record.
  expect(listText).toContain('archived')
  expect(listText).toContain(childId)
  await second.client.close()
})

test('omitting --profile inherits even with declared profiles; "default" is not a profile name', async () => {
  let listText = ''
  let badProfileText = ''
  const { client } = await openHarness({
    agents: [{ name: 'worker', description: 'declared profile' }],
    turns: [
      [spawnCall('t1', "demi agent spawn 'inherit me'", 5_000)],
      [events.text('child done'), events.response()],
      [spawnCall('t2', 'demi agent list', 5_000)],
      (request) => {
        listText = itemsText(request)
        return [spawnCall('t3', "demi agent spawn 'nope' --profile default", 5_000)]
      },
      (request) => {
        badProfileText = itemsText(request)
        return [events.text('checked'), events.response()]
      },
    ],
  })
  await client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'checked'),
    undefined,
    { timeoutMs: 5_000 },
  )
  expect(listText).toContain('archived (completed')
  expect(badProfileText).toContain('unknown profile')
  expect(badProfileText).toContain('available: worker')
  await client.close()
})

test('a harness may not declare a profile named "default"', () => {
  expect(
    () =>
      new ChildSupervisor({
        tree: { profiles: [{ name: 'default', description: 'reserved' }] },
      } as unknown as ConstructorParameters<typeof ChildSupervisor>[0]),
  ).toThrow('reserved')
})

test('the live-children ceiling rejects the spawn beyond MAX_LIVE_SUBAGENTS', async () => {
  const spawns = Array.from({ length: MAX_LIVE_SUBAGENTS }, (_, index) =>
    spawnCall(`t${index + 1}`, `demi agent spawn 'held task ${index + 1}'`, 30),
  )
  const childHold: TurnScript = [events.toolCall('c1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })]
  let limitText = ''
  const { client } = await openHarness({
    turns: [
      spawns,
      ...Array.from({ length: MAX_LIVE_SUBAGENTS }, () => childHold),
      [events.toolCall('t9', 'shell_exec', { script: "demi agent spawn 'one too many'", timeoutMs: 5_000 })],
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

test('the ceiling is configurable per server via subagents.maxLiveSubagents', async () => {
  let limitText = ''
  const { client } = await openHarness({
    maxLiveSubagents: 1,
    notifyParentOnIdle: false,
    turns: [
      [
        spawnCall('t1', "demi agent spawn 'first'", 30),
        spawnCall('t2', "demi agent spawn 'second'", 5_000),
      ],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 1000', timeoutMs: 10_000 })],
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

  expect(limitText).toContain('at most 1 running subagents')
  await client.close()
})

test('unnamed grandchildren inherit the parent profile prompt and model, keep the agent tree after command filtering, and use the configured preview budget', async () => {
  let grandchildRequest: InferenceRequest | null = null
  const contextWindows: number[] = []
  const inheritedModel = { ...model, model: { ...model.model, id: 'worker-model', contextWindow: 200_000 } }
  const { client, seen } = await openHarness({
    agents: [{
      name: 'worker',
      description: 'A worker with an inherited setup.',
      systemPrompt: (ctx) => `worker-system-marker\n${ctx.commandsPrompt}`,
      model: inheritedModel,
      commands: () => [],
    }],
    shellPreviewBudgetTokens: (contextWindow) => {
      contextWindows.push(contextWindow)
      return 25
    },
    turns: [
      [spawnCall('root-spawn', "demi agent spawn 'outer task' --profile worker", 5_000)],
      [spawnCall('child-spawn', "demi agent spawn 'inner task'", 5_000)],
      (request) => {
        grandchildRequest = request
        return [spawnCall('grandchild-list', 'demi agent list', 5_000)]
      },
      [events.text('inner result'), events.response()],
      [events.text('outer result'), events.response()],
      [events.text('parent done'), events.response()],
    ],
  })
  await client.send([{ type: 'text', text: 'go' }])
  expect(grandchildRequest).not.toBeNull()
  expect(grandchildRequest!.systemPrompt).toContain('worker-system-marker')
  expect(grandchildRequest!.systemPrompt).not.toContain('parent-system-marker')
  expect(grandchildRequest!.systemPrompt).not.toContain('Test probes')
  expect(grandchildRequest!.modelId).toBe('worker-model')
  expect(seen.filter((event) => event.type === 'subagent' && event.event === 'started')).toHaveLength(2)
  expect(contextWindows).toEqual([200_000, 200_000, 100_000])
  await client.close()
})

test('an archived --no-subagents child stays restricted after reopening and resume', async () => {
  let childId = ''
  const first = await openHarness({
    turns: [
      [spawnCall('first-spawn', "demi agent spawn 'restricted task' --no-subagents", 5_000)],
      [events.text('first result'), events.response()],
      (request) => {
        childId = subagentIdFrom(request)
        return [events.text('parent idle'), events.response()]
      },
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await first.client.close()
  let nestedFailure = ''
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    store: first.store,
    turns: [
      [spawnCall('resume', `demi agent resume ${childId} 'try delegating'`, 5_000)],
      [spawnCall('nested-spawn', "demi agent spawn 'forbidden task'", 5_000)],
      (request) => {
        nestedFailure = itemsText(request)
        return [events.text('still restricted'), events.response()]
      },
      [events.text('parent done'), events.response()],
    ],
  })
  await second.client.send([{ type: 'text', text: 'continue' }])
  expect(nestedFailure).toContain('Unknown subcommand')
  expect(second.seen.filter((event) => event.type === 'subagent' && event.event === 'started')).toHaveLength(1)
  await second.client.close()
})

test('a reopened parent restores nested child checkpoints before the outer child can settle', async () => {
  const first = await openHarness({
    turns: [
      [spawnCall('outer-spawn', "demi agent spawn 'outer task' --description outer", 50)],
      [spawnCall('inner-spawn', "demi agent spawn 'inner task' --description inner", 10_000)],
      [spawnCall('inner-hold', 'probe hold 5000', 10_000)],
      [events.text('parent idle'), events.response()],
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await first.client.close()
  expect(first.seen.filter((event) => event.type === 'subagent' && event.event === 'started')).toHaveLength(2)
  expect(first.seen.some((event) => event.type === 'subagent' && event.event === 'closed')).toBe(false)
  const restoredRequests: InferenceRequest[] = []
  const finish: TurnScript = (request) => {
    restoredRequests.push(request)
    return [events.text('restored result'), events.response()]
  }
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    store: first.store,
    turns: Array.from({ length: 6 }, () => finish),
  })
  await waitFor(
    () => second.seen.filter((event) => event.type === 'subagent' && event.event === 'closed').length === 2,
    undefined,
    { timeoutMs: 3_000 },
  )
  const started = second.seen.filter((event) => event.type === 'subagent' && event.event === 'started')
  expect(started.map((event) => event.type === 'subagent' ? event.job.description : '')).toEqual(['outer', 'inner'])
  const closed = second.seen.filter((event) => event.type === 'subagent' && event.event === 'closed')
  expect(closed.map((event) => event.type === 'subagent' ? event.job.description : '')).toEqual(['inner', 'outer'])
  expect(closed.every((event) => event.type === 'subagent' && event.job.phase === 'completed')).toBe(true)
  expect(restoredRequests.some((request) => itemsText(request).includes('inner task') && itemsText(request).includes('probe hold 5000'))).toBe(true)
  await second.client.close()
})

// The three commits across a restart (`docs/subagent.md` § Persistence): what a
// process loses between them is repaired by the next open, never twice. The
// store is mutated after the close so the dispose flush does not overwrite it.

function childNodeOf(store: MemoryAgentStore, parentId: string, description: string) {
  const node = [...store.nodes.values()].find((stored) => stored.record.parentId === parentId && stored.record.description === description)
  if (!node) throw new Error(`no child "${description}" under ${parentId}`)
  return node
}

test('a child lost before its first save still has its brief: the create commit queued it', async () => {
  const first = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'do the thing' --description brief", 50)],
      [events.toolCall('c1', 'shell_exec', { script: 'probe hold 5000', timeoutMs: 10_000 })],
      [events.text('spawned, going idle'), events.response()],
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => first.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'spawned, going idle'),
    undefined,
    { timeoutMs: 3_000 },
  )
  await first.client.close()
  // What the store holds when the process ends between the create commit and the
  // child's first save: the node row with its brief queued, no block rows.
  const child = childNodeOf(first.store, first.sessionId, 'brief')
  child.blocks.clear()
  child.blockCount = 0
  child.state = { ...child.state, phase: 'idle', queue: [{ id: 'brief', text: 'do the thing', content: [{ type: 'text', text: 'do the thing' }] }] }

  let childRequest: InferenceRequest | null = null
  let wakeText = ''
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    store: first.store,
    turns: [
      (request) => {
        childRequest = request
        return [events.text('done from the brief'), events.response()]
      },
      (request) => {
        wakeText = itemsText(request)
        return [events.text('acknowledged'), events.response()]
      },
    ],
  })
  await waitFor(() => wakeText.includes('done from the brief'), undefined, { timeoutMs: 5_000 })
  expect(itemsText(childRequest!)).toContain('do the thing')
  await waitFor(() => child.record.delivered, undefined, { timeoutMs: 3_000 })
  expect(child.record.closedPhase).toBe('completed')
  await second.client.close()
})

test('a close whose wakeup was never committed is delivered once at restore; a quiescent live child closes there', async () => {
  const first = await openHarness({
    turns: [
      [spawnCall('t1', "demi agent spawn 'first' --description one", 50), spawnCall('t2', "demi agent spawn 'second' --description two", 50)],
      [events.text('one done'), events.response()],
      [events.text('two done'), events.response()],
      [events.text('parent idle'), events.response()],
    ],
  })
  await first.client.send([{ type: 'text', text: 'go' }])
  await waitFor(
    () => first.client.transcript().blocks.some((block) => block.type === 'text' && block.text === 'parent idle'),
    undefined,
    { timeoutMs: 5_000 },
  )
  await first.client.close()
  const one = childNodeOf(first.store, first.sessionId, 'one')
  const two = childNodeOf(first.store, first.sessionId, 'two')
  expect([one.record.closedPhase, two.record.closedPhase]).toEqual(['completed', 'completed'])
  // What the store holds when the process ends between the closes and the parent's
  // checkpoint: `one` closed but its completion never taken, `two` quiescent with its
  // final checkpoint written and its close row not yet, the parent's rows carrying neither.
  one.record.delivered = false
  two.record = { ...two.record, closedPhase: null, closedAt: null, result: null, delivered: false }
  two.state = { ...two.state, phase: 'idle', queue: [] }
  const root = first.store.nodes.get(first.sessionId)!
  root.blocks.clear()
  root.blockCount = 0
  root.state = { ...root.state, phase: 'idle', queue: [] }

  const wakeups: string[] = []
  const second = await openHarness({
    root: first.root,
    sessionId: first.sessionId,
    store: first.store,
    turns: [
      (request) => {
        wakeups.push(itemsText(request))
        return [events.text('ack'), events.response()]
      },
      (request) => {
        wakeups.push(itemsText(request))
        return [events.text('ack'), events.response()]
      },
    ],
  })
  await waitFor(() => wakeups.length === 2, undefined, { timeoutMs: 5_000 })
  expect(wakeups.some((text) => text.includes('one done'))).toBe(true)
  expect(wakeups.some((text) => text.includes('two done'))).toBe(true)
  expect(two.record).toMatchObject({ closedPhase: 'completed', result: 'two done' })
  await waitFor(() => one.record.delivered && two.record.delivered, undefined, { timeoutMs: 3_000 })
  await second.client.close()

  // Reopening once more delivers nothing again: both completions are in the parent's checkpoint.
  const third = await openHarness({ root: first.root, sessionId: first.sessionId, store: first.store, turns: [] })
  await new Promise((resolve) => setTimeout(resolve, 200))
  expect(third.client.transcript().blocks.filter((block) => block.type === 'user')).toHaveLength(2)
  expect(third.seen.some((event) => event.type === 'error')).toBe(false)
  await third.client.close()
})

test('an idle root cannot reserve its running child tree; custom profiles retain product context', async () => {
  const nodes = new Map<string, string>()
  const { client, server, seen, sessionId } = await openHarness({
    notifyParentOnIdle: false,
    agents: [{ name: 'custom', description: 'custom prompt', systemPrompt: () => 'custom system' }],
    context: ctx => {
      nodes.set(ctx.agentSessionId, ctx.rootSessionId)
      return ctx.transcript.blocks.some(block => block.type === 'user' && block.preamble === 'execution snapshot') ? null : 'execution snapshot'
    },
    turns: [
      [spawnCall('spawn', "demi agent spawn 'wait' --profile custom", 30)],
      [events.toolCall('hold', 'shell_exec', { script: 'probe hold 300', timeoutMs: 5_000 })],
      [events.text('parent idle'), events.response()],
      request => {
        expect(request.systemPrompt).toContain('custom system')
        expect(itemsText(request)).toContain('execution snapshot')
        return [events.text('child done'), events.response()]
      },
    ],
  })
  await client.send([{ type: 'text', text: 'go' }])
  expect(server.sessionPhase(sessionId)).toBe('idle')
  expect(server.treeActive(sessionId)).toBe(true)
  expect(server.reserveTreeMutation(sessionId)).toBeNull()
  await waitFor(() => seen.some(event => event.type === 'subagent' && event.event === 'closed'))
  await waitFor(() => !server.treeActive(sessionId))
  expect(nodes.size).toBe(2)
  expect([...nodes.values()]).toEqual([sessionId, sessionId])
  server.reserveTreeMutation(sessionId)!()
  await client.close()
})
