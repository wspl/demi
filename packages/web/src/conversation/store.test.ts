import { afterEach, beforeEach, expect, jest, spyOn, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import type { ClientContent } from '@demicodes/protocol'
import { ConversationRuntime } from '@demicodes/web-ui/agent/conversation-runtime'
import { toasts } from '@demicodes/web-ui/infra/toast'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick, toRaw, watch } from 'vue'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import type { ConversationDraft, ConversationSummary, DraftFile, Preferences } from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { applyConversationEvent, updateLiveStatus } from './activity'
import * as draftStorage from './drafts'
import * as localState from '../state/local'
import { DRAFT_SAVE_DELAY_MS } from './draft-sync'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'

const realFetch = globalThis.fetch
const FIRST = '00000000-0000-4000-8000-000000000001'
const SECOND = '00000000-0000-4000-8000-000000000002'
const SHA = 'a'.repeat(64)
const model = {
  providerId: 'stub', thinking: null, serviceTierId: null,
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
}
/** The provider entry a conversation infers with, as the product state lists it. */
const stubProvider = {
  id: 'stub', kind: 'api_key' as const, providerType: 'stub', label: 'Stub',
  wireApi: null, vendorId: null, baseUrl: null, models: null, createdAt: '2026-09-09T00:00:00.000Z',
  details: { type: 'failed' as const, message: 'Not read in this test' },
}
/** A catalog with the one stub model. */
function stubCatalog() {
  return { providers: [{
    providerId: 'stub', displayName: 'Stub', sourceFetchedAt: '1970-01-01T00:00:00.000Z', stale: false,
    warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' },
    cliPackage: null,
    availability: { type: 'available' },
    models: [{
      id: 'stub', displayName: 'Stub', description: null, contextWindow: 1000, outputLimit: null,
      supportsTools: null, supportsAttachments: true, supportsVideo: null, acceptedExtensions: null,
      supportsReasoning: null, supportedThinkingEfforts: [],
      canDisableThinking: null, serviceTiers: [], defaultServiceTierId: null, cost: null,
      selection: model,
    }],
  }] }
}

/** The upload transport is XHR; this one answers every POST with a fresh attachment id. */
class FakeXhr {
  static nextId = 1
  upload = { onprogress: null as ((event: { lengthComputable: boolean; loaded: number; total: number }) => void) | null }
  status = 201
  responseText = ''
  timeout = 0
  withCredentials = false
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  ontimeout: (() => void) | null = null
  onabort: (() => void) | null = null
  open(): void {}
  setRequestHeader(): void {}
  abort(): void {}
  send(): void {
    this.responseText = JSON.stringify({ attachment: {
      id: `att-${FakeXhr.nextId++}`, mediaType: 'text/plain', sizeBytes: 5, sha256: SHA,
      createdAt: '2026-09-09T00:00:00.000Z', snippet: 'Local notes',
    } })
    queueMicrotask(() => this.onload?.())
  }
}
globalThis.XMLHttpRequest = FakeXhr as unknown as typeof XMLHttpRequest
let records: ConversationSummary[]
let rejectCreate: boolean
let rejectFork: boolean
let requests: {
  path: string
  body: unknown
}[]
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let savedPreferences: Preferences
/** The drafts the backend keeps, by conversation. */
let serverDrafts: Map<string, ConversationDraft>

/**
 * The backend's draft routes (`web-api.md` § Conversation drafts): a save
 * always takes effect and keeps the version it replaced when it was built on
 * another revision; a restore exchanges the replaced version with the draft.
 */
function draftRoute(id: string, method: string, action: boolean, body: Record<string, unknown>): Response {
  const current = serverDrafts.get(id) ?? { revision: 0, text: '', files: [], replaced: null }
  let next: ConversationDraft
  if (method === 'GET') {
    return Response.json({ draft: current })
  } else if (action) {
    const replaced = current.replaced
    if (!replaced || replaced.revision !== body.revision) {
      return Response.json({ code: 'draft_changed', message: 'The replaced version changed' }, { status: 409 })
    }
    const { text, files } = current
    next = body.action === 'restore'
      ? { revision: current.revision + 1, text: replaced.text, files: replaced.files, replaced: { revision: current.revision, text, files } }
      : { ...current, revision: current.revision + 1, replaced: null }
  } else {
    const files = (body.files as ClientContent[]).map((file): DraftFile =>
      file.type === 'upload' ? { ...file, mediaType: 'text/plain', sha256: SHA } : file as DraftFile)
    next = {
      revision: current.revision + 1,
      text: body.text as string,
      files,
      replaced: body.base === current.revision
        ? current.replaced
        : { revision: current.revision, text: current.text, files: current.files },
    }
  }
  serverDrafts.set(id, next)
  return Response.json({ draft: next })
}

function record(id: string, title = id): ConversationSummary {
  return {
    id,
    title,
    pinned: false,
    archived: false,
    readRevision: 0,
    revision: 0,
    unread: false,
    titleCurrent: true,
    titleGenerating: false,
    pluginsChanged: false,
    draftRevision: 0,
    panelRevision: 0,
    pluginRevisions: [],
    workingTreeRevision: 0,
    permissionRequests: 0,
    permissionsRevision: 0,
    cwd: `/home/demi/sessions/${id}`,
    target: { kind: 'cloud' },
    contextVersion: 0,
    model: null,
    createdAt: '2026-09-09T00:00:00.000Z',
    updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle',
  }
}

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  records = [record(FIRST, 'first'), record(SECOND, 'second')]
  savedPreferences = { appearance: {}, shortcuts: {} }
  serverDrafts = new Map()
  rejectCreate = false
  rejectFork = false
  requests = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    requests.push({
      path,
      body,
    })
    if (path === '/api/settings/preferences') {
      savedPreferences = { ...savedPreferences, ...body }
      return Response.json({ preferences: savedPreferences })
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === `/api/conversations/${FIRST}/fork`) {
      if (rejectFork) {
        return Response.json({ code: 'internal_error', message: 'Fork unavailable' }, { status: 500 })
      }
      const created = records.find((item) => item.id === body.id) ?? {
        ...record(body.id), title: 'first (Fork)',
        model: { providerId: 'stub', modelId: 'model', thinkingEffort: 'high', serviceTierId: 'priority' },
        target: { kind: 'cloud' as const, path: '/home/demi/sessions/first' },
      }
      if (!records.includes(created)) records.unshift(created)
      return Response.json({ conversation: created }, { status: 201 })
    }
    if (path === '/api/conversations') {
      if (rejectCreate) {
        return Response.json(
          {
            code: 'internal_error',
            message: 'Not saved',
          },
          { status: 503 },
        )
      }
      const existing = records.find((item) => item.id === body.id)
      const created = existing ?? { ...record(body.id), title: body.title ?? 'New conversation', pinned: body.pinned ?? false }
      if (!existing) {
        records.unshift(created)
      }
      const hosts = (body.hosts ?? []).map((host: { deviceId: string; name: string }) => ({
        ...host, cwd: null, online: true, attachedAt: '2026-09-09T00:00:00.000Z',
      }))
      return Response.json({ conversation: created, hosts }, { status: existing ? 200 : 201 })
    }
    if (path === '/api/conversations/batch') {
      const items = body.items as {
        id: string
        patch: Partial<ConversationSummary>
      }[]
      return Response.json(
        {
          results: items.map((item) => {
            const current = records.find((record) => record.id === item.id)!
            if (item.id === SECOND) {
              return {
                status: 'updated',
                id: item.id,
                conversation: current,
                results: [
                  {
                    field: 'archived',
                    status: 'failed',
                    code: 'turn_in_flight',
                    message: 'Turn is running',
                    httpStatus: 409,
                  },
                ],
              }
            }
            Object.assign(current, item.patch)
            return {
              status: 'updated',
              id: item.id,
              conversation: current,
              results: Object.keys(item.patch).map((field) => ({
                field,
                status: 'applied',
              })),
            }
          }),
        },
        { status: 207 },
      )
    }
    if (path.endsWith('/read')) {
      return new Response(null, { status: 204 })
    }
    const draft = /^\/api\/conversations\/([^/]+)\/draft(\/replaced)?$/.exec(path)
    if (draft) {
      return draftRoute(decodeURIComponent(draft[1]!), init?.method ?? 'GET', draft[2] !== undefined, body)
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  useConversations()
  await connect()
})

afterEach(() => {
  useConversations().stopAll()
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

/** The backend's state, as its channel brings it. */
function backendState() {
  return productState({ preferences: savedPreferences, conversations: records })
}

/** The page starts following the state, and its channel brings the backend's first. */
async function connect(): Promise<void> {
  useProduct().start()
  channels.last().connect(backendState())
  await nextTick()
}

/** The channel brings the backend's whole state again, as it does when it connects again. */
async function reconnected(): Promise<void> {
  channels.last().send({ type: 'snapshot', state: backendState() })
  await nextTick()
}

/** The channel brings the conversation's summary, as it does once a change of it commits. */
async function changed(id: string): Promise<void> {
  channels.last().send({ type: 'conversation', conversation: records.find((item) => item.id === id)! })
  await nextTick()
}

test('an empty draft saves the complete last choice and new conversations restore it after reload', async () => {
  let store = useConversations()
  const id = store.create()
  const draft = store.items.find((item) => item.id === id)!
  await store.changeModel(draft, { model: { providerId: 'account', modelId: 'chosen-model' } })
  await store.changeModel(draft, { thinkingEffort: 'high' })
  await store.changeModel(draft, { serviceTierId: 'priority' })
  const chosen = { ...draft.model }
  expect(chosen).toEqual({ providerId: 'account', modelId: 'chosen-model', thinkingEffort: 'high', serviceTierId: 'priority' })
  const nextId = store.create('project')
  expect(store.items.find((item) => item.id === nextId)!.model).toEqual(chosen)
  expect(store.items.find((item) => item.id === FIRST)!.model.modelId).toBe('')
  await usePreferences().flush()
  expect(savedPreferences.lastModel).toEqual(chosen)
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)

  store.stopAll()
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  store = useConversations()
  await connect()
  const restoredId = store.create()
  const restored = store.items.find((item) => item.id === restoredId)!
  expect(restored.model).toEqual(chosen)
  await store.changeModel(restored, { model: { providerId: 'account', modelId: 'another-model' } })
  await usePreferences().flush()
  expect(savedPreferences.lastModel).toEqual({
    providerId: 'account', modelId: 'another-model',
    thinkingEffort: null, serviceTierId: null,
  })
  expect(draft.model).toEqual(chosen)
})

test('a page saves only the drafts it changed, so another tab\'s saved draft stays', async () => {
  const written = spyOn(draftStorage, 'writeDraft').mockResolvedValue(undefined)
  const deleted = spyOn(draftStorage, 'deleteDraft').mockResolvedValue(undefined)
  try {
    const store = useConversations()
    const typedId = store.create()
    const typed = store.items.find((item) => item.id === typedId)!
    typed.draft = 'typed here'
    const untouchedId = store.create('project')
    const untouched = store.items.find((item) => item.id === untouchedId)!
    untouched.draft = 'another tab may have saved a newer copy of this one'
    await nextTick()
    written.mockClear()
    deleted.mockClear()
    typed.draft = 'typed here, and more'
    await nextTick()
    expect(written.mock.calls.map(([, id]) => id)).toEqual([typed.id])
    expect(deleted).not.toHaveBeenCalled()
  } finally {
    written.mockRestore()
    deleted.mockRestore()
  }
})

test('a completed earlier preference write cannot discard a newer model selection', async () => {
  const preferences = usePreferences()
  const started = deferred<void>()
  const release = deferred<void>()
  const fetch = globalThis.fetch
  let held = false
  globalThis.fetch = (async (input, init) => {
    const response = await fetch(input, init)
    if (String(input) === '/api/settings/preferences' && !held) {
      held = true
      started.resolve()
      await release.promise
    }
    return response
  }) as typeof fetch
  const first = {
    providerId: 'account', modelId: 'first-model',
    thinkingEffort: 'high', serviceTierId: 'priority',
  }
  const latest = { ...first, modelId: 'latest-model', thinkingEffort: 'low' }
  preferences.update({ lastModel: first }, true)
  await started.promise
  preferences.update({ lastModel: latest }, true)
  expect(preferences.lastModel).toEqual(latest)
  release.resolve()
  await preferences.flush()
  expect(preferences.lastModel).toEqual(latest)
  expect(savedPreferences.lastModel).toEqual(latest)
})

test('sidebar stays active until the last running child closes after its parent finishes', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'child-one',
    parentSessionId: conversation.id,
    description: 'First child',
    profile: null,
    phase: 'running' as const,
    startedAt: '2026-09-13T00:00:00.000Z',
    endedAt: null,
  }
  conversation.phase = 'idle'
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  expect(conversation.status).toBe('active')
  const other = { ...job, subagentId: 'child-two' }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job: other })
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'closed',
    job: { ...job, phase: 'completed', endedAt: '2026-09-13T00:01:00.000Z' },
  })
  expect(conversation.status).toBe('active')
  conversation.lastError = 'The parent failed while its second child was working'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'closed',
    job: { ...other, phase: 'aborted', endedAt: '2026-09-13T00:02:00.000Z' },
  })
  expect(conversation.status).toBe('error')
  conversation.phase = 'running'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
})

/** A `shell_exec` call of `script`, as a transcript holds it while it runs or once it returned. */
function execCall(toolUseId: string, script: string, status: 'executing' | 'completed') {
  return {
    type: 'tool_call' as const, id: `block-${toolUseId}`, createdAt: '2026-09-13T00:00:00.000Z', model,
    toolUseId, toolName: 'shell_exec', input: JSON.stringify({ script }), status, output: [], view: null,
  }
}

// `runtime.md` § Live output and § Rendering boundary: every page keeps one
// record of a command from its live frames, named by its call's script, and
// adds only the characters beyond those it has shown.
test('a command\'s live frames build what the page shows of it, until its end', () => {
  const conversation = useConversations().items[0]!
  conversation.blocks = [execCall('call-1', 'npm test', 'executing')]
  conversation.terminals = []
  const frame = (status: 'running' | 'exited', tail: string, chars: number) => ({
    type: 'shell_output' as const,
    status: status === 'running'
      ? { status, shellId: 'sh', commandId: 'cmd', toolUseId: 'call-1', tail, chars, runningMs: 10 }
      : { status, shellId: 'sh', commandId: 'cmd', toolUseId: 'call-1', tail, chars, runningMs: 10, exitCode: 0 },
  })
  applyConversationEvent(conversation, frame('running', 'one\n', 4))
  // The frame's tail holds only the newest characters; the page adds them.
  applyConversationEvent(conversation, frame('running', 'two\n', 8))
  expect(toRaw(conversation.terminals)).toMatchObject([
    { id: 'cmd', name: 'npm test', phase: 'running', output: 'one\ntwo\n', chars: 8, toolUseId: 'call-1' },
  ])
  // After a gap, the page shows the tail anew; a transcript event keeps the live view.
  applyConversationEvent(conversation, frame('running', 'ninety\n', 100))
  conversation.blocks = [{ ...execCall('call-1', 'npm test', 'completed'), view: {
    kind: 'shell', status: 'exited', exitCode: 0, shellId: 'sh', commandId: 'cmd', runningMs: 10, idleMs: 0,
    chunks: [{ stream: 'stdout', text: 'one\n' }], viewTruncated: false,
  } }]
  applyConversationEvent(conversation, { type: 'transcript_patch', patches: [], blocks: conversation.blocks, failures: {} })
  expect(toRaw(conversation.terminals)).toMatchObject([{ output: 'ninety\n', chars: 100, phase: 'running' }])
  applyConversationEvent(conversation, frame('exited', 'ninety\nend\n', 104))
  expect(toRaw(conversation.terminals)).toMatchObject([{ phase: 'exited', output: 'ninety\nend\n' }])
  expect(conversation.terminals[0]?.endedAt).toBeDefined()

  // A subagent's command takes its name from the child's transcript.
  const job = {
    subagentId: 'child', parentSessionId: conversation.id, description: 'Child', profile: null,
    phase: 'running' as const, startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  applyConversationEvent(conversation, {
    type: 'subagent_transcript_reset', subagentId: 'child', blocks: [execCall('call-1', 'cargo build', 'executing')], failures: {},
  })
  applyConversationEvent(conversation, {
    type: 'shell_output', subagentId: 'child',
    status: { status: 'running', shellId: 'sh-2', commandId: 'cmd-2', toolUseId: 'call-1', tail: 'Compiling\n', chars: 10, runningMs: 5 },
  })
  expect(toRaw(conversation.terminals)[1]).toMatchObject({
    id: 'cmd-2', name: 'cargo build', subagentId: 'child', toolUseId: 'call-1', output: 'Compiling\n',
  })
})

// A command the user or the agent stopped shows as stopped, never as done:
// from its last live frame, and from its stored view once the page reloads.
test('a stopped command stays stopped, live and after a reload', () => {
  const conversation = useConversations().items[0]!
  conversation.blocks = [execCall('call-1', 'npm run watch', 'executing')]
  conversation.terminals = []
  const view = { shellId: 'sh', commandId: 'cmd', toolUseId: 'call-1', tail: 'watching\n', chars: 9, runningMs: 10 }
  applyConversationEvent(conversation, { type: 'shell_output', status: { status: 'running', ...view } })
  applyConversationEvent(conversation, { type: 'shell_output', status: { status: 'aborted', ...view } })
  expect(toRaw(conversation.terminals)).toMatchObject([{ id: 'cmd', phase: 'aborted' }])

  conversation.terminals = []
  conversation.blocks = [{ ...execCall('call-1', 'npm run watch', 'completed'), view: {
    kind: 'shell', status: 'aborted', shellId: 'sh', commandId: 'cmd', runningMs: 10, idleMs: 0,
    chunks: [{ stream: 'stdout', text: 'watching\n' }], viewTruncated: false,
  } }]
  applyConversationEvent(conversation, { type: 'transcript_reset', blocks: conversation.blocks, failures: {} })
  expect(toRaw(conversation.terminals)).toMatchObject([{ id: 'cmd', phase: 'aborted' }])
})

test('a child keeps the failure facts of its transcript: a reset replaces them, a patch adds to them', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'child',
    parentSessionId: conversation.id,
    description: 'Child',
    profile: null,
    phase: 'running' as const,
    startedAt: '2026-09-13T00:00:00.000Z',
    endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  const lifts = { retryAt: '2026-09-22T07:37:39.000Z' }
  applyConversationEvent(conversation, { type: 'subagent_transcript_reset', subagentId: 'child', blocks: [], failures: { first: lifts } })
  applyConversationEvent(conversation, { type: 'subagent_transcript_patch', subagentId: 'child', patches: [], failures: { second: { retryAt: null } } })
  expect(conversation.subagents.find((agent) => agent.id === 'child')?.failures)
    .toEqual({ first: lifts, second: { retryAt: null } })
  applyConversationEvent(conversation, { type: 'subagent_transcript_reset', subagentId: 'child', blocks: [], failures: {} })
  expect(conversation.subagents.find((agent) => agent.id === 'child')?.failures).toEqual({})
})

test('restored running children keep an idle parent active in the sidebar', () => {
  const conversation = useConversations().items[0]!
  conversation.phase = 'idle'
  conversation.subagents = [{
    id: 'restored-child',
    name: 'Restored child',
    phase: 'running',
    startedAt: '2026-09-13T00:00:00.000Z',
    blocks: [],
    failures: {},
  }]
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('active')
  conversation.subagents[0]!.phase = 'completed'
  updateLiveStatus(conversation)
  expect(conversation.status).toBe('idle')
})

test('new conversation is local and does not depend on the server', async () => {
  const store = useConversations()
  rejectCreate = true
  const id = await store.create()
  expect(id).toMatch(/^[0-9a-f-]{36}$/)
  expect(store.items.map((item) => item.id)).toEqual([id!, FIRST, SECOND])
  expect(store.items[0]?.persistence).toBe('draft')
  expect(store.items[0]?.load).toBe('ready')
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
})

test('repeated new reuses the active empty draft and a new snapshot preserves it', async () => {
  const store = useConversations()
  const id = await store.create()
  await store.activate(id)
  expect(await store.create()).toBe(id)
  await reconnected()
  expect(store.items[0]?.id).toBe(id)
  store.items[0]!.draft = 'Keep this draft'
  expect(await store.create()).not.toBe(id)
})

test('first send failure preserves the conversation and retries its UUID and message', async () => {
  const store = useConversations()
  await store.create()
  const conversation = store.items[0]!
  conversation.draft = 'First message'
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.persistence).toBe('pending')
  expect(conversation.pendingSend?.text).toBe('First message')
  expect(conversation.pendingSend?.error).toBe('Not saved')
  expect(conversation.draft).toBe('')
  const messageId = conversation.pendingSend!.id
  await reconnected()
  expect(store.items[0]?.id).toBe(conversation.id)
  await store.send(conversation)
  expect(conversation.pendingSend?.id).toBe(messageId)
  expect(requests.filter((request) => request.path === '/api/conversations').map((request) => (request.body as { id: string }).id)).toEqual([
    conversation.id,
    conversation.id,
  ])
})

// The page hides the composer while a conversation loads: had the first send
// shown the new conversation as loading, the composer the user sent from
// would have gone, and the focus with it.
test('the first send keeps the new conversation shown while its record opens', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  useProduct().snapshot!.providers.push(stubProvider)
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) return Response.json({ blocks: [], subagents: [] })
    if (path === `/api/conversations/${id}` && init?.method === 'PATCH') {
      return Response.json({ conversation: records.find((item) => item.id === id), results: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  const shown: string[] = []
  const stop = watch(() => conversation.load, (load) => shown.push(load), { flush: 'sync' })
  try {
    await useProduct().loadModels(true)
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    conversation.draft = 'First message'
    await store.send(conversation)
    expect(submit).toHaveBeenCalledTimes(1)
    expect(conversation.persistence).toBe('synced')
    expect(shown).toEqual([])
  } finally {
    stop()
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('the first send creates the conversation with its settings and hosts in one request and reads nothing of it back', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  conversation.attachedHosts = [{ deviceId: 'laptop', name: 'build', cwd: null, online: true }]
  useProduct().snapshot!.providers.push(stubProvider)
  const originalFetch = globalThis.fetch
  const readBack: string[] = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.startsWith(`/api/conversations/${id}`)) readBack.push(`${init?.method ?? 'GET'} ${path}`)
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  try {
    await useProduct().loadModels(true)
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    conversation.draft = 'First message'
    await store.send(conversation)
    expect(submit).toHaveBeenCalledTimes(1)
    const created = requests.filter((request) => request.path === '/api/conversations')
    expect(created.map((request) => request.body)).toEqual([{
      id, title: conversation.title, pinned: false, target: { kind: 'cloud' },
      model: { providerId: 'stub', modelId: 'stub' }, serviceTierId: null,
      hosts: [{ deviceId: 'laptop', name: 'build' }],
    }])
    expect(readBack).toEqual([])
    expect(conversation.attachedHosts.map((host) => host.name)).toEqual(['build'])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('a draft keeps its files; the conversation itself is created on first send', async () => {
  const store = useConversations()
  store.create()
  const conversation = store.items[0]!
  const taken = await store.addFiles(conversation, [new File(['Local notes'], 'notes.txt', { type: 'text/plain' })])
  // The composer puts their capsules in the message and says what it now carries.
  store.arrangeFiles(conversation, taken.map((file) => file.id))
  await new Promise((resolve) => setTimeout(resolve, 0))
  expect(conversation.files[0]).toMatchObject({
    kind: 'file', name: 'notes.txt', phase: 'ready', snippet: 'Local notes', upload: { id: expect.stringMatching(/^att-/) },
  })
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.pendingSend?.fileIds).toEqual([conversation.files[0]!.id])
})

test('a file whose capsule was deleted comes back when undo brings the capsule back', async () => {
  const store = useConversations()
  store.create()
  const conversation = store.items[0]!
  const taken = await store.addFiles(conversation, [new File(['before'], 'before.png', { type: 'image/png' })])
  store.arrangeFiles(conversation, taken.map((file) => file.id))
  await new Promise((resolve) => setTimeout(resolve, 0))
  const file = conversation.files[0]!
  expect(file).toMatchObject({ name: 'before.png', phase: 'ready' })
  const uploaded = file.kind === 'file' ? file.upload?.id : undefined

  // The capsule was deleted: the message has no file, though the composer still carries it.
  store.arrangeFiles(conversation, [])
  expect(conversation.attachmentIds).toEqual([])
  expect(conversation.files).toEqual([file])

  // Undo brought the capsule back, and its file is the message's again.
  store.arrangeFiles(conversation, [file.id])
  expect(conversation.attachmentIds).toEqual([file.id])
  // It kept the upload it had: coming back is not uploading again.
  const back = conversation.files[0]!
  expect(back.kind === 'file' ? back.upload?.id : undefined).toBe(uploaded)
})

test('a message sends its text and files in the order the composer shows them', async () => {
  const store = useConversations()
  const current = store.items[0]!
  useProduct().snapshot!.providers.push(stubProvider)
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json(stubCatalog())
    }
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) return Response.json({ blocks: [], subagents: [] })
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  let sent: ClientContent[] = []
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (content) => {
    sent = content
  })
  try {
    await useProduct().loadModels(true)
    const taken = await store.addFiles(current, [
      new File(['before'], 'before.png', { type: 'image/png' }),
      new File(['after'], 'after.png', { type: 'image/png' }),
    ])
    store.arrangeFiles(current, taken.map((file) => file.id))
    await new Promise((resolve) => setTimeout(resolve, 0))
    current.draft = `Compare ${ATTACHMENT_MARK} with ${ATTACHMENT_MARK}. The **modal** padding is off.`
    // The capsules were dragged into the other order.
    store.arrangeFiles(current, [current.files[1]!.id, current.files[0]!.id])
    const upload = (name: string) => {
      const file = current.files.find((each) => each.name === name)
      return file?.kind === 'file' ? file.upload?.id : undefined
    }
    const before = upload('before.png')
    const after = upload('after.png')
    await store.send(current)
    expect(sent).toEqual([
      { type: 'text', text: 'Compare ' },
      { type: 'upload', ref: after!, fileName: 'after.png' },
      { type: 'text', text: ' with ' },
      { type: 'upload', ref: before!, fileName: 'before.png' },
      { type: 'text', text: '. The **modal** padding is off.' },
    ])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('leaving an empty draft discards it without removing a draft with input', async () => {
  const store = useConversations()
  const empty = store.create()
  await store.activate(empty)
  await store.activate(null)
  expect(store.items.some((item) => item.id === empty)).toBe(false)
  const retained = store.create()
  await store.activate(retained)
  store.items.find((item) => item.id === retained)!.draft = 'Keep this'
  await store.activate(null)
  expect(store.items.some((item) => item.id === retained)).toBe(true)
})

test('a page shows the model settings another page chose and writes nothing back', async () => {
  const store = useConversations()
  const current = store.items.find((item) => item.id === FIRST)!
  const record = records.find((item) => item.id === FIRST)!
  record.model = { providerId: 'stub', modelId: 'stub', thinkingEffort: 'low', serviceTierId: null }
  await changed(FIRST)
  expect(current.model).toEqual(record.model)
  // Another page raises the effort and turns Fast on: the summary this
  // page's channel brings shows both, as the record holds them.
  record.model = { ...record.model, thinkingEffort: 'high', serviceTierId: 'priority' }
  await changed(FIRST)
  expect(current.model).toEqual({ providerId: 'stub', modelId: 'stub', thinkingEffort: 'high', serviceTierId: 'priority' })
  expect(requests.some((request) => request.path === `/api/conversations/${FIRST}`)).toBe(false)
})

test('a model change names only the part it changes, so another page\'s change stays, and shows its answer at once', async () => {
  const store = useConversations()
  const current = store.items.find((item) => item.id === FIRST)!
  const record = records.find((item) => item.id === FIRST)!
  record.model = { providerId: 'stub', modelId: 'stub', thinkingEffort: 'low', serviceTierId: null }
  await changed(FIRST)
  // Another page turned Fast on; this page's channel has not brought it yet.
  record.model = { ...record.model, serviceTierId: 'priority' }
  const patches: unknown[] = []
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (String(input) !== `/api/conversations/${FIRST}` || init?.method !== 'PATCH') {
      return fetch(input, init)
    }
    const change = JSON.parse(String(init.body)) as { thinkingEffort: string }
    patches.push(change)
    record.model = { ...record.model!, thinkingEffort: change.thinkingEffort }
    return Response.json({ conversation: record, results: [{ field: 'thinking_effort', status: 'applied' }] })
  }) as typeof fetch
  try {
    await store.changeModel(current, { thinkingEffort: 'high' })
    expect(patches).toEqual([{ thinkingEffort: 'high' }])
    const both = { providerId: 'stub', modelId: 'stub', thinkingEffort: 'high', serviceTierId: 'priority' }
    expect(current.model).toEqual(both)
    await usePreferences().flush()
    expect(savedPreferences.lastModel).toEqual(both)
  } finally {
    globalThis.fetch = fetch
  }
})

test('a new snapshot preserves the live transcript and the unsent draft', async () => {
  const store = useConversations()
  const current = store.items[0]!
  current.draft = 'Still editing'
  current.phase = 'running'
  current.blocks = [
    { type: 'text', id: 'block', createdAt: '2026-09-09T00:00:00.000Z', model, text: 'streaming' },
  ]
  records[0]!.title = 'Server title'
  records.reverse()
  await reconnected()
  expect(store.items[1]).toBe(current)
  expect(current.title).toBe('Server title')
  expect(current.draft).toBe('Still editing')
  expect(current.blocks[0]?.id).toBe('block')
  expect(current.phase).toBe('running')
})

test('history remains readable during its own connection after navigation', async () => {
  const store = useConversations()
  const current = store.items[0]!
  current.blocks = [{ type: 'text', id: 'cached', createdAt: '2026-09-09T00:00:00.000Z', model, text: 'cached' }]
  current.draft = 'Keep the draft'
  useProduct().snapshot!.providers.push(stubProvider)
  const historyRequested = deferred<void>()
  const history = deferred<Response>()
  const connecting = deferred<void>()
  const connection = deferred<void>()
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockImplementation(() => {
    connecting.resolve()
    return connection.promise
  })
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json(stubCatalog())
    }
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) {
      historyRequested.resolve()
      return history.promise
    }
    return originalFetch(input, init)
  }) as typeof fetch
  await useProduct().loadModels(true)
  const opening = store.activate(current.id)
  try {
    expect(current).toMatchObject({ load: 'loading' })
    await historyRequested.promise
    expect(current).toMatchObject({ load: 'loading' })
    useProduct().activeConversationId = SECOND
    history.resolve(Response.json({ blocks: current.blocks, subagents: [] }))
    await connecting.promise
    expect(current).toMatchObject({ load: 'ready' })
    expect(current.draft).toBe('Keep the draft')
    connection.reject(new Error('Connection failed'))
    await opening
    expect(current).toMatchObject({ load: 'failed' })
    expect(current.lastError).toBe('Connection failed')
  } finally {
    history.resolve(Response.json({ blocks: [], subagents: [] }))
    connection.resolve()
    await opening
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('batch partial failure applies only the successful server records', async () => {
  const store = useConversations()
  expect(await store.archive([FIRST, SECOND])).toBe(false)
  expect(store.items.find((item) => item.id === FIRST)?.archived).toBe(true)
  expect(store.items.find((item) => item.id === SECOND)?.archived).toBe(false)
  expect(toasts.some((toast) => toast.message?.includes('second: Turn is running'))).toBe(true)
})

test('read acknowledgements use the observed revision and wait for history', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: { visibilityState: 'visible' },
  })
  try {
    const store = useConversations()
    const current = store.items[0]!
    current.unread = true
    current.revision = 7
    await store.markRead(current.id)
    expect(requests.some((request) => request.path.endsWith('/read'))).toBe(false)
    current.load = 'ready'
    await store.markRead(current.id)
    expect(requests.at(-1)?.body).toEqual({ revision: 7 })
    expect(current.unread).toBe(false)
  } finally {
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: original,
    })
  }
})

test('Fork preserves the source draft and opens an independent empty composer with inherited model settings', async () => {
  const store = useConversations()
  const source = store.items.find((item) => item.id === FIRST)!
  source.draft = 'Keep this unsent message'
  source.phase = 'running'
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  expect(await store.fork(source.id, request)).toBe(request.id)
  const branch = store.items.find((item) => item.id === request.id)!
  expect(branch.title).toBe('first (Fork)')
  expect(branch.draft).toBe('')
  expect(branch.files).toEqual([])
  expect(branch.model).toEqual({ providerId: 'stub', modelId: 'model', thinkingEffort: 'high', serviceTierId: 'priority' })
  expect(branch.target).toEqual({ kind: 'cloud', path: '/home/demi/sessions/first' })
  expect(source.draft).toBe('Keep this unsent message')
  expect(source.phase).toBe('running')
})

test('failed Fork creates no local conversation; retry forwards the same destination ID', async () => {
  const store = useConversations()
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  rejectFork = true
  await expect(store.fork(FIRST, request)).rejects.toThrow('Fork unavailable')
  expect(store.items.some((item) => item.id === request.id)).toBe(false)
  rejectFork = false
  await store.fork(FIRST, request)
  expect(requests.filter((item) => item.path.endsWith('/fork')).map((item) => item.body))
    .toEqual([request, request])
  expect(store.items.filter((item) => item.id === request.id)).toHaveLength(1)
})

function serveHistory(gate?: ReturnType<typeof deferred<void>>) {
  const requested = deferred<void>()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.endsWith('/hosts') || path.endsWith('/transcript')) {
      requests.push({ path, body: null })
      if (path.endsWith('/hosts')) {
        return Response.json({ hosts: [] })
      }
      requested.resolve()
      await gate?.promise
      return Response.json({ blocks: [], subagents: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  return requested.promise
}

/**
 * Serves an opening's reads, each held until the test answers it: the
 * transcript, the attached hosts and the draft. `arrived` names each as it
 * reaches the backend.
 */
function holdOpening() {
  const held = {
    transcript: deferred<void>(),
    hosts: deferred<void>(),
    draft: deferred<void>(),
    arrived: [] as string[],
  }
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const read = /\/(transcript|hosts|draft)$/.exec(path)?.[1]
    if (read && (init?.method ?? 'GET') === 'GET') {
      held.arrived.push(read)
      if (read === 'draft') {
        await held.draft.promise
        return originalFetch(input, init)
      }
      await held[read as 'transcript' | 'hosts'].promise
      return read === 'hosts' ? Response.json({ hosts: [] }) : Response.json({ blocks: [], subagents: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  return held
}

test('opening a conversation sends its reads at once and shows the transcript before its hosts answer', async () => {
  const held = holdOpening()
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  const opened = store.activate(FIRST)
  await waitFor(() => held.arrived.length === 3)
  expect(held.arrived.toSorted()).toEqual(['draft', 'hosts', 'transcript'])
  held.draft.resolve()
  held.transcript.resolve()
  await waitFor(() => first.load === 'ready')
  held.hosts.resolve()
  await opened
})

test('a conversation that can run makes its socket beside its reads', async () => {
  records[0]!.model = { providerId: 'stub', modelId: 'stub', thinkingEffort: null, serviceTierId: null }
  await changed(FIRST)
  const held = holdOpening()
  const store = useConversations()
  const opened = store.activate(FIRST)
  await waitFor(() => held.arrived.includes('transcript'))
  expect(channels.opened.map((channel) => new URL(channel.url).pathname)).toContain(`/api/conversations/${FIRST}/stream`)
  held.draft.resolve()
  held.hosts.resolve()
  held.transcript.resolve()
  await opened
})

test('the first load reads the conversation its address names before the channel\'s first state', async () => {
  useConversations().stopAll()
  useProduct().stop()
  const held = holdOpening()
  held.draft.resolve()
  held.hosts.resolve()
  held.transcript.resolve()
  const store = useConversations()
  await store.activate(FIRST)
  // Nothing names the conversation yet, and its reads are on their way.
  expect(store.items).toEqual([])
  await waitFor(() => held.arrived.length === 3)
  await connect()
  const first = store.items.find((item) => item.id === FIRST)!
  await waitFor(() => first.load === 'ready')
  // The opening took the reads the address started.
  expect(held.arrived.toSorted()).toEqual(['draft', 'hosts', 'transcript'])
})

test('switching between opened sessions performs no reads or load reset', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate(FIRST)
  const first = store.items.find((item) => item.id === FIRST)!
  first.draft = 'Keep this input'
  await store.activate(SECOND)
  const count = requests.length
  const returning = store.activate(FIRST)
  expect(first.load).toBe('ready')
  expect(useProduct().activeConversationId).toBe(FIRST)
  await returning
  await store.activate(SECOND)
  await store.activate(FIRST)
  expect(requests).toHaveLength(count)
  expect(first.draft).toBe('Keep this input')
})

test('leaving and returning during history loading shares the pending request', async () => {
  const gate = deferred<void>()
  serveHistory(gate)
  const store = useConversations()
  const first = store.activate(FIRST)
  await store.activate(null)
  const returning = store.activate(FIRST)
  gate.resolve()
  await Promise.all([first, returning])
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(1)
  expect(store.items[0]!.load).toBe('ready')
})

test('inactive context changes invalidate only that session; retry explicitly reloads', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate(FIRST)
  await store.activate(SECOND)
  records[0]!.contextVersion += 1
  await changed(FIRST)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(2)
  await store.activate(FIRST)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
  await store.activate(SECOND)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
  await store.reloadSession(SECOND)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(4)
})

test('archive changes and removal invalidate cached history', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate(FIRST)
  await store.activate(null)
  records[0]!.archived = true
  await changed(FIRST)
  await store.activate(FIRST)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(2)
  await store.activate(null)
  const removed = records.shift()!
  await reconnected()
  records.unshift(removed)
  await reconnected()
  await store.activate(FIRST)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(3)
})

test('logout cancels pending history and prevents late state restoration', async () => {
  const gate = deferred<void>()
  const requested = serveHistory(gate)
  const store = useConversations()
  const current = store.items[0]!
  const opening = store.activate(FIRST)
  await requested
  store.stopAll()
  gate.resolve()
  await opening
  expect(store.items).toEqual([])
  expect(current.load).toBe('loading')
})


test('slow or failed model discovery does not hold history behind the loading pane', async () => {
  const store = useConversations()
  const current = store.items[0]!
  const modelResponse = deferred<Response>()
  const originalFetch = globalThis.fetch
  const block = {
    type: 'user', id: 'visible-history', turnId: 'history-turn', preamble: null, model,
    createdAt: '2026-09-13T00:00:00.000Z', content: [{ type: 'text', text: 'Read this while models load' }],
  }
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return modelResponse.promise
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) return Response.json({ blocks: [block], subagents: [] })
    return originalFetch(input, init)
  }) as typeof fetch
  const models = useProduct().loadModels(true).catch(error => error)
  const opening = store.activate(current.id)
  await new Promise(resolve => setTimeout(resolve, 0))
  expect(current.lastError).toBeNull()
  expect(current.load).toBe('ready')
  expect(current.blocks[0]?.id).toBe('visible-history')
  modelResponse.reject(new Error('Catalog offline'))
  expect(await models).toBeInstanceOf(Error)
  await opening
  expect(current.load).toBe('ready')
  expect(current.blocks[0]?.id).toBe('visible-history')
})

test('a rename shows at once, survives a summary read before the write lands, and a refused one gives the title back', async () => {
  const store = useConversations()
  const product = useProduct()
  const started = deferred<void>()
  const release = deferred<void>()
  let refuse = false
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (String(input) !== `/api/conversations/${FIRST}` || init?.method !== 'PATCH') {
      return fetch(input, init)
    }
    started.resolve()
    await release.promise
    const current = records.find((item) => item.id === FIRST)!
    const { title } = JSON.parse(String(init.body)) as { title: string }
    if (!refuse) {
      current.title = title
    }
    return Response.json({
      conversation: current,
      results: [refuse
        ? { field: 'title', status: 'failed', code: 'conversation_archived', message: 'Restore it first', httpStatus: 409 }
        : { field: 'title', status: 'applied' }],
    })
  }) as typeof fetch
  const title = () => store.items.find((item) => item.id === FIRST)!.title

  store.rename(FIRST, 'Renamed')
  expect(title()).toBe('Renamed')
  await started.promise
  // The channel brings a summary read before the write landed, with the old title.
  await changed(FIRST)
  expect(title()).toBe('Renamed')
  release.resolve()
  // The write is answered, and the channel brings the summary it changed.
  await waitFor(() => records.find((item) => item.id === FIRST)?.title === 'Renamed')
  await changed(FIRST)
  expect(product.snapshot?.conversations.find((item) => item.id === FIRST)?.title).toBe('Renamed')
  expect(title()).toBe('Renamed')

  refuse = true
  store.rename(FIRST, 'Refused')
  expect(title()).toBe('Refused')
  // The refused write gives the stored title back.
  await waitFor(() => title() === 'Renamed', () => `the title is ${title()}`)
})

/** Signs the page in, as it is before it loads a conversation, with web browser storage that keeps `kept`, or nothing. */
function signIn(kept: draftStorage.SavedDraft | null = null): () => void {
  const spies = [
    spyOn(draftStorage, 'readDraft').mockResolvedValue(kept),
    spyOn(draftStorage, 'writeDraft').mockResolvedValue(undefined),
    spyOn(draftStorage, 'deleteDraft').mockResolvedValue(undefined),
    spyOn(localState, 'readLocalState').mockImplementation(() => localState.emptyLocalState()),
    spyOn(localState, 'writeLocalState').mockImplementation(() => {}),
  ]
  useSession().current = { status: 'signedIn', user: productState().user }
  return () => {
    for (const spy of spies) {
      spy.mockRestore()
    }
  }
}

/** Lets the event loop turn until `done`, which the page's requests and their answers reach without a timer. */
async function until(done: () => boolean, what: string): Promise<void> {
  for (let turn = 0; turn < 200 && !done(); turn += 1) {
    await new Promise((resolve) => setImmediate(resolve))
  }
  expect(done(), what).toBe(true)
}

/** Lets the event loop turn as often as a request and its answer take here, for what must not happen. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 20; turn += 1) {
    await new Promise((resolve) => setImmediate(resolve))
  }
}

/** Another page saves the conversation's draft; this page learns of it from its channel. */
async function savedElsewhere(id: string, text: string): Promise<void> {
  const current = serverDrafts.get(id)!
  serverDrafts.set(id, { ...current, revision: current.revision + 1, text })
  records.find((item) => item.id === id)!.draftRevision = current.revision + 1
  await changed(id)
}

const draftSaves = () => requests.filter((request) => request.path.endsWith('/draft') && request.body !== null)

test('a draft another page saved shows in a composer at rest; one its user is typing in keeps the text and saves on its own revision', async () => {
  const signOut = signIn()
  jest.useFakeTimers()
  try {
    serverDrafts.set(FIRST, { revision: 1, text: 'Fix the login', files: [], replaced: null })
    records[0]!.draftRevision = 1
    const store = useConversations()
    const current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    expect([current.draft, current.draftBase]).toEqual(['Fix the login', 1])

    // Nothing typed here: the newer draft takes the composer's place.
    const shown = current.draftShown
    await savedElsewhere(FIRST, 'Fix the login test')
    await until(() => current.draft === 'Fix the login test', 'the other page\'s draft shows')
    expect([current.draftBase, current.draftShown]).toEqual([2, shown + 1])

    // Typed here, not saved yet: the composer keeps it when another page saves.
    current.draft = 'Fix the login bug'
    await savedElsewhere(FIRST, 'Fix the login test, again')
    await until(() => current.savedDraft?.revision === 3, 'the newer draft is read')
    expect(current.draft).toBe('Fix the login bug')
    expect(draftSaves()).toEqual([])

    // Half a second after the typing pauses the page saves, built on the
    // revision its text came from, so the backend keeps what it replaces.
    jest.advanceTimersByTime(DRAFT_SAVE_DELAY_MS)
    await until(() => current.savedDraft?.revision === 4, 'the save is answered')
    expect(draftSaves().map((request) => request.body)).toEqual([{ base: 2, text: 'Fix the login bug', files: [] }])
    expect([current.draft, current.draftBase, current.savedDraft?.replaced?.text]).toEqual(['Fix the login bug', 4, 'Fix the login test, again'])

    // Restore brings the replaced version back and offers the one it displaced.
    store.restoreReplaced(current)
    await until(() => current.draft === 'Fix the login test, again', 'the restored draft shows')
    expect(current.savedDraft?.replaced?.text).toBe('Fix the login bug')
    store.dismissReplaced(current)
    await until(() => current.savedDraft?.replaced === null, 'the offer is dismissed')
    expect([current.draft, current.draftBase]).toEqual(['Fix the login test, again', 6])
  } finally {
    jest.useRealTimers()
    signOut()
  }
})

test('a send empties the draft everywhere once the backend accepts it, and a change another page made meanwhile stays to restore', async () => {
  const signOut = signIn()
  const store = useConversations()
  const current = store.items.find((item) => item.id === FIRST)!
  useProduct().snapshot!.providers.push(stubProvider)
  serverDrafts.set(FIRST, { revision: 1, text: 'Ship it', files: [], replaced: null })
  records[0]!.draftRevision = 1
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/transcript')) return Response.json({ blocks: [], subagents: [] })
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const accepted = deferred<void>()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(() => accepted.promise)
  jest.useFakeTimers()
  try {
    await useProduct().loadModels(true)
    await store.activate(FIRST)
    expect(current.draft).toBe('Ship it')
    const sending = store.send(current)
    await until(() => submit.mock.calls.length === 1, 'the message is submitted')
    // Until the backend accepts the message the draft stays, however long
    // that takes, and another page edits it meanwhile.
    expect(current.draft).toBe('')
    serverDrafts.set(FIRST, { revision: 2, text: 'Ship it today', files: [], replaced: null })
    jest.advanceTimersByTime(DRAFT_SAVE_DELAY_MS)
    await settle()
    expect(draftSaves()).toEqual([])
    accepted.resolve()
    await sending
    await until(() => draftSaves().length === 1, 'the draft is cleared')
    expect(draftSaves()[0]!.body).toEqual({ base: 1, text: '', files: [] })
    await until(() => current.savedDraft?.revision === 3, 'the clear is answered')
    expect([current.draft, current.savedDraft?.text, current.savedDraft?.replaced?.text]).toEqual(['', '', 'Ship it today'])
  } finally {
    jest.useRealTimers()
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
    signOut()
  }
})

test('a page that closes while typing leaves its text to the next page, which takes the backend\'s revision without saving it again', async () => {
  // The first page types, and closes before its save and its IndexedDB write land.
  let signOut = signIn()
  const kept = spyOn(draftStorage, 'writeLastWords').mockImplementation(() => {})
  let words: draftStorage.LastWords | undefined
  try {
    serverDrafts.set(FIRST, { revision: 1, text: 'Fix the lo', files: [], replaced: null })
    records[0]!.draftRevision = 1
    let store = useConversations()
    let current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    current.draft = 'Fix the login bug'
    store.keepLastWords()
    words = kept.mock.calls.find(([, id]) => id === FIRST)?.[2]
    expect(words).toEqual({ base: 1, text: 'Fix the login bug', attachmentIds: [] })
    signOut()
    store.stopAll()
    useProduct().stop()
    disposePinia(pinia)

    // Its save on close arrived; its IndexedDB record holds an older text.
    serverDrafts.set(FIRST, { revision: 2, text: 'Fix the login bug', files: [], replaced: null })
    records[0]!.draftRevision = 2
    pinia = createPinia()
    setActivePinia(pinia)
    signOut = signIn({
      messageEdit: null, pendingSend: null, local: null, base: 1, text: 'Fix the lo',
      model: null, files: [], scroll: null,
    })
    const taken = spyOn(draftStorage, 'takeLastWords').mockReturnValue(words ?? null)
    jest.useFakeTimers()
    try {
      store = useConversations()
      await connect()
      current = store.items.find((item) => item.id === FIRST)!
      await store.activate(FIRST)
      expect([current.draft, current.draftBase]).toEqual(['Fix the login bug', 2])
      jest.advanceTimersByTime(DRAFT_SAVE_DELAY_MS)
      await settle()
      expect(draftSaves()).toEqual([])
    } finally {
      jest.useRealTimers()
      taken.mockRestore()
    }
  } finally {
    kept.mockRestore()
    signOut()
  }
})
