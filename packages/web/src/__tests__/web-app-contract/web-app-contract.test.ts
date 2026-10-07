import { afterAll, afterEach, beforeAll, expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { waitFor } from '@demicodes/utils'
import { createPinia, setActivePinia } from 'pinia'
import type { Block, ClientContent, ModelSelection } from '@demicodes/protocol'
import {
  EditRejectedError,
  beginMessageEdit,
  changeMessageEditContent,
  lastEditableUserMessageId,
  sentEditRequest,
} from '@demicodes/web-ui/agent/message-editing'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ConversationSocketError, connectConversationClient } from '@demicodes/web-ui/transport/conversation-socket'
import type { ClientSessionEvent } from '@demicodes/web-ui/transport/protocol'
import { ApiError, apiRequest, jsonBody, readResponse } from '../../api/client'
import { changeReplacedDraft, loadDraft, saveDraft } from '../../api/drafts'
import {
  deviceAnswerSchema,
  createdConversationSchema,
  conversationUpdateSchema,
  devicesSchema,
  modelCatalogSchema,
  providerAnswerSchema,
  transcriptSchema,
  type Claim,
  type ConversationPatch,
  type CreateConversation,
  type ConversationSummary,
  type CreateProvider,
  type ReadRequest,
  type SetupRequest,
} from '../../api/generated/web-api'
import { useSession } from '../../auth/session'
import { useProduct } from '../../state/product'
import { openWebBrowser, startBackend, startRunner, temporaryRoot, type Backend, type Runner } from './harness'
import { startScriptedAnthropic, type MessagesRequest, type ScriptedAnthropic } from './scripted-anthropic'

// The web app contract suite (`scenarios.md` § Web app contract suite): the
// backend executable, driven the way the page drives it, through the web
// application's API client and the conversation socket's `ConversationClient`,
// which validate every answer and frame with the generated schemas. The
// model is a scripted Anthropic-compatible endpoint, and tools run on a real
// runner. The file takes about 4 seconds: the backend's start and a runner's
// pairing once, then each test well under a second.

const EMAIL = 'master@example.test'
const PASSWORD = 'contract-password'
/** The one model of the scripted entry, as a manual list states it. */
const MODEL_ID = 'claude-scripted'
/** How long a conversation may take to settle after its last frame. */
const SETTLE_MS = 10_000

let root: Awaited<ReturnType<typeof temporaryRoot>>
let vendor: ScriptedAnthropic
let backend: Backend
let webBrowser: ReturnType<typeof openWebBrowser>
const runners: Runner[] = []

beforeAll(async () => {
  root = await temporaryRoot()
  vendor = startScriptedAnthropic()
  backend = await startBackend(root.path)
  webBrowser = openWebBrowser(backend.origin)
  setActivePinia(createPinia())
  // The instance's first account, signed in by its setup.
  await apiRequest('/setup', { method: 'POST', ...jsonBody({ nickname: 'Master', email: EMAIL, password: PASSWORD } satisfies SetupRequest) })
})

// A test that failed may leave replies it scripted; the next starts with none.
afterEach(() => {
  vendor.clear()
})

afterAll(async () => {
  webBrowser?.restore()
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
  const request: CreateConversation = {
    id,
    ...(model ? { model: { providerId: model.providerId, modelId: model.model.id } } : {}),
  }
  const created = await readResponse(
    await apiRequest('/conversations', { method: 'POST', ...jsonBody(request) }),
    createdConversationSchema,
  )
  expect(created.conversation.model?.modelId ?? null).toBe(model?.model.id ?? null)
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
  return connectConversationClient(webBrowser.socketUrl(`/conversations/${id}/stream`))
}

type Client = Awaited<ReturnType<typeof connect>>

/**
 * The conversation's socket, connected and opened, once its snapshot is in:
 * `opened` comes first, and `pending_steers` ends the snapshot frames.
 */
async function openConversation(id: string): Promise<Client> {
  const client = await connect(id)
  try {
    const snapshot = nextEvent(client, (event) => (event.type === 'pending_steers' ? true : undefined))
    await client.open()
    await snapshot
  } catch (error) {
    client.disconnect()
    throw error
  }
  return client
}

/**
 * The first event of `client` that `pick` answers for, as the page's
 * listeners receive it; a connection that ends first fails the wait.
 */
function nextEvent<T>(client: Client, pick: (event: ClientSessionEvent) => T | undefined): Promise<T> {
  const wait = new Promise<T>((resolve, reject) => {
    const stop = client.subscribe((event) => {
      if (event.type === 'disconnected') {
        stop()
        reject(event.error)
        return
      }
      const value = pick(event)
      if (value !== undefined) {
        stop()
        resolve(value)
      }
    })
  })
  // A test that fails before it waits leaves no rejection unhandled; one
  // that waits still receives it.
  wait.catch(() => {})
  return wait
}

/** The next time the session shows `idle`: everything it ran or queued has ended and is saved. */
function idle(client: Client): Promise<true> {
  return nextEvent(client, (event) => (event.type === 'phase' && event.phase === 'idle') || undefined)
}

/** The next queue the session shows, as the ids of its messages, once it satisfies `check`. */
function queueOf(client: Client, check: (ids: string[]) => boolean): Promise<string[]> {
  return nextEvent(client, (event) => {
    if (event.type !== 'queue') {
      return undefined
    }
    const ids = event.queue.map((message) => message.id)
    return check(ids) ? ids : undefined
  })
}

/** The user's side of what the model received, as JSON, to look for a message's text in. */
function userTexts(request: MessagesRequest | undefined): string {
  return JSON.stringify(request?.messages.filter((message) => message.role === 'user'))
}

/** Another page of the user's, in the same web browser: its own product state, followed through its own channel. */
function openPage() {
  return useProduct(createPinia())
}

/** Waits, as the page's own code does, until the page's copy holds what `check` looks for. */
function pageShows(page: ReturnType<typeof openPage>, id: string, check: (summary: ConversationSummary) => boolean) {
  return page.until(
    (state) => state.conversations.some((conversation) => conversation.id === id && check(conversation)),
    AbortSignal.timeout(SETTLE_MS),
  )
}

/** The transcript the backend serves cold, from the conversation's database. */
async function coldTranscript(id: string) {
  return readResponse(await apiRequest(`/conversations/${id}/transcript`), transcriptSchema)
}

/** A device paired as the page pairs one: its runner prints a code, and the signed-in page claims it. */
let paired: Promise<{ deviceId: string; runner: Runner }> | undefined
function pairedDevice() {
  return paired ??= (async () => {
    const runner = await startRunner(root.path, backend.origin, 'laptop')
    runners.push(runner)
    const { device } = await readResponse(
      await apiRequest('/devices/claim', { method: 'POST', ...jsonBody({ code: await runner.code } satisfies Claim) }),
      deviceAnswerSchema,
    )
    const deadline = Date.now() + SETTLE_MS
    for (;;) {
      const { devices } = await readResponse(await apiRequest('/devices'), devicesSchema)
      if (devices.some((each) => each.id === device.id && each.state === 'online')) {
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

/** Whether a WebSocket to the `/api` route `path` opens, as the page's would. */
function opens(path: string): Promise<boolean> {
  const socket = new WebSocket(webBrowser.socketUrl(path))
  return new Promise((resolve) => {
    socket.onopen = () => {
      socket.close()
      resolve(true)
    }
    socket.onclose = () => resolve(false)
  })
}

test('a user signs in through the API client, and the session admits the synchronization channel, whose snapshot is the account\'s, and the conversation socket', async () => {
  const session = useSession()
  const id = await createConversation()
  await session.signOut()
  expect(webBrowser.signedIn()).toBe(false)
  // Signed out, the page reaches neither the API, nor its channel, nor the
  // socket.
  const refused = await apiRequest('/conversations').catch((error: unknown) => error)
  expect(refused).toBeInstanceOf(ApiError)
  expect(refused).toMatchObject({ status: 401, code: 'unauthenticated' })
  expect(await opens('/sync')).toBe(false)
  expect(await connect(id).catch((error: unknown) => error)).toBeInstanceOf(ConversationSocketError)

  await session.signIn(EMAIL, PASSWORD, new AbortController().signal)
  expect(session.user?.email).toBe(EMAIL)
  // The page's module takes the product state from its channel, checking
  // every message against the generated schema. Creating a conversation
  // does not make the user's Cloud, so it is not made yet.
  const product = useProduct()
  product.start()
  try {
    await waitFor(() => product.load === 'ready', () => `the channel is ${product.load}`, { timeoutMs: SETTLE_MS })
    expect(product.snapshot?.user.email).toBe(EMAIL)
    expect(product.snapshot?.cloud).toMatchObject({ device: null, state: 'unallocated' })
    // A change made through the API reaches the page on its channel.
    await patchConversation(id, { title: 'Renamed on another page' })
    const title = () => product.snapshot?.conversations.find((conversation) => conversation.id === id)?.title
    await waitFor(() => title() === 'Renamed on another page', () => `the title is ${title()}`, { timeoutMs: SETTLE_MS })
  } finally {
    product.stop()
  }
  const model = await scriptedModel()
  await patchConversation(id, { model: { providerId: model.providerId, modelId: model.model.id } })
  const client = await connect(id)
  try {
    const events: ClientSessionEvent['type'][] = []
    client.subscribe((event) => events.push(event.type))
    await client.open()
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
    await client.open()
    // The send resolves when the action it started ends.
    await client.send([text('Say hello')])
    expect(phases).toEqual(['idle', 'running', 'idle'])
    const blocks = client.transcript().blocks
    // The model learned its Host before its first request.
    expect(kinds(blocks)).toEqual(['user', 'context', 'text', 'response'])
    expect(blocks[2]).toMatchObject({ type: 'text', text: 'Hello from the script.' })
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
    input: { script: 'printf contract > probe.txt && cat probe.txt', description: 'Write the probe file', timeoutMs: 30_000 },
  })
  vendor.reply({ type: 'text', text: 'The probe says contract.' })
  const client = await connect(id)
  try {
    await client.open()
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
    await live.open()
    await live.send([text('Run a command')])
    // A send resolves when the socket shows idle, once the turn's save has committed.
    await live.send([text('Answer again')])
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
    await reloaded.open()
    expect(await reset.promise).toEqual(cold.blocks)
  } finally {
    reloaded.disconnect()
  }
  expect(vendor.unused()).toBe(0)
})

test('a submitted message is acknowledged once it is in the transcript, before the model answers', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  const held = vendor.hold({ type: 'text', text: 'An answer that waits.' })
  const client = await openConversation(id)
  try {
    const messageId = crypto.randomUUID()
    await client.submit([text('Acknowledge me')], messageId)
    // The page clears its composer now; the reply has not come.
    const written = client.transcript().blocks
    expect(kinds(written)).toEqual(['user'])
    expect(written[0]).toMatchObject({ type: 'user', turnId: messageId, content: [{ type: 'text', text: 'Acknowledge me' }] })
    await held.requested
    const ended = idle(client)
    held.release()
    await ended
    expect(kinds(client.transcript().blocks)).toEqual(['user', 'context', 'text', 'response'])
  } finally {
    client.disconnect()
  }
})

test('an edit of the last message replaces it and what follows, and an edit of a version the page no longer shows is refused', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  vendor.reply({ type: 'text', text: 'Answer to the first.' })
  vendor.reply({ type: 'text', text: 'Answer to the second.' })
  vendor.reply({ type: 'text', text: 'Answer to the edit.' })
  const client = await openConversation(id)
  try {
    await client.send([text('First message')])
    await client.send([text('Second message')])
    const blocks = client.transcript().blocks
    const target = blocks.find((block) => block.id === lastEditableUserMessageId(blocks))
    const version = client.transcriptVersion()
    if (!target || !version) {
      throw new Error(`Nothing to edit: ${JSON.stringify(blocks)}`)
    }
    // The page opens the message in its editor and changes the text.
    const editing = changeMessageEditContent(beginMessageEdit(target, version), (content) => {
      content.splice(0, content.length, { type: 'text', text: 'Edited second message' })
    })
    const ended = idle(client)
    await client.editAndSend(sentEditRequest(editing.request))
    await ended
    const edited = client.transcript().blocks
    expect(kinds(edited)).toEqual(['user', 'context', 'text', 'response', 'user', 'text', 'response'])
    expect(edited.slice(0, 4)).toEqual(blocks.slice(0, 4))
    expect(edited[4]).toMatchObject({ type: 'user', content: [{ type: 'text', text: 'Edited second message' }] })
    expect(edited[5]).toMatchObject({ type: 'text', text: 'Answer to the edit.' })
    // The model read the first exchange and the edit, never the replaced message.
    const request = userTexts(vendor.turns().at(-1))
    expect(request).toContain('First message')
    expect(request).toContain('Edited second message')
    expect(request).not.toContain('Second message')
    expect(edited).toEqual((await coldTranscript(id)).blocks)

    // An editor opened on the old version is refused, although its message
    // is still there, and nothing changes.
    const first = edited[0]
    if (!first) {
      throw new Error('The first message is gone')
    }
    const stale = beginMessageEdit(first, version)
    await expect(client.editAndSend(sentEditRequest(stale.request))).rejects.toBeInstanceOf(EditRejectedError)
    expect(client.transcript().blocks).toEqual(edited)
  } finally {
    client.disconnect()
  }
})

test('messages sent while a turn runs wait in the queue, where the page removes one and moves another to the front', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  const held = vendor.hold({ type: 'text', text: 'Answer to the first.' })
  vendor.reply({ type: 'text', text: 'Answer to the moved one.' })
  vendor.reply({ type: 'text', text: 'Answer to the waiting one.' })
  const client = await openConversation(id)
  try {
    await client.submit([text('Running message')])
    await held.requested
    const [removed, waiting, moved] = [crypto.randomUUID(), crypto.randomUUID(), crypto.randomUUID()]
    // Each submit is acknowledged once the queue holds its message.
    await client.submit([text('Removed message')], removed)
    await client.submit([text('Waiting message')], waiting)
    await client.submit([text('Moved message')], moved)
    const afterRemove = queueOf(client, (ids) => !ids.includes(removed))
    client.dequeueMessage(removed)
    expect(await afterRemove).toEqual([waiting, moved])
    const afterMove = queueOf(client, (ids) => ids[0] === moved)
    client.sendQueuedMessage(moved)
    expect(await afterMove).toEqual([moved, waiting])

    const ended = idle(client)
    held.release()
    await ended
    const turns = client.transcript().blocks.flatMap((block) => (block.type === 'user' ? [JSON.stringify(block.content)] : []))
    expect(turns.map((content) => content.match(/(\w+) message/)?.[1])).toEqual(['Running', 'Moved', 'Waiting'])
    expect(JSON.stringify(vendor.turns().slice(-3))).not.toContain('Removed message')
  } finally {
    client.disconnect()
  }
})

test('a queued message sent now steers the running turn: the page lists it as pending, and the model reads it at the next boundary', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  const held = vendor.hold({ type: 'text', text: 'Running the tests.' })
  vendor.reply({ type: 'text', text: 'Skipping the flaky suite.' })
  const client = await openConversation(id)
  try {
    await client.submit([text('Run the tests')])
    await held.requested
    const queued = crypto.randomUUID()
    await client.submit([text('Skip the flaky suite')], queued)
    const steerId = crypto.randomUUID()
    const listed = nextEvent(client, (event) => (event.type === 'pending_steers' && event.pendingSteers.length > 0 ? event.pendingSteers : undefined))
    const emptied = queueOf(client, (ids) => ids.length === 0)
    // The page's Send now while a turn runs.
    await client.steerQueuedMessage(queued, steerId)
    expect(await listed).toMatchObject([{ id: steerId, content: [{ type: 'text', text: 'Skip the flaky suite' }] }])
    expect(await emptied).toEqual([])

    const ended = idle(client)
    held.release()
    await ended
    const blocks = client.transcript().blocks
    expect(kinds(blocks)).toEqual(['user', 'context', 'text', 'response', 'steer', 'text', 'response'])
    expect(blocks[4]).toMatchObject({ type: 'steer', id: steerId })
    expect(client.pendingSteers()).toEqual([])
    // The turn asked the model once more, with the steer.
    expect(userTexts(vendor.turns().at(-1))).toContain('Skip the flaky suite')
  } finally {
    client.disconnect()
  }
})

test('Stop ends the running turn with its pending steer written, and Continue goes on from there', async () => {
  const model = await scriptedModel()
  const id = await createConversation(model)
  const held = vendor.hold({ type: 'text', text: 'An answer that never arrives.' })
  vendor.reply({ type: 'text', text: 'Continuing with the steer.' })
  const client = await openConversation(id)
  try {
    await client.submit([text('Start something long')])
    await held.requested
    const queued = crypto.randomUUID()
    await client.submit([text('Use the other approach')], queued)
    await client.steerQueuedMessage(queued)
    // The page's interrupt with a pending steer: Stop, then Continue.
    const stopped = idle(client)
    expect(await client.abort()).toEqual({ target: 'active_provider_stream', canAbortAgain: false })
    // The answer comes once the stop is in the transcript, and the backend
    // stopped asking the model.
    expect(kinds(client.transcript().blocks)).toEqual(['user', 'context', 'steer', 'abort'])
    await held.cancelled
    // Continue is admitted once the session is idle, after the stopped
    // turn's save (`runtime.md` § Actions); the answer to Stop comes before.
    await stopped
    await client.resume()
    const blocks = client.transcript().blocks
    expect(blocks.find((block) => block.type === 'abort')).toMatchObject({ isResumed: true })
    expect(blocks.at(-2)).toMatchObject({ type: 'text', text: 'Continuing with the steer.' })
    expect(userTexts(vendor.turns().at(-1))).toContain('Use the other approach')
  } finally {
    client.disconnect()
  }
})

test('the page answers a running command\'s prompt, and the command\'s output reaches the page and the model', async () => {
  const model = await scriptedModel()
  const { id } = await conversationOnDevice(model)
  vendor.reply({
    type: 'tool_use',
    name: 'shell_exec',
    input: { script: 'echo "name?"; read line; echo "got $line"', description: 'Ask for a name', timeoutMs: 30_000 },
  })
  vendor.reply({ type: 'text', text: 'The command read the line.' })
  const client = await openConversation(id)
  try {
    const view = (event: ClientSessionEvent) => (event.type === 'shell_output' ? event.status : undefined)
    // The user answers once the page shows the prompt.
    const prompted = nextEvent(client, (event) => {
      const status = view(event)
      return status?.status === 'running' && status.tail === 'name?\n' ? status.commandId : undefined
    })
    const exited = nextEvent(client, (event) => {
      const status = view(event)
      return status && status.status !== 'running' ? status : undefined
    })
    const ended = idle(client)
    await client.submit([text('Read a line')])
    const commandId = await prompted
    await client.shellWrite(commandId, 'contract\n')
    expect(await exited).toMatchObject({ status: 'exited', exitCode: 0, commandId, tail: 'name?\ngot contract\n' })
    await ended
    const call = client.transcript().blocks.find((block) => block.type === 'tool_call')
    expect(call).toMatchObject({ toolName: 'shell_exec', status: 'completed' })
    expect(JSON.stringify(call?.output)).toContain('got contract')
    expect(JSON.stringify(vendor.turns().at(-1)?.messages)).toContain('got contract')
  } finally {
    client.disconnect()
  }
})

test('a draft saved on one page reaches another, a save over a version the page never showed keeps it to restore, and a stale action is refused', async () => {
  const { deviceId, runner } = await pairedDevice()
  const id = await createConversation()
  const signal = new AbortController().signal
  const other = openPage()
  other.start()
  try {
    expect(await loadDraft(id, signal)).toEqual({ revision: 0, text: '', files: [], replaced: null })
    const trace = { type: 'remote_file', deviceId, path: join(runner.home, 'trace.txt') } as const
    const first = await saveDraft(id, { base: 0, text: `Fix the login ${ATTACHMENT_MARK}`, files: [trace] }, { signal })
    expect(first).toEqual({ revision: 1, text: `Fix the login ${ATTACHMENT_MARK}`, files: [trace], replaced: null })
    // The other page learns of the save from the summary, then reads the draft.
    await pageShows(other, id, (summary) => summary.draftRevision === 1)
    expect(await loadDraft(id, signal)).toEqual(first)

    // Both pages type on revision 1 at once.
    const mine = await saveDraft(id, { base: 1, text: 'Fix the login bug', files: [] }, { signal })
    expect(mine).toMatchObject({ revision: 2, replaced: null })
    const theirs = await saveDraft(id, { base: 1, text: 'Fix the login test', files: [] }, { signal })
    expect(theirs).toEqual({ revision: 3, text: 'Fix the login test', files: [], replaced: { revision: 2, text: 'Fix the login bug', files: [] } })
    // Restoring exchanges the two, so nothing is lost.
    const restored = await changeReplacedDraft(id, { action: 'restore', revision: 2 }, signal)
    expect(restored).toEqual({ revision: 4, text: 'Fix the login bug', files: [], replaced: { revision: 3, text: 'Fix the login test', files: [] } })
    // A page that still offers revision 2 is told the draft changed.
    const refused = await changeReplacedDraft(id, { action: 'dismiss', revision: 2 }, signal).catch((error: unknown) => error)
    expect(refused).toBeInstanceOf(ApiError)
    expect(refused).toMatchObject({ status: 409, code: 'draft_changed' })
    await pageShows(other, id, (summary) => summary.draftRevision === 4)
  } finally {
    other.stop()
  }
})

test('another page follows a conversation it does not show: its creation, its turn running and ending unread, and the read acknowledgement', async () => {
  const model = await scriptedModel()
  const other = openPage()
  other.start()
  try {
    // The snapshot is in, so the conversation reaches the page as a change.
    await other.until(() => true, AbortSignal.timeout(SETTLE_MS))
    const id = await createConversation(model)
    // A new conversation takes the front of the other page's list.
    await other.until((state) => state.conversations[0]?.id === id, AbortSignal.timeout(SETTLE_MS))
    const held = vendor.hold({ type: 'text', text: 'Done while you looked away.' })
    const client = await openConversation(id)
    try {
      await client.submit([text('Work on it')])
      await pageShows(other, id, (summary) => summary.status === 'running')
      const ended = idle(client)
      held.release()
      await ended
    } finally {
      client.disconnect()
    }
    await pageShows(other, id, (summary) => summary.status === 'completed' && summary.unread)
    const revision = other.snapshot?.conversations.find((conversation) => conversation.id === id)?.revision ?? 0
    // The page that showed the output acknowledges it.
    await apiRequest(`/conversations/${id}/read`, { method: 'POST', ...jsonBody({ revision } satisfies ReadRequest) })
    await pageShows(other, id, (summary) => !summary.unread && summary.readRevision === revision)
  } finally {
    other.stop()
  }
})
