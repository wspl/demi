import { afterAll, beforeAll, expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { createPinia, setActivePinia } from 'pinia'
import type { Block, ClientContent, ModelSelection } from '@demicodes/protocol'
import { AgentSocketError, connectAgentClient } from '@demicodes/web-ui/transport/agent-socket'
import type { ClientSessionEvent } from '@demicodes/web-ui/transport/protocol'
import { ApiError, apiRequest, jsonBody, readResponse } from '../../api/client'
import {
  claimedDeviceSchema,
  conversationAnswerSchema,
  conversationsSchema,
  conversationUpdateSchema,
  devicesSchema,
  modelCatalogSchema,
  providerAnswerSchema,
  transcriptSchema,
  type Claim,
  type ConversationPatch,
  type CreateConversation,
  type CreateProvider,
  type SetupRequest,
} from '../../api/generated/web-api'
import { productStateSchema } from '../../api/unported'
import { useSession } from '../../auth/session'
import { openBrowser, startBackend, startRunner, temporaryRoot, type Backend, type Runner } from './harness'
import { startScriptedAnthropic, type ScriptedAnthropic } from './scripted-anthropic'

// The browser-contract suite (`scenarios.md` § Browser-contract suite): the
// backend executable, driven the way the page drives it, through the web
// application's API client and the conversation socket's `AgentClient`,
// which validate every answer and frame with the generated schemas. The
// model is a scripted Anthropic-compatible endpoint, and tools run on a real
// runner.

const EMAIL = 'master@example.test'
const PASSWORD = 'contract-password'
/** The one model of the scripted entry, as a manual list states it. */
const MODEL_ID = 'claude-scripted'
/** How long a conversation may take to settle after its last frame. */
const SETTLE_MS = 10_000

let root: Awaited<ReturnType<typeof temporaryRoot>>
let vendor: ScriptedAnthropic
let backend: Backend
let browser: ReturnType<typeof openBrowser>
const runners: Runner[] = []

beforeAll(async () => {
  root = await temporaryRoot()
  vendor = startScriptedAnthropic()
  backend = await startBackend(root.path)
  browser = openBrowser(backend.origin)
  setActivePinia(createPinia())
  // The instance's first account, signed in by its setup.
  await apiRequest('/setup', { method: 'POST', ...jsonBody({ email: EMAIL, password: PASSWORD } satisfies SetupRequest) })
})

afterAll(async () => {
  browser?.restore()
  for (const runner of runners) {
    await runner.stop()
  }
  await backend?.stop()
  await vendor?.stop()
  await root?.remove()
})

/** The scripted entry's model as the page selects it: the catalog's selection, which the backend built. */
let scripted: Promise<ModelSelection> | undefined
function scriptedModel(): Promise<ModelSelection> {
  return scripted ??= (async () => {
    const entry: CreateProvider = {
      source: 'custom',
      providerType: 'anthropic',
      label: 'Scripted',
      apiKey: 'sk-ant-contract',
      baseUrl: vendor.url,
      models: [{
        id: MODEL_ID, displayName: 'Scripted', contextWindow: 200_000, outputLimit: null,
        thinkingEfforts: [], acceptedExtensions: null, fastTier: null,
      }],
    }
    const { provider } = await readResponse(await apiRequest('/providers', { method: 'POST', ...jsonBody(entry) }), providerAnswerSchema)
    const catalog = await readResponse(await apiRequest('/models'), modelCatalogSchema)
    const model = catalog.providers
      .find((candidate) => candidate.providerId === provider.id)
      ?.models.find((candidate) => candidate.id === MODEL_ID)
    if (!model) {
      throw new Error(`The catalog has no ${MODEL_ID}: ${JSON.stringify(catalog)}`)
    }
    return model.selection
  })()
}

/** A new conversation under the id the page chose, set to the scripted model as the page's first send sets it. */
async function createConversation(model?: ModelSelection): Promise<string> {
  const id = crypto.randomUUID()
  await readResponse(
    await apiRequest('/conversations', { method: 'POST', ...jsonBody({ id } satisfies CreateConversation) }),
    conversationAnswerSchema,
  )
  if (model) {
    await patchConversation(id, { model: { providerId: model.providerId, modelId: model.model.id } })
  }
  return id
}

async function patchConversation(id: string, patch: ConversationPatch): Promise<void> {
  const update = await readResponse(
    await apiRequest(`/conversations/${id}`, { method: 'PATCH', ...jsonBody(patch) }),
    conversationUpdateSchema,
  )
  expect(update.results.filter((result) => result.status !== 'applied')).toEqual([])
}

/** The conversation's socket, as the page connects it. */
function connect(id: string) {
  return connectAgentClient(browser.socketUrl(`/conversations/${id}/stream`))
}

/** The transcript the backend serves cold, from the conversation's database. */
async function coldTranscript(id: string) {
  return readResponse(await apiRequest(`/conversations/${id}/transcript`), transcriptSchema)
}

/**
 * Waits until the conversation no longer runs. The save that ends an action
 * follows the idle phase the socket shows, and the transcript route reads
 * what the save wrote.
 */
async function settled(id: string): Promise<void> {
  const deadline = Date.now() + SETTLE_MS
  for (;;) {
    const { conversations } = await readResponse(await apiRequest('/conversations'), conversationsSchema)
    const summary = conversations.find((conversation) => conversation.id === id)
    if (summary && summary.status !== 'running' && summary.status !== 'compacting') {
      return
    }
    if (Date.now() > deadline) {
      throw new Error(`The conversation still runs after ${SETTLE_MS} ms: ${JSON.stringify(summary)}`)
    }
    await Bun.sleep(50)
  }
}

/** A device paired as the page pairs one: its runner prints a code, and the signed-in page claims it. */
let paired: Promise<{ deviceId: string; runner: Runner }> | undefined
function pairedDevice() {
  return paired ??= (async () => {
    const runner = await startRunner(root.path, backend.origin, 'laptop')
    runners.push(runner)
    const { device } = await readResponse(
      await apiRequest('/devices/claim', { method: 'POST', ...jsonBody({ code: await runner.code } satisfies Claim) }),
      claimedDeviceSchema,
    )
    const deadline = Date.now() + SETTLE_MS
    for (;;) {
      const { devices } = await readResponse(await apiRequest('/devices'), devicesSchema)
      if (devices.some((each) => each.id === device.id && each.online)) {
        return { deviceId: device.id, runner }
      }
      if (Date.now() > deadline) {
        throw new Error(`The device did not come online:\n${runner.log()}`)
      }
      await Bun.sleep(50)
    }
  })()
}

/** A conversation whose work runs in a new `work` directory of the paired device, as a target switch leaves it. */
async function conversationOnDevice(model: ModelSelection): Promise<{ id: string; work: string }> {
  const { deviceId, runner } = await pairedDevice()
  const work = join(runner.home, `work-${crypto.randomUUID()}`)
  await Bun.write(join(work, '.keep'), '')
  const id = await createConversation(model)
  await patchConversation(id, { target: { kind: 'device', deviceId, path: work } })
  return { id, work }
}

function text(value: string): ClientContent {
  return { type: 'text', text: value }
}

function kinds(blocks: readonly Block[]): string[] {
  return blocks.map((block) => block.type)
}

test('a user signs in through the API client, and the session admits the account snapshot and the conversation socket', async () => {
  const session = useSession()
  const id = await createConversation()
  await session.signOut()
  expect(browser.signedIn()).toBe(false)
  // Signed out, the page reaches neither the API nor the socket.
  const refused = await apiRequest('/conversations').catch((error: unknown) => error)
  expect(refused).toBeInstanceOf(ApiError)
  expect(refused).toMatchObject({ status: 401, code: 'unauthenticated' })
  expect(await connect(id).catch((error: unknown) => error)).toBeInstanceOf(AgentSocketError)

  await session.signIn(EMAIL, PASSWORD, new AbortController().signal)
  expect(session.user?.email).toBe(EMAIL)
  // The account snapshot the page renders; creating a conversation does not
  // make the user's Cloud, so it is not made yet.
  const state = await readResponse(await apiRequest('/state'), productStateSchema)
  expect(state.user.email).toBe(EMAIL)
  expect(state.cloud).toMatchObject({ device: null, state: 'unallocated' })
  const client = await connect(id)
  try {
    const events: ClientSessionEvent['type'][] = []
    client.subscribe((event) => events.push(event.type))
    await client.open(await scriptedModel())
    expect(events[0]).toBe('opened')
  } finally {
    client.disconnect()
  }
})

test('a conversation created through the API runs a turn, and the client receives the reply and the end of the turn', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  vendor.reply({ type: 'text', text: 'Hello from the script.' })
  const client = await connect(id)
  try {
    const phases: string[] = []
    client.subscribe((event) => {
      if (event.type === 'phase') {
        phases.push(event.phase)
      }
    })
    await client.open(model)
    // The send resolves when the action it started ends.
    await client.send([text('Say hello')])
    expect(phases).toEqual(['idle', 'running', 'idle'])
    const blocks = client.transcript().blocks
    expect(kinds(blocks)).toEqual(['user', 'text', 'response'])
    expect(blocks[1]).toMatchObject({ type: 'text', text: 'Hello from the script.' })
    const request = vendor.turns().at(-1)
    expect(request?.model).toBe(MODEL_ID)
    expect(JSON.stringify(request?.messages)).toContain('Say hello')
  } finally {
    client.disconnect()
  }
})

test('a scripted tool call runs on the real runner, and its result appears in the transcript', async () => {
  const model = await scriptedModel()
  const { id, work } = await conversationOnDevice(model)
  vendor.reply({
    type: 'tool_use',
    name: 'shell_exec',
    input: { script: 'printf contract > probe.txt && cat probe.txt', description: 'Probe file written', timeoutMs: 30_000 },
  })
  vendor.reply({ type: 'text', text: 'The probe says contract.' })
  const client = await connect(id)
  try {
    await client.open(model)
    await client.send([text('Write the probe')])
    const blocks = client.transcript().blocks
    // The switch to the device reaches the model as the context of its next request.
    expect(kinds(blocks)).toEqual(['user', 'context', 'tool_call', 'response', 'text', 'response'])
    const call = blocks[2]
    if (call?.type !== 'tool_call') {
      throw new Error(`No tool call: ${JSON.stringify(blocks)}`)
    }
    expect(call).toMatchObject({ toolName: 'shell_exec', status: 'completed', view: { kind: 'shell', status: 'exited', exitCode: 0 } })
    expect(JSON.stringify(call.output)).toContain('contract')
    // The command ran on the device, in the conversation's directory.
    expect(await readFile(join(work, 'probe.txt'), 'utf8')).toBe('contract')
    // The model read the tool's result in its next request.
    expect(JSON.stringify(vendor.turns().at(-1)?.messages)).toContain('tool_result')
  } finally {
    client.disconnect()
  }
})

test('after a reload, the transcript the client assembled from live patches equals the cold transcript', async () => {
  const model = await scriptedModel()
  const { id } = await conversationOnDevice(model)
  vendor.reply({ type: 'tool_use', name: 'shell_exec', input: { script: 'echo first', timeoutMs: 30_000 } })
  vendor.reply({ type: 'text', text: 'The first turn ran a command.' })
  vendor.reply({ type: 'text', text: 'And the second turn answered at once.' })
  const live = await connect(id)
  let assembled: Block[]
  try {
    await live.open(model)
    await live.send([text('Run a command')])
    await live.send([text('Answer again')])
    await settled(id)
    assembled = live.transcript().blocks
  } finally {
    live.disconnect()
  }
  expect(kinds(assembled)).toEqual(['user', 'context', 'tool_call', 'response', 'text', 'response', 'user', 'text', 'response'])
  const cold = await coldTranscript(id)
  expect(assembled).toEqual(cold.blocks)

  // The page reloads: a new client opens the conversation and receives the transcript whole.
  const reloaded = await connect(id)
  try {
    const reset = Promise.withResolvers<Block[]>()
    reloaded.subscribe((event) => {
      if (event.type === 'transcript_reset') {
        reset.resolve(event.blocks)
      }
    })
    await reloaded.open(model)
    expect(await reset.promise).toEqual(cold.blocks)
  } finally {
    reloaded.disconnect()
  }
  expect(vendor.unused()).toBe(0)
})
