import { afterEach, beforeEach, expect, jest, setSystemTime, spyOn, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import type { ClientContent, MessageComment, ServerFrame } from '@demicodes/protocol'
import { RefusedError } from '@demicodes/conversation-client'
import { ConversationRuntime, RemovalRefusedError } from '@demicodes/web-ui/agent/conversation-runtime'
import { listTailBlocks } from '@demicodes/web-ui/agent/list-tail'
import { rowKey } from '@demicodes/web-ui/agent/row-key'
import { toasts } from '@demicodes/web-ui/infra/toast'
import { codeText, reportPageErrorsWith } from '@demicodes/web-ui/infra/errors'
import { createPageI18n } from '@demicodes/web-ui/i18n/text'
import { sendPageErrorReport } from '../api/page-errors'
import { pageReturned, reconnectNow } from '@demicodes/web-ui/transport/liveness'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick, toRaw, watch } from 'vue'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import type { AttachedHost, ConversationDraft, ConversationSummary, DraftFile, Preferences, SavedMessage, WorkspaceDto } from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { applyConversationEvent, updateLiveStatus } from './activity'
import * as draftStorage from './drafts'
import * as localState from '../state/local'
import { DRAFT_SAVE_DELAY_MS } from './record'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { contentCopy } from '@demicodes/web-ui/agent/message-input/copied-message'
import { quotedWords, textWords } from '@demicodes/web-ui/agent/send-to-conversation'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { latestBlocks, wholeHistory } from '@demicodes/web-ui/agent/history'
import { TestSocket } from '@demicodes/web-ui/transport/__tests__/test-socket'

/** The latest page of an empty transcript, as the backend answers it. */
const EMPTY_PAGE = { start: 0, length: 0, blocks: [], instructions: [], summaries: [] }
// What a child without a profile runs on, in the jobs below.
const RUNS_ON = { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false }

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
  wireApi: null, vendorId: null, compatible: false, endpoint: null, hasApiKey: true, rotation: 'manual' as const, media: { image: { shape: null, enabled: true }, video: { shape: null, enabled: true }, audio: { shape: null, enabled: true }, catalog: '2026-09-01T00:00:00.000Z' }, models: null, createdAt: '2026-09-09T00:00:00.000Z',
  details: { type: 'failed' as const, message: 'Not read in this test' },
}
/** The state the page read lists the stub provider entry too. */
function addStubProvider(): void {
  const product = useProduct()
  product.read = { ...product.read!, providers: [...product.read!.providers, stubProvider] }
}
/** A catalog with the one stub model. */
function stubCatalog() {
  return { providers: [{
    providerId: 'stub', displayName: 'Stub', sourceFetchedAt: '1970-01-01T00:00:00.000Z', imageModels: [], videoModels: [], audioModels: [], stale: false,
    warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' },
    cliPackage: null,
    availability: { type: 'available' },
    models: [{
      id: 'stub', displayName: 'Stub', description: null, contextWindow: 1000, outputLimit: null,
      supportsTools: null, acceptedExtensions: null,
      supportsReasoning: null, supportedThinkingEfforts: [],
      canDisableThinking: null, serviceTiers: [], defaultServiceTierId: null, cost: null,
      selection: model,
    }],
  }], smallModel: null }
}

/**
 * The upload transport is XHR; this one answers every POST with a fresh
 * attachment id, once `gate` opens when a test holds the uploads, and with the
 * backend's failure for the next `failures` uploads.
 */
class FakeXhr {
  static nextId = 1
  static gate: Promise<void> | null = null
  static failures = 0
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
    void (FakeXhr.gate ?? Promise.resolve()).then(() => {
      if (FakeXhr.failures > 0) {
        FakeXhr.failures -= 1
        this.status = 500
        this.responseText = JSON.stringify({ requestId: 'req_test000000000001', code: 'internal_error', message: 'The blob store failed.' })
      } else {
        this.status = 201
        this.responseText = JSON.stringify({ attachment: {
          id: `att-${FakeXhr.nextId++}`, mediaType: 'text/plain', sizeBytes: 5, sha256: SHA,
          createdAt: '2026-09-09T00:00:00.000Z', snippet: 'Local notes',
        } })
      }
      this.onload?.()
    })
  }
}
globalThis.XMLHttpRequest = FakeXhr as unknown as typeof XMLHttpRequest
let records: ConversationSummary[]
let rejectCreate: boolean
/** Holds the next creation's answer until the test opens it. */
let holdCreate: Promise<void> | null
/** What a test records the order of: each conversation PATCH the backend took, beside what it records. */
let order: string[] | null
let rejectFork: boolean
let requests: {
  path: string
  body: unknown
}[]
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let savedPreferences: Preferences
/** The user's projects as the backend keeps them. */
let workspaces: WorkspaceDto[]
/** The drafts the backend keeps, by conversation. */
let serverDrafts: Map<string, ConversationDraft>

/**
 * The backend's draft routes: a save always takes effect and keeps the version it
 * replaced when it was built on another revision; a restore exchanges the replaced
 * version with the draft.
 */
function draftRoute(id: string, method: string, action: boolean, body: Record<string, unknown>): Response {
  const current = serverDrafts.get(id) ?? { revision: 0, text: '', files: [], comments: [], replaced: null }
  let next: ConversationDraft
  if (method === 'GET') {
    return Response.json({ draft: current })
  } else if (action) {
    const replaced = current.replaced
    if (!replaced || replaced.revision !== body.revision) {
      return Response.json({ requestId: 'req_test000000000001', code: 'draft_changed', message: 'The replaced version changed' }, { status: 409 })
    }
    const { text, files, comments } = current
    next = body.action === 'restore'
      ? { revision: current.revision + 1, text: replaced.text, files: replaced.files, comments: replaced.comments, replaced: { revision: current.revision, text, files, comments } }
      : { ...current, revision: current.revision + 1, replaced: null }
  } else {
    const files = (body.files as ClientContent[]).map((file): DraftFile =>
      file.type === 'upload' ? { ...file, mediaType: 'text/plain', sha256: SHA } : file as DraftFile)
    next = {
      revision: current.revision + 1,
      text: body.text as string,
      files,
      comments: body.comments as MessageComment[],
      replaced: body.base === current.revision
        ? current.replaced
        : { revision: current.revision, text: current.text, files: current.files, comments: current.comments },
    }
  }
  serverDrafts.set(id, next)
  records.find((item) => item.id === id)!.draftRevision = next.revision
  return Response.json({ draft: next, revisions: revisionsOf(id) })
}

/** The messages saved for later the backend keeps, by conversation, newest first. */
let serverSaved: Map<string, SavedMessage[]>
/** The answers of the edits the backend made, by their operation id. */
let serverEdits: Map<string, unknown>

/** The conversation's revisions of its draft and its list, as every write answers them. */
function revisionsOf(id: string) {
  return { draft: serverDrafts.get(id)?.revision ?? 0, saved: records.find((item) => item.id === id)!.savedRevision }
}

/**
 * The backend's routes of messages saved for later: the list, with the revision the
 * conversation's summary carries; a save of the composer's message, which empties the
 * draft, and a plain one, each under the page's id; an edit, kept by its operation id;
 * and a deletion, which a message gone already answers the same.
 */
function savedRoute(id: string, method: string, path: string, body: Record<string, unknown>): Response {
  const summary = records.find((item) => item.id === id)!
  const messages = serverSaved.get(id) ?? []
  const current = serverDrafts.get(id) ?? { revision: 0, text: '', files: [], comments: [], replaced: null }
  const raise = () => {
    summary.savedRevision += 1
    summary.saved = serverSaved.get(id)!.length
  }
  const [, messageId, action] = /^\/saved(?:\/([^/]+))?(?:\/([^/]+))?$/.exec(path) ?? []
  if (method === 'GET') {
    return Response.json({ revision: summary.savedRevision, messages })
  }
  if (method === 'DELETE') {
    if (messages.some((message) => message.id === messageId)) {
      serverSaved.set(id, messages.filter((message) => message.id !== messageId))
      raise()
    }
    return Response.json({ revisions: revisionsOf(id) })
  }
  if (action === 'edit') {
    const kept = serverEdits.get(body.op as string)
    if (kept) {
      return Response.json(kept)
    }
    const moved = messages.find((message) => message.id === messageId)
    if (!moved) {
      return Response.json({ requestId: 'req_test000000000001', code: 'saved_not_found', message: 'No such message saved for later' }, { status: 404 })
    }
    const rest = messages.filter((message) => message !== moved)
    const draft = { revision: current.revision + 1, text: moved.text, files: moved.files, comments: current.comments, replaced: current.replaced }
    serverDrafts.set(id, draft)
    summary.draftRevision = draft.revision
    serverSaved.set(id, current.text.trim() || current.files.length
      ? [{ id: body.id as string, text: current.text, files: current.files, savedAt: '2026-09-09T00:00:00.000Z' }, ...rest]
      : rest)
    raise()
    const answer = { draft, revisions: revisionsOf(id) }
    serverEdits.set(body.op as string, answer)
    return Response.json(answer)
  }
  const message: SavedMessage = { id: body.id as string, text: body.text as string, files: [], savedAt: '2026-09-09T00:00:00.000Z' }
  if (!messages.some((each) => each.id === message.id)) {
    serverSaved.set(id, [message, ...messages])
    raise()
  }
  if (messageId === 'from-draft') {
    const draft = { revision: current.revision + 1, text: '', files: [], comments: current.comments, replaced: current.replaced }
    serverDrafts.set(id, draft)
    summary.draftRevision = draft.revision
    return Response.json({ message, draft, revisions: revisionsOf(id) }, { status: 201 })
  }
  return Response.json({ message, revisions: revisionsOf(id) }, { status: 201 })
}

function record(id: string, title = id): ConversationSummary {
  return conversationSummary(id, title, { cwd: `/home/demi/sessions/${id}` })
}

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  records = [record(FIRST, 'first'), record(SECOND, 'second')]
  savedPreferences = { appearance: {}, shortcuts: {} }
  workspaces = []
  serverDrafts = new Map()
  serverSaved = new Map()
  serverEdits = new Map()
  FakeXhr.gate = null
  FakeXhr.failures = 0
  rejectCreate = false
  holdCreate = null
  order = null
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
      return Response.json({ providers: [], smallModel: null })
    }
    const workspacePatch = /^\/api\/workspaces\/([^/]+)$/.exec(path)
    if (workspacePatch && init?.method === 'PATCH') {
      const changed = { ...workspaces.find((item) => item.id === decodeURIComponent(workspacePatch[1]!))!, ...body }
      workspaces = workspaces.map((item) => item.id === changed.id ? changed : item)
      return Response.json({ workspace: changed })
    }
    if (path === `/api/conversations/${FIRST}/fork`) {
      if (rejectFork) {
        return Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Fork unavailable' }, { status: 500 })
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
      const held = holdCreate
      holdCreate = null
      await held
      if (rejectCreate) {
        return Response.json(
          {
            requestId: 'req_test000000000001',
            code: 'internal_error',
            message: 'Not saved',
          },
          { status: 503 },
        )
      }
      const existing = records.find((item) => item.id === body.id)
      const created = existing ?? {
        ...record(body.id), title: body.title ?? null, pinned: body.pinned ?? false, workflow: body.workflow ?? { type: 'default' },
      }
      if (!existing) {
        records.unshift(created)
      }
      return Response.json({ conversation: created }, { status: existing ? 200 : 201 })
    }
    if (path === '/api/conversations/batch') {
      const items = body.items as {
        id: string
        patch: Partial<ConversationSummary> & { notifyAgent?: boolean }
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
            // Telling the agent is no field of the record, and has no result of its own.
            const { notifyAgent: _told, ...fields } = item.patch
            Object.assign(current, fields)
            return {
              status: 'updated',
              id: item.id,
              conversation: current,
              results: Object.keys(fields).map((field) => ({
                field,
                status: 'applied',
              })),
            }
          }),
        },
        { status: 207 },
      )
    }
    const conversationPatch = /^\/api\/conversations\/([^/]+)$/.exec(path)
    if (conversationPatch && init?.method === 'PATCH') {
      order?.push('patch')
      const current = records.find((item) => item.id === decodeURIComponent(conversationPatch[1]!))!
      Object.assign(current, body)
      return Response.json({
        conversation: current,
        results: Object.keys(body).map((field) => ({ field, status: 'applied' })),
      })
    }
    if (path.endsWith('/read')) {
      return new Response(null, { status: 204 })
    }
    if (/^\/api\/conversations\/[^/]+\/cost$/.test(path)) {
      return Response.json(NO_COST)
    }
    const draft = /^\/api\/conversations\/([^/]+)\/draft(\/replaced)?$/.exec(path)
    if (draft) {
      return draftRoute(decodeURIComponent(draft[1]!), init?.method ?? 'GET', draft[2] !== undefined, body)
    }
    const saved = /^\/api\/conversations\/([^/]+)(\/saved.*)$/.exec(path)
    if (saved) {
      return savedRoute(decodeURIComponent(saved[1]!), init?.method ?? 'GET', saved[2]!, body)
    }
    if (path.startsWith('/api/attachments?')) {
      // The backend holds none of these files: their bytes go next.
      return Response.json({ attachment: null })
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
  return productState({ preferences: savedPreferences, conversations: records, workspaces })
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

/** A project of the user's, on a device, which remembers no choice yet. */
function workspace(id: string): WorkspaceDto {
  return { id, deviceId: 'laptop', path: `/work/${id}`, name: id, createdAt: '2026-09-09T00:00:00.000Z' }
}

test('each project, and the conversations outside projects, start a new conversation with the choices last made there', async () => {
  workspaces = [workspace('demi'), workspace('website')]
  await reconnected()
  let store = useConversations()
  /** Opens a new conversation there, and keeps it from being the next one. */
  const open = (projectId: string | null) => {
    const id = store.create(projectId)
    const conversation = store.items.find((item) => item.id === id)!
    conversation.draft = 'Not empty'
    return conversation
  }
  const opus = { model: { providerId: 'account', modelId: 'opus' } }
  const sonnet = { model: { providerId: 'account', modelId: 'sonnet' } }
  const inDemi = open('demi')
  await store.changeModel(inDemi, opus)
  await store.changeWorkflow(inDemi.id, { type: 'workflow', id: 'parallel' })
  await store.changeModel(open(null), sonnet)
  // A project never used starts as the conversations outside one do.
  const inWebsite = open('website')
  expect([inWebsite.model.modelId, inWebsite.workflow]).toEqual(['sonnet', { type: 'none' }])
  await store.changeWorkflow(inWebsite.id, { type: 'default' })
  await usePreferences().flush()
  await waitFor(() => workspaces.every((item) => item.lastWorkflow !== undefined), () => 'a project kept no choice')
  expect(savedPreferences.lastWorkflow).toBeUndefined()
  expect(savedPreferences.lastModel?.modelId).toBe('sonnet')

  // Another page, which reads the choices from the backend.
  store.stopAll()
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  store = useConversations()
  await connect()
  const starts = (projectId: string | null) => {
    const conversation = open(projectId)
    return [conversation.model.modelId, conversation.workflow]
  }
  expect(starts('demi')).toEqual(['opus', { type: 'workflow', id: 'parallel' }])
  expect(starts(null)).toEqual(['sonnet', { type: 'none' }])
  // Website's own workflow, and the only model chosen for it, outside.
  expect(starts('website')).toEqual(['sonnet', { type: 'default' }])
  // A project the page does not know starts as outside.
  expect(starts('unknown')).toEqual(['sonnet', { type: 'none' }])
})

test('a project that remembers only a model starts with the workflow chosen outside projects', async () => {
  workspaces = [{ ...workspace('demi'), lastModel: { providerId: 'account', modelId: 'opus', thinkingEffort: null, serviceTierId: null } }]
  savedPreferences = {
    ...savedPreferences,
    lastModel: { providerId: 'account', modelId: 'sonnet', thinkingEffort: null, serviceTierId: null },
    lastWorkflow: { type: 'default' },
  }
  await reconnected()
  const store = useConversations()
  const id = store.create('demi')
  const conversation = store.items.find((item) => item.id === id)!
  expect([conversation.model.modelId, conversation.workflow]).toEqual(['opus', { type: 'default' }])
})

test('a new conversation whose project no longer has the workflow it remembers goes on with No Subagents', async () => {
  workspaces = [{ ...workspace('demi'), lastWorkflow: { type: 'workflow', id: 'explore' } }]
  await reconnected()
  const store = useConversations()
  const id = store.create('demi')
  const conversation = store.items.find((item) => item.id === id)!
  const review = { id: 'review', name: 'Review Team', path: '/work/demi/.demi/workflows/review.yaml', unavailable: null, profiles: [] }
  const explore = { ...review, id: 'explore', name: 'Explore', path: '/work/demi/.demi/workflows/explore.yaml' }
  store.settleStartingWorkflow(id, { state: 'reading' })
  store.settleStartingWorkflow(id, { state: 'read', workflows: [review, explore] })
  expect(conversation.workflow).toEqual({ type: 'workflow', id: 'explore' })
  store.settleStartingWorkflow(id, { state: 'read', workflows: [review] })
  expect(conversation.workflow).toEqual({ type: 'none' })
  // Nothing is remembered: the project still starts with what it had.
  expect(requests.some((request) => request.path.startsWith('/api/workspaces'))).toBe(false)
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

// A waiting comment on a picture reaches the web browser's storage as
// IndexedDB takes it: a copy whose picture is no Vue proxy, which IndexedDB
// cannot clone.
test('a draft holding a comment on a picture is written as IndexedDB can store it', async () => {
  const stored: unknown[] = []
  const written = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, _id, draft) => {
    // As IndexedDB's put clones what it stores.
    stored.push(structuredClone(draft))
  })
  const signedOut = useSession().current
  useSession().current = { status: 'signedIn', user: productState().user }
  try {
    const store = useConversations()
    const id = store.create()
    const conversation = store.items.find((item) => item.id === id)!
    const picture: MessageComment = { id: 'c1', blockId: 'b1', start: 0, end: 1, at: 12, quote: 'The chart', picture: { type: 'url', url: 'https://example.com/chart.png' }, text: 'Which one?' }
    conversation.draft = 'And the rest?'
    store.commenting(conversation).add(picture)
    await nextTick()
    expect(stored.at(-1)).toMatchObject({ comments: [picture] })
  } finally {
    useSession().current = signedOut
    written.mockRestore()
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
  preferences.update({ lastModel: first })
  await started.promise
  preferences.update({ lastModel: latest })
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
    number: 1,
    parentSessionId: conversation.id,
    onPath: true,
    description: 'First child',
    profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
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
  conversation.lastError = { message: 'The parent failed while its second child was working', requestId: null }
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

// A resumed child's line counts its new round: the `started` job of round 2
// replaces the closed round's start and end.
test('a resumed child\'s record takes its new round\'s start, end and model', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'child', number: 1, parentSessionId: conversation.id, onPath: true, description: 'Look',
    profile: 'research', model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase: 'running' as const, startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  applyConversationEvent(conversation, {
    type: 'subagent', event: 'closed', job: { ...job, phase: 'completed', endedAt: '2026-09-13T00:01:00.000Z' },
  })
  const second = { ...job, round: 2, startedAt: '2026-09-13T00:05:00.000Z', model: { ...RUNS_ON, fast: true } }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job: second })
  expect(conversation.subagents.find((agent) => agent.id === 'child')).toMatchObject({
    phase: 'running', round: 2, startedAt: '2026-09-13T00:05:00.000Z', endedAt: undefined, model: second.model,
  })
})

// A child's tail row reads its own phase: a child idle beside a command it
// left running shows no Requesting row, and its phase never becomes the
// root's.
test('a live child\'s phase frames set its own phase, not the root\'s', () => {
  const conversation = useConversations().items[0]!
  conversation.phase = 'idle'
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'started',
    job: {
      subagentId: 'child', number: 1, parentSessionId: conversation.id, onPath: true, description: 'Serve',
      profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work', phase: 'running', startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
    },
  })
  const child = () => conversation.subagents.find((agent) => agent.id === 'child')!
  applyConversationEvent(conversation, { type: 'phase', subagentId: 'child', phase: 'running' })
  expect(child().sessionPhase).toBe('running')
  applyConversationEvent(conversation, { type: 'phase', subagentId: 'child', phase: 'idle' })
  expect(child().sessionPhase).toBe('idle')
  expect(conversation.phase).toBe('idle')
})

/** A `shell` call of `script`, as a transcript holds it while it runs or once it returned. */
function execCall(toolUseId: string, script: string, status: 'executing' | 'completed', description?: string) {
  return {
    type: 'tool_call' as const, id: `block-${toolUseId}`, createdAt: '2026-09-13T00:00:00.000Z', model,
    toolUseId, toolName: 'shell', input: JSON.stringify({ script, description }), status, endedAt: status === 'executing' ? null : '2026-09-13T00:00:00.000Z', output: [], view: null,
  }
}

// `crates/agent-server/src/server/tree/live.rs`: every page keeps one record of
// a command from its live frames, titled by its call's title and opening with
// its script as the frames carry them, and adds only the characters beyond
// those it has shown.
test('a command\'s live frames build what the page shows of it, until its end', () => {
  const conversation = useConversations().items[0]!
  conversation.history = wholeHistory([execCall('call-1', 'npm test', 'executing', 'Run the unit tests')])
  conversation.terminals = []
  const frame = (status: 'running' | 'exited', tail: string, chars: number) => ({
    type: 'shell_output' as const,
    status: status === 'running'
      ? { status, commandId: 'cmd', toolUseId: 'call-1', title: 'Run the unit tests', script: 'npm test', tail, chars, runningMs: 10 }
      : { status, commandId: 'cmd', toolUseId: 'call-1', title: 'Run the unit tests', script: 'npm test', tail, chars, runningMs: 10, exitCode: 0 },
  })
  applyConversationEvent(conversation, frame('running', 'one\n', 4))
  // The frame's tail holds only the newest characters; the page adds them.
  applyConversationEvent(conversation, frame('running', 'two\n', 8))
  expect(toRaw(conversation.terminals)).toMatchObject([
    { id: 'cmd', title: 'Run the unit tests', script: 'npm test', phase: 'running', output: 'one\ntwo\n', chars: 8, toolUseId: 'call-1' },
  ])
  // After a gap, the page shows the tail anew.
  applyConversationEvent(conversation, frame('running', 'ninety\n', 100))
  expect(toRaw(conversation.terminals)).toMatchObject([
    { title: 'Run the unit tests', script: 'npm test', output: 'ninety\n', chars: 100, phase: 'running' },
  ])
  applyConversationEvent(conversation, frame('exited', 'ninety\nend\n', 104))
  expect(toRaw(conversation.terminals)).toMatchObject([{ phase: 'exited', exitCode: 0, output: 'ninety\nend\n' }])
  expect(conversation.terminals[0]?.endedAt).toBeDefined()

  // A subagent's command is titled as its frames say, as the root's is.
  const job = {
    subagentId: 'child', number: 1, parentSessionId: conversation.id, onPath: true, description: 'Child', profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase: 'running' as const, startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  applyConversationEvent(conversation, {
    type: 'subagent_transcript_reset', subagentId: 'child', start: 0, length: 1, blocks: [execCall('call-1', 'cargo build', 'executing')], failures: {},
  })
  applyConversationEvent(conversation, {
    type: 'shell_output', subagentId: 'child',
    status: { status: 'running', commandId: 'cmd-2', toolUseId: 'call-1', title: 'cargo build', script: 'cargo build', tail: 'Compiling\n', chars: 10, runningMs: 5 },
  })
  expect(toRaw(conversation.terminals)[1]).toMatchObject({
    id: 'cmd-2', title: 'cargo build', script: 'cargo build', subagentId: 'child', toolUseId: 'call-1', output: 'Compiling\n',
  })
})

// A command the user or the agent stopped shows as stopped, never as done.
test('a stopped command stays stopped', () => {
  const conversation = useConversations().items[0]!
  conversation.history = wholeHistory([execCall('call-1', 'npm run watch', 'executing')])
  conversation.terminals = []
  const view = { commandId: 'cmd', toolUseId: 'call-1', title: 'Watch', script: 'npm run watch', tail: 'watching\n', chars: 9, runningMs: 10 }
  applyConversationEvent(conversation, { type: 'shell_output', status: { status: 'running', ...view } })
  applyConversationEvent(conversation, { type: 'shell_output', status: { status: 'aborted', ...view } })
  expect(toRaw(conversation.terminals)).toMatchObject([{ id: 'cmd', phase: 'aborted' }])
})

test('a child keeps the failure facts of its transcript, which resets and patches add to', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'child',
    number: 1,
    parentSessionId: conversation.id,
    onPath: true,
    description: 'Child',
    profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase: 'running' as const,
    startedAt: '2026-09-13T00:00:00.000Z',
    endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  const lifts = { retryAt: '2026-09-22T07:37:39.000Z' }
  applyConversationEvent(conversation, { type: 'subagent_transcript_reset', subagentId: 'child', start: 0, length: 0, blocks: [], failures: { first: lifts } })
  applyConversationEvent(conversation, { type: 'subagent_transcript_patch', subagentId: 'child', patches: [], failures: { second: { retryAt: null } } })
  expect(conversation.subagents.find((agent) => agent.id === 'child')?.failures)
    .toEqual({ first: lifts, second: { retryAt: null } })
  // A reset brings the blocks from its start; the page may hold those before it, with their facts.
  applyConversationEvent(conversation, { type: 'subagent_transcript_reset', subagentId: 'child', start: 0, length: 0, blocks: [], failures: {} })
  expect(conversation.subagents.find((agent) => agent.id === 'child')?.failures)
    .toEqual({ first: lifts, second: { retryAt: null } })
})

test('a child’s directory follows its changed frames, with the round untouched', () => {
  const conversation = useConversations().items[0]!
  const job = {
    subagentId: 'mover',
    number: 2,
    parentSessionId: conversation.id,
    onPath: true,
    description: 'Mover',
    profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase: 'running' as const,
    startedAt: '2026-09-13T00:00:00.000Z',
    endedAt: null,
  }
  applyConversationEvent(conversation, { type: 'subagent', event: 'started', job })
  expect(conversation.subagents.find((agent) => agent.id === 'mover')?.cwd).toBe('/work')
  applyConversationEvent(conversation, { type: 'subagent', event: 'changed', job: { ...job, cwd: '/work/slots/2' } })
  expect(conversation.subagents.find((agent) => agent.id === 'mover')).toMatchObject({ cwd: '/work/slots/2', round: 1, phase: 'running' })
})

test('restored running children keep an idle parent active in the sidebar', () => {
  const conversation = useConversations().items[0]!
  conversation.phase = 'idle'
  conversation.subagents = [{
    id: 'restored-child',
    number: 1,
    name: 'Restored child',
    onPath: true,
    profile: null,
    model: RUNS_ON,
    round: 1,
    forkedFrom: null,
    phase: 'running',
    sessionPhase: 'idle',
    waitingSince: null,
    startedAt: '2026-09-13T00:00:00.000Z',
    pendingCalls: [],
    history: EMPTY_TRANSCRIPT,
    failures: {},
    contextUsage: null,
    cwd: '/work',
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

test('a new conversation is listed while it is open, and left empty it goes from the list', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id, { newConversation: true })
  const listed = (which: string) => store.listed.some((item) => item.id === which)
  expect(listed(id)).toBe(true)
  await store.activate(FIRST)
  expect(listed(id)).toBe(false)
  const kept = store.create()
  await store.activate(kept, { newConversation: true })
  store.items.find((item) => item.id === kept)!.attachmentIds = ['att-1']
  await store.activate(FIRST)
  expect(listed(kept)).toBe(true)
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

test('a new conversation keeps the workflow it was given, in its saved draft and in the record its first send makes', async () => {
  const written = spyOn(draftStorage, 'writeDraft').mockResolvedValue(undefined)
  const store = useConversations()
  const id = store.create()
  const conversation = store.items.find((item) => item.id === id)!
  store.changeWorkflow(id, { type: 'workflow', id: 'review' })
  // Kept with the draft, and no record is asked to change.
  expect(conversation.workflow).toEqual({ type: 'workflow', id: 'review' })
  expect(requests.some((request) => request.path.startsWith('/api/conversations'))).toBe(false)
  conversation.draft = 'First message'
  await store.send(conversation)
  const saved = written.mock.calls.map(([, , draft]) => draft).filter((draft) => draft.local !== null)
  expect(saved.at(-1)?.workflow).toEqual({ type: 'workflow', id: 'review' })
  const created = requests.filter((request) => request.path === '/api/conversations')
  expect(created.map((request) => (request.body as { workflow?: unknown }).workflow)).toEqual([{ type: 'workflow', id: 'review' }])
  written.mockRestore()
})

test('a choice made while the first message makes the record applies before that message goes, after a failed creation and its retry too', async () => {
  const seen: string[] = []
  order = seen
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async () => {
    seen.push('submit')
  })
  const store = useConversations()
  const product = useProduct()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  // A model the conversation can open with.
  product.snapshot!.providers.push(stubProvider)
  const backend = globalThis.fetch
  globalThis.fetch = (async (input, init) =>
    String(input).startsWith('/api/models') ? Response.json(stubCatalog()) : backend(input, init)) as typeof fetch
  await product.loadModels(true)
  await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
  conversation.draft = 'First message'
  // The record's creation is held; meanwhile the user chooses Review Team.
  let open!: () => void
  holdCreate = new Promise((resolve) => { open = resolve })
  rejectCreate = true
  const sending = store.send(conversation)
  await waitFor(() => requests.some((request) => request.path === '/api/conversations'), () => 'the record was not asked for')
  await store.changeWorkflow(id, { type: 'workflow', id: 'review' })
  expect(requests.filter((request) => request.path === `/api/conversations/${id}`)).toEqual([])
  open()
  await sending
  // It failed: the retry carries the choice made meanwhile.
  expect(conversation.pendingSends[0]?.error).toBe('Something went wrong on the server.')
  const messageId = conversation.pendingSends[0]!.id
  rejectCreate = false
  holdCreate = new Promise((resolve) => { open = resolve })
  const retried = store.retrySends(conversation)
  await waitFor(() => requests.filter((request) => request.path === '/api/conversations').length === 2, () => 'no retry')
  expect((requests.at(-1)!.body as { workflow?: unknown }).workflow).toEqual({ type: 'workflow', id: 'review' })
  // While the retry is held, the user chooses No Subagents: it is written
  // once the record is made, before the message goes to the session.
  await store.changeWorkflow(id, { type: 'none' })
  open()
  await retried
  expect(requests.filter((request) => request.path === `/api/conversations/${id}`).map((request) => request.body))
    .toEqual([{ workflow: { type: 'none' } }])
  expect(records.find((record) => record.id === id)!.workflow).toEqual({ type: 'none' })
  expect(seen).toEqual(['patch', 'submit'])
  submit.mockRestore()
  connect.mockRestore()
})

test('first send failure preserves the conversation and retries its UUID and message', async () => {
  const store = useConversations()
  await store.create()
  const conversation = store.items[0]!
  conversation.draft = 'First message'
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.persistence).toBe('pending')
  expect(conversation.pendingSends.map(({ text, error }) => ({ text, error }))).toEqual([{ text: 'First message', error: 'Something went wrong on the server.' }])
  expect(conversation.draft).toBe('')
  const messageId = conversation.pendingSends[0]!.id
  await reconnected()
  expect(store.items[0]?.id).toBe(conversation.id)
  await store.retrySends(conversation)
  expect(conversation.pendingSends.map((pending) => pending.id)).toEqual([messageId])
  expect(requests.filter((request) => request.path === '/api/conversations').map((request) => (request.body as { id: string }).id)).toEqual([
    conversation.id,
    conversation.id,
  ])
})

// A product check sent the first message of a new conversation right after
// the page loaded, and it failed with "No available model for this
// conversation.": the providers changed while the record was made, and the
// conversation opened on the model list's load that the change replaced.
test('a first send whose conversation opens while the model list loads again goes once the list answers', async () => {
  const store = useConversations()
  const product = useProduct()
  const sync = channels.last()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
  const asked = deferred<void>()
  const answerCreate = deferred<void>()
  /** Each model list the page asked for, answered when the test says. */
  const lists: ReturnType<typeof deferred<Response>>[] = []
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      const list = deferred<Response>()
      // An aborted request ends as the web browser ends it.
      init?.signal?.addEventListener('abort', () => list.reject(init.signal!.reason), { once: true })
      lists.push(list)
      return list.promise
    }
    if (path === '/api/conversations') {
      asked.resolve()
      await answerCreate.promise
    }
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  try {
    const loaded = product.loadModels(true)
    await waitFor(() => lists.length === 1)
    lists[0]!.resolve(Response.json(stubCatalog()))
    await loaded
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    conversation.draft = 'First message'
    const sending = store.send(conversation)
    await asked.promise
    // The entry is renamed while the record is made: the list loads again.
    const renamed = [{ ...stubProvider, label: 'Stub Account' }]
    sync.send({ type: 'providers', providers: renamed })
    await waitFor(() => lists.length === 2)
    answerCreate.resolve()
    // The conversation opens: its socket's connection is made beside its reads.
    await waitFor(() => channels.opened.some((channel) => channel.url.endsWith(`/${id}/stream`)))
    // The entry's account is read again, which loads the list once more.
    sync.send({ type: 'providers', providers: renamed })
    await waitFor(() => lists.length === 3)
    // The replaced load has ended; the event loop turns once, which a send
    // that did not wait for the list needed to fail.
    await new Promise((resolve) => setImmediate(resolve))
    expect(conversation.pendingSends.map((pending) => pending.error)).toEqual([null])
    expect(submit).not.toHaveBeenCalled()
    lists[2]!.resolve(Response.json(stubCatalog()))
    await sending
    expect(conversation.pendingSends.map((pending) => pending.error)).toEqual([])
    expect(submit).toHaveBeenCalledTimes(1)
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

// A model list that fails while a message waits for it is why the message
// did not go, and the message says so.
test('a first send waiting for the model list ends undelivered with the list’s failure when it fails', async () => {
  const store = useConversations()
  const product = useProduct()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
  const asked = deferred<void>()
  const answerCreate = deferred<void>()
  /** Each model list the page asked for, answered when the test says. */
  const lists: ReturnType<typeof deferred<Response>>[] = []
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      const list = deferred<Response>()
      lists.push(list)
      return list.promise
    }
    if (path === '/api/conversations') {
      asked.resolve()
      await answerCreate.promise
    }
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  try {
    const loaded = product.loadModels(true)
    await waitFor(() => lists.length === 1)
    lists[0]!.resolve(Response.json(stubCatalog()))
    await loaded
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    conversation.draft = 'First message'
    const sending = store.send(conversation)
    await asked.promise
    // The entry is renamed while the record is made: the list loads again, and fails.
    channels.last().send({ type: 'providers', providers: [{ ...stubProvider, label: 'Stub Account' }] })
    await waitFor(() => lists.length === 2)
    answerCreate.resolve()
    lists[1]!.resolve(Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Catalog offline' }, { status: 500 }))
    await sending
    expect(conversation.pendingSends.map((pending) => pending.error)).toEqual(['Couldn’t load the models. Something went wrong on the server.'])
    expect(submit).not.toHaveBeenCalled()
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

/**
 * Answers what opening the conversation `id` reads, beside the backend's own
 * routes, after `intercept` had its look at each request; gives the backend
 * its routes back when called.
 */
function answerOpening(id: string, intercept: (path: string) => void = () => {}): () => void {
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    intercept(path)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
    if (path === `/api/conversations/${id}` && init?.method === 'PATCH') {
      return Response.json({ conversation: records.find((item) => item.id === id), results: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  return () => {
    globalThis.fetch = originalFetch
  }
}

/** The conversation infers with the stub model, which the catalog lists. */
async function chooseStubModel(conversation: ReturnType<typeof useConversations>['items'][number]): Promise<void> {
  addStubProvider()
  await useProduct().loadModels(true)
  await useConversations().changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
}

// A message sent while Demi restarts shows as waiting, never as failed, and
// goes with its id once the page reaches the backend again.
test('a first send while the backend is away waits without failing and goes once with its id when it is back', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  let away = false
  let refused = 0
  const restoreFetch = answerOpening(id, (path) => {
    if (away && path === '/api/conversations') {
      refused += 1
      throw new TypeError('Failed to fetch')
    }
  })
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  try {
    await chooseStubModel(conversation)
    conversation.draft = 'Sent while Demi restarts'
    away = true
    channels.last().end(1001, 'backend_closing')
    expect(useProduct().connection).toBe('restarting')
    const sending = store.send(conversation)
    await waitFor(() => refused === 1, () => 'no create request')
    const messageId = conversation.pendingSends[0]?.id
    expect(conversation.pendingSends[0]?.error).toBeNull()
    expect(submit).not.toHaveBeenCalled()
    away = false
    // The page connects again at once rather than after its wait, and the channel's snapshot ends the banner.
    await pageReturned()
    channels.last().connect({ ...backendState(), providers: [stubProvider] })
    await sending
    expect(submit).toHaveBeenCalledTimes(1)
    expect(submit.mock.calls[0]?.[1]).toBe(messageId)
    expect(conversation.pendingSends).toEqual([])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

test('Stop pressed while the first message is on its way stops the turn it starts, once the session holds it', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  const restoreFetch = answerOpening(id)
  const held = deferred<void>()
  const order: string[] = []
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async () => {
    await held.promise
    order.push('submit')
  })
  const abort = spyOn(ConversationRuntime.prototype, 'abort').mockImplementation(async () => {
    order.push('abort')
  })
  try {
    await chooseStubModel(conversation)
    conversation.draft = 'Write a long poem'
    const sending = store.send(conversation)
    const stopping = store.stop(conversation)
    await waitFor(() => submit.mock.calls.length === 1, () => 'the message was not submitted')
    expect(order).toEqual([])
    held.resolve()
    await sending
    await stopping
    await waitFor(() => order.length === 2, () => `order: ${order.join(', ')}`)
    expect(order).toEqual(['submit', 'abort'])
  } finally {
    abort.mockRestore()
    submit.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

/**
 * The keys of the rows the list shows below the turn, as `ChatSession` builds
 * them from what the page passes it (`ChatPage.vue`), and of the turn's own
 * message row on its way.
 */
function shownRows(conversation: ReturnType<typeof useConversations>['items'][number]): { turn: string[]; waiting: string[] } {
  const tail = listTailBlocks({
    phase: conversation.phase,
    blocks: latestBlocks(conversation.history),
    pendingCalls: [],
    pendingSteers: conversation.pendingSteers,
    queue: conversation.queue,
    pendingSubmissions: conversation.pendingSends.map((pending) => ({ ...pending, attachments: [] })),
    removing: conversation.removing,
  })
  return {
    turn: tail.turn.map(rowKey),
    waiting: tail.waiting.filter((row) => row.type !== 'queue_divider').map(rowKey),
  }
}

// Recall or Send Now pressed on a steer still on its way is the user's
// latest wish, carried out once the session holds the steer, on what it
// became: the steer, or the queued message it went as when the session refused it. Recall
// takes the row away at once all the same, and puts the message back into the
// empty composer once it is carried out.
test.each([
  { held: 'steer', press: 'Recall', request: 'removeWaiting' },
  { held: 'steer', press: 'Send Now', request: 'interruptPendingSteer' },
  { held: 'queue', press: 'Recall', request: 'removeWaiting' },
] as const)('$press pressed on a steer on its way is carried out once the session holds it as a $held', async ({ held, press, request }) => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  const restoreFetch = answerOpening(id)
  const holds = deferred<void>()
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const steer = spyOn(ConversationRuntime.prototype, 'steer').mockImplementation(async () => {
    await holds.promise
  })
  // Whether the session held the steer when each request went out.
  const heldAtRequest: boolean[] = []
  const carried = spyOn(ConversationRuntime.prototype, request).mockImplementation(async (messageId: string) => {
    heldAtRequest.push([...conversation.pendingSteers, ...conversation.queue].some((message) => message.id === messageId))
  })
  try {
    await chooseStubModel(conversation)
    conversation.phase = 'running'
    conversation.draft = 'Also check the sign-up page'
    const sending = store.send(conversation, 'steer')
    await waitFor(() => steer.mock.calls.length === 1, () => 'the steer was not sent')
    const messageId = conversation.pendingSends[0]!.id
    // It shows as the steer it will be from the send.
    expect(shownRows(conversation).waiting).toEqual([`message:${messageId}`])
    if (press === 'Recall') {
      store.recall(conversation, messageId)
      expect(shownRows(conversation).waiting).toEqual([])
      // Nothing comes back before the session holds the message.
      expect(conversation.draft).toBe('')
    } else {
      store.interruptWithSteer(conversation, messageId)
    }
    // One task's turn: a request sent at once has reached the runtime by now.
    await new Promise((resolve) => setTimeout(resolve, 0))
    const content = [{ type: 'text' as const, text: 'Also check the sign-up page' }]
    if (held === 'steer') {
      conversation.pendingSteers = [{ id: messageId, content }]
    } else {
      conversation.queue = [{ id: messageId, content }]
    }
    holds.resolve()
    await sending
    await waitFor(() => carried.mock.calls.length === 1, () => `${request} was not called`)
    expect(carried.mock.calls[0]?.[0]).toBe(messageId)
    expect(heldAtRequest).toEqual([true])
    expect(conversation.pendingSends).toEqual([])
    if (press === 'Recall') {
      expect(conversation.draft).toBe('Also check the sign-up page')
    }
  } finally {
    carried.mockRestore()
    steer.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

/** A waiting message with a picture the user's blobs hold (`SHA`) between its two texts. */
function withPicture(id: string) {
  return {
    id,
    content: [
      { type: 'text' as const, text: 'Compare ' },
      { type: 'attachment' as const, name: 'before.png', id: 'a1', mediaType: 'image/png', sizeBytes: 4, sha256: SHA },
      { type: 'text' as const, text: ' with the old one' },
    ],
  }
}

/**
 * The open conversation, running, with the session's answer to a removal
 * held in `removal.answer` and the claims of stored files answered at once;
 * `removal.sendBytes` sees any upload of bytes.
 */
async function withRemovals(check: (
  conversation: ReturnType<typeof useConversations>['items'][number],
  removal: { answer: ReturnType<typeof deferred<void>>; calls: () => number; sendBytes: ReturnType<typeof spyOn> },
) => Promise<void>): Promise<void> {
  const store = useConversations()
  const restoreFetch = answerOpening(FIRST)
  await store.activate(FIRST)
  const conversation = store.items.find((item) => item.id === FIRST)!
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const answer = deferred<void>()
  const remove = spyOn(ConversationRuntime.prototype, 'removeWaiting').mockImplementation(() => answer.promise)
  const claims = deferred()
  claims.resolve()
  const restoreClaims = answerClaims(claims.promise)
  const sendBytes = spyOn(FakeXhr.prototype, 'send')
  try {
    await chooseStubModel(conversation)
    conversation.phase = 'running'
    await check(conversation, { answer, calls: () => remove.mock.calls.length, sendBytes })
  } finally {
    sendBytes.mockRestore()
    restoreClaims()
    remove.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
}

// Recall takes a waiting message back into the empty composer, as
// iMessage's does: its text, and its files as capsules, which attach as a
// pasted message's do, without their bytes. Before, the message was only
// removed.
test('Recall puts a steer back into the empty composer, its file a capsule attached without uploading it', async () => {
  await withRemovals(async (conversation, removal) => {
    const store = useConversations()
    conversation.pendingSteers = [{ ...withPicture('second'), comments: [recalledComment] }]
    const focusRequests = store.composerFocusRequests
    store.recall(conversation, 'second')
    expect(shownRows(conversation).waiting).toEqual([])
    // At once, before the session answers, its comments waiting again.
    expect(conversation.draft).toBe(`Compare ${ATTACHMENT_MARK} with the old one`)
    expect(conversation.comments).toEqual([recalledComment])
    expect(conversation.attachmentIds).toHaveLength(1)
    const file = conversation.files.find((each) => each.id === conversation.attachmentIds[0])
    expect(file).toMatchObject({ name: 'before.png' })
    expect(store.composerFocusRequests).toBe(focusRequests + 1)
    await waitFor(() => removal.calls() === 1, () => 'the removal was not sent')
    removal.answer.resolve()
    await waitFor(() => file?.kind === 'file' && file.phase === 'ready', () => 'the file was not attached')
    expect(requests.some((request) => request.path.includes(`sha256=${SHA}`))).toBe(true)
    expect(removal.sendBytes).not.toHaveBeenCalled()
    expect(toasts.some((toast) => toast.title === 'Message Recalled')).toBe(false)
  })
})

// With a draft in the composer, Recall keeps the draft as it is and the
// message is not put back; a toast's Undo sends it again the way it was
// sent. Its comments go with it: the draft's own waiting comments stay as they
// are, and Undo sends the recalled ones again. Before, they joined the draft's
// and Undo sent the message without them.
const recalledComment = { id: 'c-recalled', blockId: 'text-1', start: 0, end: 7, at: 0, quote: 'Uploads', text: 'Which ones?' }
const ownComment = { id: 'c-own', blockId: 'text-1', start: 8, end: 12, at: 8, quote: 'fail', text: 'Since when?' }
test.each<{ holds: string; draft: string; comments: MessageComment[] }>([
  { holds: 'a draft', draft: 'My own draft', comments: [] },
  { holds: 'a waiting comment', draft: '', comments: [ownComment] },
])('Recall with $holds in the composer keeps it untouched, and the toast\'s Undo sends the message again to the queue, its comments with it', async ({ draft, comments }) => {
  await withRemovals(async (conversation, removal) => {
    const store = useConversations()
    const sent: [ClientContent[], readonly MessageComment[]][] = []
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (content, _id, carried = []) => {
      sent.push([content, carried])
    })
    try {
      conversation.draft = draft
      conversation.comments = comments
      conversation.queue = [{ ...withPicture('queued'), comments: [recalledComment] }]
      store.recall(conversation, 'queued')
      expect(shownRows(conversation).waiting).toEqual([])
      expect(conversation.draft).toBe(draft)
      expect(conversation.comments).toEqual(comments)
      expect(conversation.attachmentIds).toEqual([])
      const recalled = toasts.at(-1)
      expect(recalled).toMatchObject({ title: 'Message Recalled', tone: 'success', action: { label: 'Undo' } })
      removal.answer.resolve()
      // The session no longer lists it.
      conversation.queue = []
      recalled?.action?.run()
      await waitFor(() => sent.length === 1, () => 'the message was not sent again')
      expect(sent[0]).toEqual([[
        { type: 'text', text: 'Compare ' },
        { type: 'upload', ref: 'att-held', fileName: 'before.png' },
        { type: 'text', text: ' with the old one' },
      ], [recalledComment]])
      expect([conversation.draft, conversation.comments]).toEqual([draft, comments])
      expect(removal.sendBytes).not.toHaveBeenCalled()
    } finally {
      submit.mockRestore()
    }
  })
})

// A Recall the agent beat, as when it read the steer meanwhile, is no
// failure: the message shows where the agent took it, with no toast and no
// report of a defect, and what Recall put in the composer goes back out
// unless the user changed it since. Before, it toasted "Could Not Recall the
// Message" and posted it to /api/page-errors.
test.each([
  { since: 'left as it was', draft: '' },
  { since: 'edited', draft: `Compare ${ATTACHMENT_MARK} with the old one, please` },
] as const)('a Recall the agent beat leaves the message in the transcript with no toast and no report, and the composer it refilled, $since, holds "$draft"', async ({ since, draft }) => {
  const stopReports = reportPageErrorsWith(sendPageErrorReport)
  try {
    await withRemovals(async (conversation, removal) => {
      const store = useConversations()
      const steer = (text: string) => ({ id: text, content: [{ type: 'text' as const, text }] })
      conversation.pendingSteers = [steer('first'), withPicture('second'), steer('third')]
      store.recall(conversation, 'second')
      expect(shownRows(conversation).waiting).toEqual(['message:first', 'message:third'])
      const fileIds = [...conversation.attachmentIds]
      if (since === 'edited') {
        conversation.draft += ', please'
      }
      await waitFor(() => removal.calls() === 1, () => 'the removal was not sent')
      // The agent read the steer: the session writes it into the turn.
      conversation.pendingSteers = [steer('first'), steer('third')]
      conversation.history = wholeHistory([{
        type: 'user', id: 'second', turnId: 'running-turn', createdAt: '2026-10-16T00:00:00.000Z', model,
        content: withPicture('second').content, preamble: null,
      }])
      const shown = toasts.length
      removal.answer.reject(new RemovalRefusedError('The agent had already read it.'))
      await waitFor(() => !conversation.removing.includes('second'), () => 'the Recall did not end')
      expect(latestBlocks(conversation.history).map((block) => block.id)).toEqual(['second'])
      expect(shownRows(conversation).waiting).toEqual(['message:first', 'message:third'])
      expect(toasts.slice(shown)).toEqual([])
      expect(requests.filter((request) => request.path === '/api/page-errors')).toEqual([])
      expect(conversation.draft).toBe(draft)
      expect(conversation.attachmentIds).toEqual(since === 'edited' ? fileIds : [])
      expect(conversation.files.some((file) => fileIds.includes(file.id))).toBe(since === 'edited')
    })
  } finally {
    stopReports()
  }
})

// Any other failure of a Recall is one: the row comes back where it was,
// and a toast says why.
test('a Recall that fails brings the row back where it was with a toast', async () => {
  await withRemovals(async (conversation, removal) => {
    const store = useConversations()
    const steer = (text: string) => ({ id: text, content: [{ type: 'text' as const, text }] })
    conversation.pendingSteers = [steer('first'), steer('second')]
    store.recall(conversation, 'second')
    await waitFor(() => removal.calls() === 1, () => 'the removal was not sent')
    removal.answer.reject(new Error('The agent session is closed'))
    await waitFor(() => shownRows(conversation).waiting.length === 2, () => 'the row did not come back')
    expect(toasts.at(-1)).toMatchObject({ title: 'Could Not Recall the Message', message: 'The agent session is closed' })
    expect(conversation.draft).toBe('')
  })
})

// A message can be sent while the one before it is on its way: before, Enter
// did nothing until the first was confirmed. Both show at once, as what they
// will be, and go in order.
test('two messages sent in a row while the first is unconfirmed both show at once, go in order, and each becomes its row in place', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  const restoreFetch = answerOpening(id)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const order: string[] = []
  const submitted = deferred<void>()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (_content, messageId) => {
    order.push(`submit ${messageId}`)
    await submitted.promise
  })
  const steered = deferred<void>()
  const steer = spyOn(ConversationRuntime.prototype, 'steer').mockImplementation(async (_content, messageId) => {
    order.push(`steer ${messageId}`)
    await steered.promise
  })
  try {
    await chooseStubModel(conversation)
    conversation.draft = 'Run the tests'
    const first = store.send(conversation)
    // The composer sends the next one as a steer: the turn is as good as running.
    conversation.draft = 'Skip the e2e tests'
    const second = store.send(conversation, 'steer')
    expect(conversation.draft).toBe('')
    const [one, two] = conversation.pendingSends.map((pending) => pending.id)
    expect(shownRows(conversation)).toEqual({ turn: [`message:${one}`], waiting: [`message:${two}`] })
    await waitFor(() => order.length === 1, () => 'nothing was sent')
    expect(order).toEqual([`submit ${one}`])
    // The session holds the first: its turn's message is written under its id.
    conversation.history = wholeHistory([{
      type: 'user', id: 'u-1', turnId: one!, createdAt: '2026-10-11T00:00:00.000Z', model,
      content: [{ type: 'text', text: 'Run the tests' }], preamble: null,
    }])
    conversation.phase = 'running'
    submitted.resolve()
    await first
    await waitFor(() => order.length === 2, () => 'the second was not sent')
    expect(order).toEqual([`submit ${one}`, `steer ${two}`])
    expect(shownRows(conversation)).toEqual({ turn: [], waiting: [`message:${two}`] })
    // The session holds the second as a steer: the same row.
    conversation.pendingSteers = [{ id: two!, content: [{ type: 'text', text: 'Skip the e2e tests' }] }]
    steered.resolve()
    await second
    expect(conversation.pendingSends).toEqual([])
    expect(shownRows(conversation)).toEqual({ turn: [], waiting: [`message:${two}`] })
  } finally {
    steer.mockRestore()
    submit.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

// Send Now in a comment's box: a message holding that comment alone goes at
// once, as the composer's send would, a steer while the agent works; the other
// waiting comments and the typed draft stay as they are.
test.each([
  { phase: 'idle', way: 'submit' },
  { phase: 'running', way: 'steer' },
] as const)('Send Now on a comment while the agent is $phase sends it alone as a $way and keeps the draft and the other comments', async ({ phase, way }) => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  const restoreFetch = answerOpening(id)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const sent: { way: string; content: unknown; comments: readonly MessageComment[] }[] = []
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (content, _id, comments = []) => {
    sent.push({ way: 'submit', content, comments })
  })
  const steer = spyOn(ConversationRuntime.prototype, 'steer').mockImplementation(async (content, _id, comments = []) => {
    sent.push({ way: 'steer', content, comments })
  })
  try {
    await chooseStubModel(conversation)
    conversation.phase = phase
    const other: MessageComment = { id: 'c1', blockId: 'b1', start: 0, end: 4, at: 0, quote: 'Uplo', text: 'Smaller?' }
    const now: MessageComment = { id: 'c2', blockId: 'b1', start: 0, end: 1, at: 0, quote: 'Chart', picture: { type: 'url', url: 'https://example.com/chart.png' }, text: 'Which one?' }
    conversation.comments = [other]
    conversation.draft = 'My own draft'
    store.commenting(conversation).sendNow!(now)
    await waitFor(() => sent.length === 1, () => 'nothing was sent')
    expect(sent).toEqual([{ way, content: [], comments: [now] }])
    expect(conversation.draft).toBe('My own draft')
    expect(conversation.comments).toEqual([other])
  } finally {
    steer.mockRestore()
    submit.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

// A comment's Send Now the session refuses leaves no failed message: the
// comment waits with the others again, after them, and the typed draft is
// as it was.
test('Send Now on a comment the session refuses puts the comment back with the waiting ones and leaves no failed message', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  const restoreFetch = answerOpening(id)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockRejectedValue(new Error('The model is not available.'))
  const errors = spyOn(console, 'error').mockImplementation(() => {})
  try {
    await chooseStubModel(conversation)
    const other: MessageComment = { id: 'c1', blockId: 'b1', start: 0, end: 4, at: 0, quote: 'Uplo', text: 'Smaller?' }
    const now: MessageComment = { id: 'c2', blockId: 'b1', start: 6, end: 9, at: 6, quote: 'fai', text: 'Since when?' }
    conversation.comments = [other]
    conversation.draft = 'My own draft'
    store.commenting(conversation).sendNow!(now)
    await waitFor(() => conversation.comments.length === 2, () => 'the comment did not come back')
    expect(submit).toHaveBeenCalledTimes(1)
    expect(conversation.pendingSends).toEqual([])
    expect(conversation.comments).toEqual([other, now])
    expect(conversation.draft).toBe('My own draft')
  } finally {
    errors.mockRestore()
    submit.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

// The page hides the composer while a conversation loads: had the first send
// shown the new conversation as loading, the composer the user sent from
// would have gone, and the focus with it.
test('the first send keeps the new conversation shown while its record opens', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
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

test('the first send creates the conversation with its settings and no title in one request and reads nothing of it back', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
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
    // The placeholder goes as no title: it holds no words, so it reads
    // right in whichever language a page shows it later.
    expect(created.map((request) => request.body)).toEqual([{
      id, pinned: false, target: { kind: 'cloud' },
      model: { providerId: 'stub', modelId: 'stub' }, serviceTierId: null,
      // What a new conversation outside projects starts with before any choice.
      workflow: { type: 'none' },
    }])
    expect(conversation.title).toBeNull()
    expect(readBack).toEqual([])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

test('a draft moves without telling anyone and is created where it moved; a conversation with a record asks the backend to tell its agent', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) =>
    String(input).startsWith('/api/models') ? Response.json(stubCatalog()) : originalFetch(input, init)) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  const onLaptop = { kind: 'device' as const, deviceId: 'laptop', path: '/Users/ada/work' }
  try {
    await useProduct().loadModels(true)
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    expect(await store.switchTarget(id, onLaptop, true)).toBe(true)
    conversation.draft = 'First message'
    await store.send(conversation)
    const created = requests.filter((request) => request.path === '/api/conversations')
    expect(created.map((request) => request.body)).toMatchObject([{ target: onLaptop }])
    expect(requests.filter((request) => request.path === '/api/conversations/batch')).toEqual([])

    expect(await store.switchTarget(FIRST, onLaptop, true)).toBe(true)
    expect(await store.switchTarget(FIRST, { kind: 'cloud' })).toBe(true)
    expect(requests.filter((request) => request.path === '/api/conversations/batch').map((request) => request.body)).toEqual([
      { items: [{ id: FIRST, patch: { target: onLaptop, notifyAgent: true } }] },
      { items: [{ id: FIRST, patch: { target: { kind: 'cloud' } } }] },
    ])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
})

// Moving a conversation whose agent
// works stops its work first, and the move goes out once the session says it
// is idle, since the backend refuses a move while its tree is busy.
test('moving a conversation whose agent works stops its work and sends the move only once the session is idle', async () => {
  const store = useConversations()
  const conversation = store.items.find((item) => item.id === FIRST)!
  const onLaptop = { kind: 'device' as const, deviceId: 'laptop', path: '/Users/ada/work' }
  const stopped: string[] = []
  const restoreFetch = answerOpening(FIRST)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const abort = spyOn(ConversationRuntime.prototype, 'abort').mockImplementation(async () => {
    stopped.push('turn')
  })
  const abortTerminal = spyOn(ConversationRuntime.prototype, 'abortTerminal').mockImplementation(async (id) => {
    stopped.push(`command ${id}`)
  })
  try {
    await chooseStubModel(conversation)
    await store.activate(FIRST)
    conversation.phase = 'running'
    conversation.terminals.push({ id: 'build', title: 'Build', phase: 'running', startedAt: '2026-09-09T00:00:00.000Z', output: '' })
    const moving = store.switchTargetAfterStop(conversation, onLaptop, true)
    await waitFor(() => stopped.length === 2, () => `stopped: ${stopped.join(', ')}`)
    expect(stopped.toSorted()).toEqual(['command build', 'turn'])
    expect(batchBodies()).toEqual([])
    // The session's own phase change, which the page hears as the stop lands.
    conversation.phase = 'idle'
    // The command's job holds the conversation until its runner says it ended.
    await nextTick()
    expect(batchBodies()).toEqual([])
    conversation.terminals[0]!.phase = 'aborted'
    expect(await moving).toBe(true)
    expect(batchBodies()).toEqual([{ items: [{ id: FIRST, patch: { target: onLaptop, notifyAgent: true } }] }])
  } finally {
    abortTerminal.mockRestore()
    abort.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

test('a conversation whose turn ended meanwhile moves at once, and one whose stop is refused does not move', async () => {
  const store = useConversations()
  const conversation = store.items.find((item) => item.id === FIRST)!
  const onLaptop = { kind: 'device' as const, deviceId: 'laptop', path: '/Users/ada/work' }
  const restoreFetch = answerOpening(FIRST)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const abort = spyOn(ConversationRuntime.prototype, 'abort').mockRejectedValue(new Error('Agent connection closed'))
  try {
    await chooseStubModel(conversation)
    expect(await store.switchTargetAfterStop(conversation, onLaptop, false)).toBe(true)
    expect(abort).not.toHaveBeenCalled()
    expect(batchBodies()).toEqual([{ items: [{ id: FIRST, patch: { target: onLaptop } }] }])

    conversation.phase = 'running'
    expect(await store.switchTargetAfterStop(conversation, { kind: 'cloud' }, false)).toBe(false)
    expect(batchBodies()).toHaveLength(1)
  } finally {
    conversation.phase = 'idle'
    abort.mockRestore()
    connect.mockRestore()
    restoreFetch()
  }
})

test('a detach shows the hosts without the one it removed, with no read', async () => {
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  first.attachedHosts.replace([{ deviceId: 'laptop', name: 'laptop', cwd: null, state: 'online', attachedAt: '2026-09-09T00:00:00.000Z' }])
  const originalFetch = globalThis.fetch
  const sent: string[] = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const method = init?.method ?? 'GET'
    sent.push(`${method} ${path}`)
    if (path === `/api/conversations/${FIRST}/hosts/laptop` && method === 'DELETE') return new Response(null, { status: 204 })
    return originalFetch(input, init)
  }) as typeof fetch
  try {
    await store.detachHost(first, 'laptop')
    expect(first.attachedHosts.shown.value).toEqual([])
    expect(sent).toEqual([`DELETE /api/conversations/${FIRST}/hosts/laptop`])
  } finally {
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
  expect(conversation.pendingSends[0]?.fileIds).toEqual([conversation.files[0]!.id])
})

// A failed upload is told on its capsule, with Retry, where it happened: a
// toast, which stays until closed, would outlive the conversation it was about.
test('a file whose upload fails says so on its capsule, whose Retry uploads it, and raises no toast', async () => {
  const store = useConversations()
  store.create()
  const conversation = store.items[0]!
  FakeXhr.failures = 1
  const [file] = store.addFiles(conversation, [new File(['Local notes'], 'notes.txt', { type: 'text/plain' })])
  store.arrangeFiles(conversation, [file!.id])
  const phase = () => (file?.kind === 'file' ? file.phase : null)
  await waitFor(() => phase() === 'failed', () => 'the upload did not fail')
  store.retryFile(conversation, file!.id)
  await waitFor(() => phase() === 'ready', () => 'Retry did not upload the file')
  expect(toasts.filter((toast) => toast.title === 'Could Not Upload the Attachment')).toEqual([])
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
  addStubProvider()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json(stubCatalog())
    }
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
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

/**
 * Runs `check` on the first conversation with its session opened over stubs,
 * the content its messages deliver collected in `sent`.
 */
async function withSession(check: (current: ReturnType<typeof useConversations>['items'][number], sent: ClientContent[][]) => Promise<void>): Promise<void> {
  const store = useConversations()
  const current = store.items[0]!
  addStubProvider()
  const restore = answerOpening(current.id)
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const sent: ClientContent[][] = []
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (content) => {
    sent.push(content)
  })
  try {
    await useProduct().loadModels(true)
    await check(current, sent)
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    restore()
  }
}

/** Puts a picture into the conversation's message after `text`, as the composer does with a pasted one. */
function pastePicture(current: ReturnType<typeof useConversations>['items'][number], text: string) {
  const [file] = useConversations().addFiles(current, [new File(['shot'], 'shot.png', { type: 'image/png' })])
  useConversations().arrangeFiles(current, [file!.id])
  current.draft = `${text}${ATTACHMENT_MARK}`
  return file!
}

// A message whose file still uploads is sent like any other, and goes once
// the upload is done.
test('a message sent while its file uploads leaves the composer at once and is delivered with the file once the upload finishes', async () => {
  await withSession(async (current, sent) => {
    const gate = deferred()
    FakeXhr.gate = gate.promise
    const file = pastePicture(current, 'What is wrong here? ')
    const sending = useConversations().send(current)
    expect(current.draft).toBe('')
    expect(current.pendingSends.map((pending) => toRaw(pending))).toMatchObject([{ fileIds: [file.id], error: null }])
    expect(useConversations().waitingForUploads).toBe(true)
    // The message waits for its file: nothing goes while the upload runs.
    await waitFor(() => requests.some((request) => request.path.startsWith('/api/attachments?')))
    expect(sent).toEqual([])
    gate.resolve()
    await sending
    expect(sent).toEqual([[
      { type: 'text', text: 'What is wrong here? ' },
      { type: 'upload', ref: expect.stringMatching(/^att-/), fileName: 'shot.png' },
    ]])
    expect(current.pendingSends).toEqual([])
    expect(useConversations().waitingForUploads).toBe(false)
  })
})

// Two screenshots pasted one after the other are both `image.png`; a name
// says nothing about a file's contents.
test('two files of one name both join the message, upload, and go as two files', async () => {
  await withSession(async (current, sent) => {
    const taken = useConversations().addFiles(current, [
      new File(['first shot'], 'image.png', { type: 'image/png' }),
      new File(['second shot'], 'image.png', { type: 'image/png' }),
    ])
    expect(taken.map((file) => file.name)).toEqual(['image.png', 'image.png'])
    useConversations().arrangeFiles(current, taken.map((file) => file.id))
    current.draft = `Before ${ATTACHMENT_MARK} and after ${ATTACHMENT_MARK}`
    await useConversations().send(current)
    const uploads = sent[0]!.filter((part) => part.type === 'upload')
    expect(uploads).toEqual([
      { type: 'upload', ref: expect.stringMatching(/^att-/), fileName: 'image.png' },
      { type: 'upload', ref: expect.stringMatching(/^att-/), fileName: 'image.png' },
    ])
    expect(new Set(uploads.map((part) => part.ref)).size).toBe(2)
  })
})

test('a message whose upload fails is not delivered and keeps its file, and Retry uploads it again and delivers it', async () => {
  await withSession(async (current, sent) => {
    const gate = deferred()
    FakeXhr.gate = gate.promise
    FakeXhr.failures = 1
    const file = pastePicture(current, 'What is wrong here? ')
    const sending = useConversations().send(current)
    gate.resolve()
    await sending
    expect(sent).toEqual([])
    expect(current.pendingSends[0]?.error).toMatch(/^shot\.png did not upload\./)
    expect(current.files.map((each) => each.id)).toEqual([file.id])
    // Retry: the upload goes again, then the message.
    await useConversations().retrySends(current)
    expect(sent).toEqual([[
      { type: 'text', text: 'What is wrong here? ' },
      { type: 'upload', ref: expect.stringMatching(/^att-/), fileName: 'shot.png' },
    ]])
    expect(current.pendingSends).toEqual([])
  })
})

// The failure under a message not delivered is in the page's language, the
// file's name with it (`localization.md`). Before, its first sentence was
// English on every page.
test('a message whose upload fails says so in the page’s language', async () => {
  try {
    createPageI18n('zh-Hans')
    await withSession(async (current) => {
      const gate = deferred()
      FakeXhr.gate = gate.promise
      FakeXhr.failures = 1
      pastePicture(current, 'What is wrong here? ')
      const sending = useConversations().send(current)
      gate.resolve()
      await sending
      expect(current.pendingSends[0]?.error).toMatch(/^shot\.png 未能上传。/)
    })
  } finally {
    createPageI18n('en')
  }
})

// A reload shows a message not delivered as the page showed it before: its file
// whose upload failed is a failed capsule, and nothing uploads it until the
// dock's Retry. Before, the page uploaded it again on its own, and its capsule
// showed a plain file under a failure that said it did not upload.
test('a message whose upload failed keeps its file failed after a reload, and nothing uploads it until Retry', async () => {
  let signOut = signIn()
  let kept: draftStorage.SavedDraft | null = null
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) kept = structuredClone(draft)
  })
  try {
    await withSession(async (current) => {
      await useConversations().activate(FIRST)
      const gate = deferred()
      FakeXhr.gate = gate.promise
      FakeXhr.failures = 1
      pastePicture(current, 'What is wrong here? ')
      const sending = useConversations().send(current)
      gate.resolve()
      await sending
      FakeXhr.gate = null
      expect(current.pendingSends[0]?.error).toMatch(/^shot\.png did not upload\./)
    })
    writes.mockRestore()
    signOut = await reopenWith(signOut, kept!)
    await withSession(async (current, sent) => {
      const uploads = spyOn(FakeXhr.prototype, 'send')
      try {
        const store = useConversations()
        await store.activate(FIRST)
        const file = current.files.find((each) => each.id === current.pendingSends[0]?.fileIds[0])
        expect(file?.kind === 'file' ? file.phase : file).toBe('failed')
        expect(uploads).not.toHaveBeenCalled()
        await store.retrySends(current)
        expect(uploads).toHaveBeenCalledTimes(1)
        expect(sent).toHaveLength(1)
        expect(file?.kind === 'file' ? file.phase : file).toBe('ready')
      } finally {
        uploads.mockRestore()
      }
    })
  } finally {
    writes.mockRestore()
    signOut()
  }
})

// The backend decides that a delivery failed and says why; the bubble says
// it as the page words the refusal's code, and a failure inside the backend,
// which has no cause of its own, with the request to quote. Before, a slow send
// read as a failure the page made up after 30 seconds, with no cause and no
// request.
test('a message the backend refused for a cause of its own is not delivered, and says why', async () => {
  await withSession(async (current) => {
    const refusal = new RefusedError({
      code: 'device_offline',
      reason: 'Referenced device laptop is offline',
      requestId: 'req_offline',
    })
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockRejectedValue(refusal)
    try {
      pastePicture(current, 'What is wrong here? ')
      await useConversations().send(current)
      expect(current.pendingSends.map((pending) => pending.error)).toEqual(['The device is offline.'])
    } finally {
      submit.mockRestore()
    }
  })
})

test('a message the backend failed to send says so with the request ID, which Retry clears', async () => {
  await withSession(async (current) => {
    const refusal = new RefusedError({
      code: 'send_failed',
      reason: 'database is locked',
      requestId: 'req_ffb05w4w6kxpdtz0',
    })
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockRejectedValueOnce(refusal)
    try {
      pastePicture(current, 'What is wrong here? ')
      await useConversations().send(current)
      expect(current.pendingSends.map(({ error, requestId }) => ({ error, requestId }))).toEqual([
        { error: 'The message couldn’t be sent.', requestId: 'req_ffb05w4w6kxpdtz0' },
      ])
      submit.mockResolvedValueOnce(undefined)
      const retried = useConversations().retrySends(current)
      expect(current.pendingSends[0]?.requestId).toBeUndefined()
      await retried
    } finally {
      submit.mockRestore()
    }
  })
})

// A message not delivered stays where it was sent: it keeps the messages sent
// after it, in order, before the first of which that shows it stands, and one
// deleted leaves the list.
test('a message not delivered keeps the messages sent after it in order, without one deleted', async () => {
  await withSession(async (current) => {
    const store = useConversations()
    const submit = spyOn(ConversationRuntime.prototype, 'submit')
      .mockRejectedValueOnce(new RefusedError({ code: 'device_offline', reason: 'offline' }))
      .mockRejectedValueOnce(new RefusedError({ code: 'device_offline', reason: 'offline' }))
      .mockResolvedValueOnce(undefined)
    try {
      current.draft = 'First'
      await store.send(current)
      current.draft = 'Second'
      await store.send(current)
      current.draft = 'Third'
      await store.send(current)
      const [first, second] = store.submissions(current)
      const third = submit.mock.calls[2]![1]!
      expect([first?.followers, second?.followers]).toEqual([[second!.id, third], [third]])
      store.deleteSend(current, second!.id)
      expect(store.submissions(current).map(({ text, followers }) => ({ text, followers }))).toEqual([{ text: 'First', followers: [third] }])
    } finally {
      submit.mockRestore()
    }
  })
})

// A message sent after one not delivered, which waits in the queue and is
// taken back, leaves the order only once its removal is done, and the order
// is saved with the page's draft; a removal refused leaves it in place.
// Before, the message not delivered lost its place to the one taken back,
// and stood after the next message and its answer, also after a reload.
test.each([
  ['done', ['third']],
  ['refused', ['queued', 'third']],
] as const)('Recall on a message queued after one not delivered, its removal %s, keeps the order', async (removal, followers) => {
  const signOut = signIn()
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async () => {})
  try {
    await withRemoval(removal, followers, writes)
  } finally {
    writes.mockRestore()
    signOut()
  }
})

async function withRemoval(removal: 'done' | 'refused', followers: readonly string[], writes: ReturnType<typeof spyOn<typeof draftStorage, 'writeDraft'>>): Promise<void> {
  await withSession(async (current) => {
    const store = useConversations()
    const answer = deferred<void>()
    const remove = spyOn(ConversationRuntime.prototype, 'removeWaiting').mockImplementation(() => answer.promise)
    const submit = spyOn(ConversationRuntime.prototype, 'submit')
      .mockRejectedValueOnce(new RefusedError({ code: 'device_offline', reason: 'offline' }))
      .mockResolvedValue(undefined)
    try {
      // Opened, its draft is this web browser's to save.
      await store.activate(FIRST)
      current.draft = 'First'
      await store.send(current)
      current.phase = 'running'
      current.draft = 'Queued'
      await store.send(current, 'queue')
      const queued = submit.mock.calls[1]![1]!
      current.queue = [{ id: queued, content: [{ type: 'text', text: 'Queued' }] }]
      current.draft = 'Third'
      await store.send(current, 'queue')
      const third = submit.mock.calls[2]![1]!
      const named = (ids: readonly string[]) => ids.map((id) => id === queued ? 'queued' : id === third ? 'third' : id)
      store.recall(current, queued)
      await waitFor(() => remove.mock.calls.length === 1, () => 'the removal was not sent')
      // While the removal is on its way, the order holds the message.
      expect(named(store.submissions(current)[0]!.followers!)).toEqual(['queued', 'third'])
      writes.mockClear()
      if (removal === 'done') {
        answer.resolve()
      } else {
        answer.reject(new RemovalRefusedError('The agent had already read it.'))
      }
      await waitFor(() => !current.removing.includes(queued), () => 'the removal did not end')
      expect(named(store.submissions(current)[0]!.followers!)).toEqual([...followers])
      if (removal === 'done') {
        await waitFor(() => writes.mock.calls.some(([, , draft]) => named(draft.pendingSends[0]?.followers ?? []).join() === 'third'), () => 'the order was not saved')
      }
    } finally {
      submit.mockRestore()
      remove.mockRestore()
    }
  })
}

// Retry while a message is on its way sends the messages not delivered
// after it, never ahead of it: each waits in the queue.
test('Retry while a message is on its way sends the one not delivered after it, in the queue', async () => {
  await withSession(async (current) => {
    const store = useConversations()
    const answer = deferred<void>()
    const sent: string[] = []
    const submit = spyOn(ConversationRuntime.prototype, 'submit')
      .mockRejectedValueOnce(new RefusedError({ code: 'device_offline', reason: 'offline' }))
      .mockImplementationOnce(async (content) => {
        sent.push('Second')
        await answer.promise
      })
      .mockImplementation(async (content) => {
        sent.push(content.map((part) => part.type === 'text' ? part.text : part.type).join(''))
      })
    try {
      current.draft = 'First'
      await store.send(current)
      current.draft = 'Second'
      const sending = store.send(current)
      await waitFor(() => sent.length === 1, () => 'the second message did not go')
      const retried = store.retrySends(current)
      expect(current.pendingSends.map(({ text, way, error }) => ({ text, way, error }))).toEqual([
        { text: 'Second', way: 'send', error: null },
        { text: 'First', way: 'queue', error: null },
      ])
      expect(sent).toEqual(['Second'])
      answer.resolve()
      await Promise.all([sending, retried])
      expect(sent).toEqual(['Second', 'First'])
      expect(current.pendingSends).toEqual([])
    } finally {
      submit.mockRestore()
    }
  })
})

// The dock's Retry sends every message not delivered again, in the order
// they were sent, each as a new message at the end, and skips one a resend
// cannot deliver, which leaves no Retry.
test('Retry sends the messages not delivered again in order, each at the end, and skips one whose file is gone', async () => {
  await withSession(async (current) => {
    const store = useConversations()
    const answer = deferred<void>()
    const sent: string[] = []
    let refuse = true
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(async (content) => {
      if (refuse) {
        throw new RefusedError({ code: 'device_offline', reason: 'offline' })
      }
      sent.push(content.map((part) => part.type === 'text' ? part.text : part.type).join(''))
      await answer.promise
    })
    try {
      current.draft = 'First'
      await store.send(current)
      // Its text holds a place for a file it has none for: no resend delivers it.
      current.draft = `Compare ${ATTACHMENT_MARK}`
      await store.send(current)
      current.draft = 'Third'
      await store.send(current)
      expect(store.submissions(current).map(({ text, retryable }) => ({ text, retryable }))).toEqual([
        { text: 'First', retryable: undefined },
        { text: `Compare ${ATTACHMENT_MARK}`, retryable: false },
        { text: 'Third', retryable: undefined },
      ])
      refuse = false
      const retried = store.retrySends(current)
      expect(current.pendingSends.map(({ text, way, error }) => ({ text, way, delivering: error === null }))).toEqual([
        { text: `Compare ${ATTACHMENT_MARK}`, way: 'send', delivering: false },
        { text: 'First', way: 'send', delivering: true },
        { text: 'Third', way: 'queue', delivering: true },
      ])
      answer.resolve()
      await retried
      expect(sent).toEqual(['First', 'Third'])
      expect(store.submissions(current).map(({ text, retryable }) => ({ text, retryable }))).toEqual([{ text: `Compare ${ATTACHMENT_MARK}`, retryable: false }])
    } finally {
      submit.mockRestore()
    }
  })
})

test('a message never goes without a file its text holds a place for', async () => {
  await withSession(async (current, sent) => {
    // The text holds a capsule's mark, but the message names no file for it.
    current.draft = `Compare ${ATTACHMENT_MARK} with the old one`
    await useConversations().send(current)
    expect(sent).toEqual([])
    expect(current.pendingSends[0]?.error).toMatch(/file of this message/)
    expect(current.pendingSends[0]?.text).toBe(`Compare ${ATTACHMENT_MARK} with the old one`)
  })
})

/**
 * The page closes and opens again with `kept` as this web browser's draft of
 * the first conversation, as a reload does; answers the sign-out of the new page.
 */
async function reopenWith(signOut: () => void, kept: draftStorage.SavedDraft): Promise<() => void> {
  signOut()
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  const next = signIn(kept)
  // The store is there when the state comes, as the page's is.
  useConversations()
  await connect()
  return next
}

/** Two pictures of one name around a line of text, uploaded, as a user pastes them one after the other. */
async function pasteTwoPictures(current: ReturnType<typeof useConversations>['items'][number]) {
  const taken = useConversations().addFiles(current, [
    new File(['first shot'], 'image.png', { type: 'image/png' }),
    new File(['second shot'], 'image.png', { type: 'image/png' }),
  ])
  useConversations().arrangeFiles(current, taken.map((file) => file.id))
  current.draft = `${ATTACHMENT_MARK} between ${ATTACHMENT_MARK}`
  await until(() => taken.every((file) => file.kind === 'file' && file.upload !== null), 'both files upload')
}

// A reload at any moment of a send restores the message on its way with
// each file once, and it goes again by itself. Before, the draft saved halfway
// through the send held the message both in the composer and on its way,
// each file twice, and the restored message failed every Retry with "A file
// of this message is no longer available".
test('a message restored from any draft its send saved goes again with each of its files once', async () => {
  let signOut = signIn()
  const snapshots: draftStorage.SavedDraft[] = []
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) snapshots.push(structuredClone(draft))
  })
  try {
    await withSession(async (current) => {
      await useConversations().activate(FIRST)
      await pasteTwoPictures(current)
      snapshots.length = 0
      await useConversations().send(current)
    })
    writes.mockRestore()
    const sending = snapshots.filter((snapshot) => snapshot.pendingSends.length === 1)
    expect(sending.length).toBeGreaterThan(0)
    for (const kept of sending) {
      expect(kept.files.map((file) => file.id)).toEqual([...new Set(kept.files.map((file) => file.id))])
      signOut = await reopenWith(signOut, kept)
      await withSession(async (current, sent) => {
        await useConversations().activate(FIRST)
        await until(() => current.pendingSends.length === 0, 'the message goes again')
        expect(sent.map((content) => content.filter((part) => part.type === 'upload').length)).toEqual([2])
      })
    }
  } finally {
    writes.mockRestore()
    signOut()
  }
})

// A reload before the page heard the backend's answer to a message: the
// backend may already hold it, as a queued message the page's history does
// not show yet. The message stays on its way, with no Edit or Delete, which
// would drop the id the backend knows it by, and goes again with that id
// once the session is open, which takes it once. Before, it came back as not
// delivered, and Edit left a copy to send again while Delete let the original
// run.
test('a message whose answer a reload cut off stays on its way, offers no Edit or Delete, and goes again with its id', async () => {
  let signOut = signIn()
  let kept: draftStorage.SavedDraft | null = null
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) kept = structuredClone(draft)
  })
  try {
    let firstId = ''
    await withSession(async (current) => {
      // The backend takes the message; its answer never reaches this page.
      const unheard = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(() => new Promise(() => {}))
      try {
        await useConversations().activate(FIRST)
        current.draft = 'Then run the whole suite'
        void useConversations().send(current)
        await until(() => unheard.mock.calls.length === 1, 'the message is given to the session')
        firstId = current.pendingSends[0]!.id
      } finally {
        unheard.mockRestore()
      }
    })
    writes.mockRestore()
    signOut = await reopenWith(signOut, kept!)
    await withSession(async (current) => {
      const answer = deferred<void>()
      const again = spyOn(ConversationRuntime.prototype, 'submit').mockImplementation(() => answer.promise)
      try {
        const store = useConversations()
        await store.activate(FIRST)
        await until(() => again.mock.calls.length === 1, 'the message goes again once the session is open')
        expect(again.mock.calls[0]![1]).toBe(firstId)
        expect(store.submissions(current).map(({ id, error }) => ({ id, error }))).toEqual([{ id: firstId, error: null }])
        store.editSend(current, firstId)
        store.deleteSend(current, firstId)
        expect(current.pendingSends.map((pending) => pending.id)).toEqual([firstId])
        expect(current.draft).toBe('')
        answer.resolve()
        await until(() => current.pendingSends.length === 0, 'the session takes it')
      } finally {
        again.mockRestore()
      }
    })
  } finally {
    writes.mockRestore()
    signOut()
  }
})

// A page reloaded right after Send, while the message's files still upload:
// the IndexedDB write of the send, which carries the files, is cut off with
// the page, and the record still holds the composer as it was before. The
// page's last words name the message on its way, which the next page sends
// with its files, its composer empty. Before, the last words held only the
// empty composer, which emptied the restored one, and the message was gone.
test('a message sent just before a reload that cut off its stored draft goes after the page opens again', async () => {
  let signOut = signIn()
  const snapshots: draftStorage.SavedDraft[] = []
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) snapshots.push(structuredClone(draft))
  })
  const left = spyOn(draftStorage, 'writeLastWords').mockImplementation(() => {})
  let before: draftStorage.SavedDraft | undefined
  let words: draftStorage.LastWords | undefined
  try {
    await withSession(async (current) => {
      const store = useConversations()
      await store.activate(FIRST)
      // The uploads go on slowly: the message is sent before they end.
      FakeXhr.gate = new Promise(() => {})
      const taken = store.addFiles(current, [
        new File(['first shot'], 'image.png', { type: 'image/png' }),
        new File(['second shot'], 'image.png', { type: 'image/png' }),
      ])
      store.arrangeFiles(current, taken.map((file) => file.id))
      current.draft = `${ATTACHMENT_MARK} between ${ATTACHMENT_MARK}`
      store.saveDrafts()
      before = snapshots.at(-1)
      expect(before?.files.length).toBe(2)
      void store.send(current)
      expect(current.pendingSends.length).toBe(1)
      store.keepLastWords()
      words = left.mock.calls.find(([, id]) => id === FIRST)?.[2]
    })
    writes.mockRestore()
    FakeXhr.gate = null
    // The next page reads the record the send's write never replaced.
    signOut = await reopenWith(signOut, before!)
    const taken = spyOn(draftStorage, 'takeLastWords').mockReturnValue(words ?? null)
    try {
      await withSession(async (current, sent) => {
        await useConversations().activate(FIRST)
        await until(() => sent.length === 1, 'the message goes')
        expect(sent[0]!.filter((part) => part.type === 'upload').length).toBe(2)
        expect(sent[0]!.some((part) => part.type === 'text' && part.text.includes('between'))).toBe(true)
        await until(() => current.pendingSends.length === 0, 'the session takes it')
        expect([current.draft, current.attachmentIds]).toEqual(['', []])
      })
    } finally {
      taken.mockRestore()
    }
  } finally {
    writes.mockRestore()
    left.mockRestore()
    signOut()
  }
})

// The same reload where the backend's draft still holds the message's text,
// saved while it was typed: the next page's composer stays empty, as it was
// after Send, and once the message is taken the backend's draft is emptied
// too; a sent message's text never comes back. Before, the draft read put the
// text back in the composer, since an empty composer counted as no change, and
// it was saved again.
test('after a reload that cut off a send, the composer stays empty and the backend draft is emptied once the message is taken', async () => {
  const typed = 'Compare these'
  serverDrafts.set(FIRST, { revision: 1, text: typed, files: [], comments: [], replaced: null })
  records[0]!.draftRevision = 1
  let signOut = signIn()
  const snapshots: draftStorage.SavedDraft[] = []
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) snapshots.push(structuredClone(draft))
  })
  const left = spyOn(draftStorage, 'writeLastWords').mockImplementation(() => {})
  let before: draftStorage.SavedDraft | undefined
  let words: draftStorage.LastWords | undefined
  try {
    await withSession(async (current) => {
      const store = useConversations()
      await store.activate(FIRST)
      await until(() => current.draft === typed, 'the composer shows the typed draft')
      FakeXhr.gate = new Promise(() => {})
      const [file] = store.addFiles(current, [new File(['shot'], 'image.png', { type: 'image/png' })])
      store.arrangeFiles(current, [file!.id])
      current.draft = `${typed} ${ATTACHMENT_MARK}`
      store.saveDrafts()
      before = snapshots.at(-1)
      void store.send(current)
      store.keepLastWords()
      words = left.mock.calls.find(([, id]) => id === FIRST)?.[2]
    })
    writes.mockRestore()
    FakeXhr.gate = null
    requests = []
    signOut = await reopenWith(signOut, before!)
    const taken = spyOn(draftStorage, 'takeLastWords').mockReturnValue(words ?? null)
    try {
      await withSession(async (current, sent) => {
        await useConversations().activate(FIRST)
        await until(() => sent.length === 1 && current.pendingSends.length === 0, 'the message goes and is taken')
        expect(current.draft).toBe('')
        await until(() => draftSaves().length > 0, 'the emptied draft is saved')
        expect(draftSaves().map((request) => (request.body as { text: string }).text)).toEqual([''])
      })
    } finally {
      taken.mockRestore()
    }
  } finally {
    writes.mockRestore()
    left.mockRestore()
    signOut()
  }
})

// A message whose text and files are out of step, whichever way, fails every
// resend: its row offers no Retry, and Edit puts it back in the composer,
// each place its text holds a capsule, a file gone a capsule to delete or
// attach again, so nothing of it is lost.
test('a message whose files are out of step offers no Retry, and Edit puts it back in the composer with each file gone a capsule that says so', async () => {
  const cases: {
    name: string
    change: (pending: draftStorage.SavedDraft['pendingSends'][number], draft: draftStorage.SavedDraft) => void
    capsules: ('ready' | 'missing')[]
  }[] = [
    { name: 'a file it names is not kept', change: (_pending, draft) => { draft.files = draft.files.slice(0, 1) }, capsules: ['ready', 'missing'] },
    { name: 'its text holds more places than it names files', change: (pending, draft) => { pending.fileIds = pending.fileIds.slice(0, 1); draft.files = draft.files.slice(0, 1) }, capsules: ['ready', 'missing'] },
    { name: 'it names more files than its text holds places', change: (pending) => { pending.text = `${ATTACHMENT_MARK} between` }, capsules: ['ready', 'ready'] },
    { name: 'a file it names has no upload and no bytes', change: (_pending, draft) => { draft.files = draft.files.map((file, index) => index === 1 && file.kind === 'file' ? { ...file, upload: null, file: null } : file) }, capsules: ['ready', 'missing'] },
  ]
  let signOut = signIn()
  let kept: draftStorage.SavedDraft | null = null
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    if (id === FIRST) kept = structuredClone(draft)
  })
  try {
    await withSession(async (current) => {
      const submit = spyOn(ConversationRuntime.prototype, 'submit').mockRejectedValue(new RefusedError({ code: 'device_offline', reason: 'offline' }))
      try {
        await useConversations().activate(FIRST)
        await pasteTwoPictures(current)
        await useConversations().send(current)
        expect(current.pendingSends[0]?.error).toBe('The device is offline.')
      } finally {
        submit.mockRestore()
      }
    })
    writes.mockRestore()
    const undelivered: draftStorage.SavedDraft = kept!
    for (const { name, change, capsules } of cases) {
      const draft = structuredClone(undelivered)
      change(draft.pendingSends[0]!, draft)
      // Each case's page opens on a backend that holds no draft of the one before.
      serverDrafts = new Map()
      signOut = await reopenWith(signOut, draft)
      await withSession(async (current, sent) => {
        const store = useConversations()
        await store.activate(FIRST)
        current.draft = 'Already typed'
        const id = current.pendingSends[0]!.id
        expect(store.submissions(current)[0]?.retryable, name).toBe(false)
        await store.retrySends(current)
        expect(current.pendingSends.map((pending) => pending.id), name).toEqual([id])
        store.editSend(current, id)
        expect(current.pendingSends, name).toEqual([])
        expect(current.draft.startsWith('Already typed\n\n'), name).toBe(true)
        expect(current.draft.split(ATTACHMENT_MARK).length - 1, name).toBe(capsules.length)
        expect(current.attachmentIds.map((fileId) => {
          const file = current.files.find((each) => each.id === fileId)
          return file?.kind === 'file' ? file.phase : null
        }), name).toEqual(capsules)
        // A file gone keeps the message in the composer until it is deleted or attached again.
        await store.send(current)
        expect(sent.length, name).toBe(capsules.includes('missing') ? 0 : 1)
      })
    }
  } finally {
    writes.mockRestore()
    signOut()
  }
})

test('Delete on a message not delivered takes its row and its files away', async () => {
  await withSession(async (current) => {
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockRejectedValue(new RefusedError({ code: 'device_offline', reason: 'offline' }))
    try {
      pastePicture(current, 'What is wrong here? ')
      await useConversations().send(current)
      useConversations().deleteSend(current, current.pendingSends[0]!.id)
      expect(current.pendingSends).toEqual([])
      expect(current.files).toEqual([])
    } finally {
      submit.mockRestore()
    }
  })
})

/** Answers claims of stored files, once `gate` opens, with an upload of the blob `SHA` names; any other the user's blobs do not hold. */
function answerClaims(gate: Promise<void>): () => void {
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/attachments?')) {
      requests.push({ path, body: null })
      await gate
      if (new URLSearchParams(path.split('?')[1]).get('sha256') === SHA) {
        return Response.json({ attachment: {
          id: 'att-held', mediaType: 'image/png', sizeBytes: 4, sha256: SHA, createdAt: '2026-09-09T00:00:00.000Z',
        } }, { status: 201 })
      }
    }
    return originalFetch(input, init)
  }) as typeof fetch
  return () => {
    globalThis.fetch = originalFetch
  }
}

// A pasted message's stored file is a capsule at once, attaching beside it
// as an upload does; a message sent meanwhile waits for it.
test('a pasted message\'s stored file is the message\'s at once, and a send waits for its attaching, which sends no bytes', async () => {
  await withSession(async (current, sent) => {
    const gate = deferred()
    const restore = answerClaims(gate.promise)
    const sendBytes = spyOn(FakeXhr.prototype, 'send')
    try {
      const store = useConversations()
      const { item, kept } = store.claimFile(current, { type: 'stored', name: 'before.png', sha256: SHA, mediaType: 'image/png' }, null)
      // Before the backend answers, the composer carries it, attaching.
      expect(item).toMatchObject({ name: 'before.png', phase: 'attaching', upload: null })
      store.arrangeFiles(current, [item.id])
      current.draft = `Compare ${ATTACHMENT_MARK}`
      const sending = store.send(current)
      expect(current.draft).toBe('')
      expect(sent).toEqual([])
      gate.resolve()
      await sending
      expect(await kept).toBe(true)
      expect(sent).toEqual([[
        { type: 'text', text: 'Compare ' },
        { type: 'upload', ref: 'att-held', fileName: 'before.png' },
      ]])
      expect(sendBytes).not.toHaveBeenCalled()
    } finally {
      sendBytes.mockRestore()
      restore()
    }
  })
})

test('a pasted file the user\'s blobs do not hold uploads the copy\'s picture in its place, or else is not kept', async () => {
  const gate = deferred()
  gate.resolve()
  const restore = answerClaims(gate.promise)
  try {
    const store = useConversations()
    const current = store.items[0]!
    const gone = { type: 'stored' as const, name: 'gone.png', sha256: 'c'.repeat(64), mediaType: 'image/png' }
    const withPicture = store.claimFile(current, gone, new File(['png'], 'gone.png', { type: 'image/png' }))
    expect(await withPicture.kept).toBe(true)
    expect(withPicture.item).toMatchObject({ phase: 'ready', upload: { id: expect.stringMatching(/^att-/) } })
    const without = store.claimFile(current, { ...gone, name: 'other.png' }, null)
    expect(await without.kept).toBe(false)
    expect(without.item).toMatchObject({ phase: 'failed', upload: null })
  } finally {
    restore()
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
  current.history = wholeHistory([
    { type: 'text', id: 'block', createdAt: '2026-09-09T00:00:00.000Z', model, text: 'streaming' },
  ])
  records[0]!.title = 'Server title'
  records.reverse()
  await reconnected()
  expect(store.items[1]).toBe(current)
  expect(current.title).toBe('Server title')
  expect(current.draft).toBe('Still editing')
  expect(latestBlocks(current.history)[0]?.id).toBe('block')
  expect(current.phase).toBe('running')
})

test('history remains readable during its own connection after navigation', async () => {
  const store = useConversations()
  const current = store.items[0]!
  const cached = [{ type: 'text' as const, id: 'cached', createdAt: '2026-09-09T00:00:00.000Z', model, text: 'cached' }]
  current.history = wholeHistory(cached)
  current.draft = 'Keep the draft'
  addStubProvider()
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
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
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
    history.resolve(Response.json({ start: 0, length: 1, blocks: cached, instructions: [], summaries: [] }))
    await connecting.promise
    expect(current).toMatchObject({ load: 'ready' })
    expect(current.draft).toBe('Keep the draft')
    connection.reject(new Error('Connection failed'))
    await opening
    expect(current).toMatchObject({ load: 'failed' })
    expect(current.lastError).toEqual({ message: 'Connection failed', requestId: null })
  } finally {
    history.resolve(Response.json(EMPTY_PAGE))
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
  expect(toasts.at(-1)).toMatchObject({ title: 'Could Not Archive “second”', message: 'Turn is running.' })
})

test('archiving says so in a toast whose Undo brings the conversation back', async () => {
  const store = useConversations()
  expect(await store.archive([FIRST])).toBe(true)
  expect(store.items.find((item) => item.id === FIRST)?.archived).toBe(true)
  const toast = toasts.findLast((item) => item.title === 'Conversation Archived')
  expect(toast?.action?.label).toBe('Undo')
  toast!.action!.run()
  await waitFor(() => store.items.find((item) => item.id === FIRST)?.archived === false, () => 'Undo did not restore it')
  expect(requests.filter((request) => request.path === '/api/conversations/batch').map((request) => request.body)).toEqual([
    { items: [{ id: FIRST, patch: { archived: true } }] },
    { items: [{ id: FIRST, patch: { archived: false } }] },
  ])
})

/**
 * Holds the backend's answers to the requests `matches` picks until each is
 * released, as a slow connection does: `arrived` resolves as the next one
 * reaches the backend, and `release` lets the oldest held one through.
 */
function holdAnswers(matches: (path: string, method: string) => boolean) {
  const answering = globalThis.fetch
  const held: (() => void)[] = []
  let arrival = deferred<void>()
  globalThis.fetch = (async (input, init) => {
    if (!matches(String(input), init?.method ?? 'GET')) {
      return answering(input, init)
    }
    const gate = deferred<void>()
    held.push(gate.resolve)
    arrival.resolve()
    arrival = deferred<void>()
    await gate.promise
    return answering(input, init)
  }) as typeof fetch
  return {
    /** Resolves once `count` requests are held. */
    arrived: async (count: number) => {
      while (held.length < count) {
        await arrival.promise
      }
    },
    release: () => held.shift()!(),
  }
}

const archivedOf = (id: string) => useConversations().items.find((item) => item.id === id)?.archived
const batchBodies = () => requests.filter((request) => request.path === '/api/conversations/batch').map((request) => request.body)

test('archive and pin show at once, and a pin while the archive is on its way is not refused', async () => {
  const store = useConversations()
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  const pinnedOf = () => store.items.find((item) => item.id === FIRST)?.pinned
  const archived = store.archive([FIRST])
  // The row leaves and the toast says so in the same turn, before any answer.
  expect(archivedOf(FIRST)).toBe(true)
  expect(toasts.at(-1)?.title).toBe('Conversation Archived')
  const pinned = store.pin([FIRST], true)
  expect(pinnedOf()).toBe(true)
  await backend.arrived(1)
  backend.release()
  expect(await archived).toBe(true)
  await backend.arrived(1)
  backend.release()
  expect(await pinned).toBe(true)
  expect(batchBodies()).toEqual([
    { items: [{ id: FIRST, patch: { archived: true } }] },
    { items: [{ id: FIRST, patch: { pinned: true } }] },
  ])
  expect(archivedOf(FIRST)).toBe(true)
  expect(pinnedOf()).toBe(true)
})

test('Undo before the archive is answered brings the row back at once and sends the restore after the archive', async () => {
  const store = useConversations()
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  const archived = store.archive([FIRST])
  const undo = toasts.findLast((toast) => toast.title === 'Conversation Archived')!.action!
  await backend.arrived(1)
  undo.run()
  expect(archivedOf(FIRST)).toBe(false)
  backend.release()
  await archived
  // The archive's answer lands under the restore, which still shows.
  expect(archivedOf(FIRST)).toBe(false)
  await backend.arrived(1)
  backend.release()
  await waitFor(() => batchBodies().length === 2, () => 'the restore was not sent')
  expect(batchBodies()).toEqual([
    { items: [{ id: FIRST, patch: { archived: true } }] },
    { items: [{ id: FIRST, patch: { archived: false } }] },
  ])
  await waitFor(() => records.find((item) => item.id === FIRST)?.archived === false)
  expect(archivedOf(FIRST)).toBe(false)
})

test('a refused archive brings the row back where it was and says why', async () => {
  const store = useConversations()
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  const order = () => store.items.filter((item) => !item.archived).map((item) => item.id)
  const before = order()
  // The stub's backend refuses to archive the second conversation, as one that runs.
  const archived = store.archive([SECOND])
  expect(archivedOf(SECOND)).toBe(true)
  const said = toasts.at(-1)!
  expect(said.title).toBe('Conversation Archived')
  await backend.arrived(1)
  backend.release()
  expect(await archived).toBe(false)
  expect(archivedOf(SECOND)).toBe(false)
  expect(order()).toEqual(before)
  // The toast that said it was archived goes; the refusal's says why.
  expect(toasts.some((toast) => toast.id === said.id)).toBe(false)
  expect(toasts.at(-1)).toMatchObject({ title: 'Could Not Archive “second”', message: 'Turn is running.' })
})

test('a summary the channel brings before the archive is answered does not bring the row back', async () => {
  const store = useConversations()
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  const archived = store.archive([FIRST])
  await backend.arrived(1)
  // The channel brings a summary read before the archive reached the backend.
  await changed(FIRST)
  expect(archivedOf(FIRST)).toBe(true)
  backend.release()
  await archived
  expect(archivedOf(FIRST)).toBe(true)
  // The channel brings the summary the archive committed, and then another
  // page's restore, which shows: nothing of this page's holds it.
  await changed(FIRST)
  expect(archivedOf(FIRST)).toBe(true)
  records.find((item) => item.id === FIRST)!.archived = false
  await changed(FIRST)
  expect(archivedOf(FIRST)).toBe(false)
})

const THIRD = '00000000-0000-4000-8000-000000000003'

test('archiving three of which the backend refuses one ends with the toast saying two, and its Undo restores those two', async () => {
  records.push(record(THIRD, 'third'))
  await changed(THIRD)
  const store = useConversations()
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  // The stub's backend refuses to archive the second conversation, as one that runs.
  const archived = store.archive([FIRST, SECOND, THIRD])
  const toast = toasts.at(-1)!
  expect(toast.title).toBe('3 Conversations Archived')
  await backend.arrived(1)
  backend.release()
  expect(await archived).toBe(false)
  // The same toast says it, in place; the refusal's own toast says why.
  expect(toasts.find((item) => item.id === toast.id)?.title).toBe('2 Conversations Archived')
  expect(archivedOf(SECOND)).toBe(false)
  toasts.find((item) => item.id === toast.id)!.action!.run()
  await backend.arrived(1)
  backend.release()
  await waitFor(() => batchBodies().length === 2, () => 'the restore was not sent')
  expect(batchBodies()[1]).toEqual({ items: [FIRST, THIRD].map((id) => ({ id, patch: { archived: false } })) })
})

// Cost: no timer; the held answer is released by the test.
test('a change of one conversation goes out beside another\'s archive, and one of the same conversation, or of a selection with it, after it', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { visibilityState: 'visible' } })
  try {
    const store = useConversations()
    const arrivals: string[] = []
    const archiveAnswer = deferred<void>()
    const answering = globalThis.fetch
    globalThis.fetch = (async (input, init) => {
      const path = String(input)
      if (path === '/api/conversations/batch') {
        const { items } = JSON.parse(String(init?.body)) as { items: { id: string; patch: object }[] }
        const label = `${JSON.stringify(items[0]!.patch)} ${items.map((item) => item.id === FIRST ? 'A' : 'B').join('+')}`
        arrivals.push(label)
        if (label === '{"archived":true} A') {
          await archiveAnswer.promise
        }
      } else if (path.endsWith('/read')) {
        arrivals.push('read B')
      }
      return answering(input, init)
    }) as typeof fetch
    Object.assign(records.find((item) => item.id === SECOND)!, { unread: true, revision: 3 })
    await changed(SECOND)
    store.items.find((item) => item.id === SECOND)!.load = 'ready'
    const archived = store.archive([FIRST])
    // Marking B read does not wait behind A's archive.
    const read = store.markRead(SECOND)
    await waitFor(() => arrivals.includes('read B'), () => 'the read waits behind the archive')
    // Undo of A's archive, and a pin of A and B, wait for it, in order.
    const undo = store.restore([FIRST])
    const pinned = store.pin([FIRST, SECOND], true)
    arrivals.push('archive answered')
    archiveAnswer.resolve()
    await Promise.all([archived, read, undo, pinned])
    expect(arrivals).toEqual([
      '{"archived":true} A',
      'read B',
      'archive answered',
      '{"archived":false} A',
      '{"pinned":true} A+B',
    ])
  } finally {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: original })
  }
})

/** The block the open conversation's transcript holds. */
const shownBlock = { type: 'text' as const, id: 'shown', createdAt: '2026-09-09T00:00:00.000Z', model, text: 'Shown' }

/**
 * Plays the page's conversation sockets from now on, which open at once:
 * with a `handshake`, each answers `open` as the backend does, with
 * `opened` and the handshake after it in one message; without, the test
 * answers it. The page's synchronization channel stays the test's. Answers
 * the sockets in the order the page opened them.
 */
function playPageSockets(handshake?: ServerFrame[]): TestSocket[] {
  const sockets: TestSocket[] = []
  const channel = globalThis.WebSocket
  function PageSocket(url: string | URL) {
    if (!new URL(url).pathname.endsWith('/stream')) {
      return new channel(url)
    }
    const socket = new TestSocket(String(url))
    const send = socket.send.bind(socket)
    socket.send = (data) => {
      send(data)
      if (handshake && socket.sent.at(-1)?.type === 'open') {
        queueMicrotask(() => socket.receive({ type: 'opened' }, ...handshake))
      }
    }
    sockets.push(socket)
    queueMicrotask(() => socket.open())
    return socket
  }
  Object.assign(PageSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 })
  Reflect.set(globalThis, 'WebSocket', PageSocket)
  return sockets
}

/** Waits until the page's socket `index` has sent `open`. */
async function openSent(sockets: TestSocket[], index = 0): Promise<void> {
  await waitFor(() => sockets[index]?.sent.some((frame) => frame.type === 'open') === true, () => `socket ${index} sent no open`)
}

/**
 * Starts opening the conversation `id` with a session: its transcript holds
 * one block, its agents are `subagents`, and its socket opens at once. With
 * a `handshake`, the socket answers `open` as the backend does, with
 * `opened` and the handshake after it in one message; without, the test
 * answers it. `models` holds the model list until it answers. Answers the
 * conversation, its sockets in the order the page opened them, and the
 * opening.
 */
async function startOpening(
  id: string,
  { handshake, subagents = [], models }: {
    handshake?: ServerFrame[]
    /** The subagent list's answer, or when it comes. */
    subagents?: unknown[] | Promise<unknown[]>
    models?: Promise<void>
  } = {},
) {
  records.find((item) => item.id === id)!.model = { providerId: 'stub', modelId: 'stub', thinkingEffort: null, serviceTierId: null }
  await changed(id)
  addStubProvider()
  const answering = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      await models
      return Response.json(stubCatalog())
    }
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: await subagents })
    if (path.endsWith('/transcript')) {
      requests.push({ path, body: null })
      return Response.json({ start: 0, length: 1, blocks: [shownBlock], instructions: [], summaries: [] })
    }
    return answering(input, init)
  }) as typeof fetch
  const sockets = playPageSockets(handshake)
  const listed = useProduct().loadModels(true)
  if (!models) {
    await listed
  }
  const store = useConversations()
  const opening = store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  return { conversation, sockets, opening }
}

/**
 * Opens the conversation `id` with a session, whose socket answers `open`
 * with the `handshake` (`startOpening`). Answers the conversation, its
 * sockets in the order the page opened them, and every `load` it had since
 * it opened.
 */
async function openWithSession(id: string, handshake: ServerFrame[] = []) {
  const { conversation, sockets, opening } = await startOpening(id, { handshake })
  await opening
  expect(conversation.load).toBe('ready')
  expect(sockets).toHaveLength(1)
  const loads: string[] = []
  watch(() => conversation.load, (load) => loads.push(load), { flush: 'sync' })
  return { conversation, sockets, loads }
}

/** A child of the conversation `FIRST`, as the backend lists it. */
function child(subagentId: string, number: number, phase: 'running' | 'completed') {
  return {
    subagentId, number, parentSessionId: FIRST, onPath: true, description: `Agent ${number}`,
    profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase, startedAt: '2026-09-13T00:00:00.000Z', endedAt: phase === 'running' ? null : '2026-09-13T00:01:00.000Z',
  }
}

/** The step of the opening the conversation shows: what names its agents, and what only its session knows. */
function shownStep(conversation: ReturnType<typeof useConversations>['items'][number]) {
  return {
    load: conversation.load,
    transcript: latestBlocks(conversation.history).map((block) => block.id),
    agents: conversation.subagents.map((agent) => `${agent.id} ${agent.phase}`),
    phase: conversation.phase,
    commands: conversation.terminals.map((terminal) => terminal.id),
    status: conversation.status,
  }
}

// Opening a conversation shows in two steps: production showed the dock's "1 Agent" over the loading pane, the REST
// list applied before the transcript showed, and a running conversation's
// sidebar dot went out while it opened, computed from a phase the page had
// not heard. Cost: one opening, no wait.
test('a conversation opens in two steps: its transcript with its agents, then its session', async () => {
  records.find((item) => item.id === FIRST)!.status = 'running'
  const subagents = [child('reader', 1, 'completed')]
  const { conversation, sockets, opening } = await startOpening(FIRST, { subagents })
  await waitFor(() => sockets[0]?.sent.some((frame) => frame.type === 'open') === true, () => 'no open sent')

  // The first step: the transcript with its agents, the status the summary's.
  // The session that opens sends a live child's transcript when shown.
  expect(useConversations().streamsSubagents(conversation)).toBe(true)
  expect(shownStep(conversation)).toEqual({
    load: 'opening', transcript: ['shown'], agents: ['reader completed'], phase: 'idle', commands: [], status: 'active',
  })

  sockets[0]!.receive(
    { type: 'opened' },
    { type: 'transcript_reset', start: 0, length: 1, blocks: [shownBlock], version: { epoch: 'epoch', revision: 1 } },
    { type: 'phase', phase: 'running' },
    { type: 'subagents', subagents: subagents.map((summary) => ({ ...summary, revision: 2 })) },
    {
      type: 'shell_output',
      status: { status: 'running', commandId: 'cmd-1', toolUseId: 'call-cmd-1', title: 'Serve cmd-1', script: 'serve cmd-1', tail: 'one\n', chars: 4, runningMs: 10 },
    },
  )
  await opening

  // The second step: the session's turn and its running command join.
  expect(shownStep(conversation)).toEqual({
    load: 'ready', transcript: ['shown'], agents: ['reader completed'], phase: 'running', commands: ['cmd-1'], status: 'active',
  })
})

// A large tree's subagent list took a minute to answer after a restart, and
// the page showed the conversation loading all that time, since its first
// step waited for the list and its `open` for the first step. The transcript
// and the session wait for nothing else; the session's own list names the
// children. Cost: one opening, no wait; the list's read never ends, which holds
// no timer.
test('a conversation whose subagent list does not answer shows its transcript and opens its session', async () => {
  const never = new Promise<unknown[]>(() => {})
  const { conversation, sockets, opening } = await startOpening(FIRST, { subagents: never })
  await waitFor(() => sockets[0]?.sent.some((frame) => frame.type === 'open') === true, () => 'no open sent')
  const first = shownStep(conversation)
  sockets[0]!.receive(
    { type: 'opened' },
    { type: 'transcript_reset', start: 0, length: 1, blocks: [shownBlock], version: { epoch: 'epoch', revision: 1 } },
    { type: 'phase', phase: 'idle' },
    { type: 'subagents', subagents: [{ ...child('reader', 1, 'running'), revision: 2 }] },
  )
  await opening

  expect(first).toMatchObject({ load: 'opening', transcript: ['shown'], agents: [] })
  expect(shownStep(conversation)).toMatchObject({ load: 'ready', transcript: ['shown'], agents: ['reader running'] })
})

// Marking read needs only the transcript, so it comes with the first step,
// not with a session that may take long to open. Cost: one opening, no wait.
test('a conversation is marked read at the first step of its opening, before its session opens', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { visibilityState: 'visible' } })
  try {
    Object.assign(records.find((item) => item.id === FIRST)!, { unread: true, revision: 4 })
    const { conversation, sockets } = await startOpening(FIRST)
    await waitFor(() => sockets[0]?.sent.some((frame) => frame.type === 'open') === true, () => 'no open sent')
    expect(conversation.load).toBe('opening')

    await useConversations().markRead(FIRST)

    expect(requests.find((request) => request.path.endsWith('/read'))?.body).toEqual({ revision: 4 })
    expect(conversation.unread).toBe(false)
  } finally {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: original })
  }
})

// Retry after a refused opening loads again from nothing: production kept
// the earlier attempt's agents, so "1 Agent" stood over the loading pane.
// Cost: two openings, no wait.
test('Retry after a refused opening shows nothing of the earlier attempt while it loads again', async () => {
  const store = useConversations()
  const { conversation, sockets, opening } = await startOpening(FIRST, { subagents: [child('reader', 1, 'running')] })
  await openSent(sockets)
  sockets[0]!.receive({ type: 'error', message: 'The provider is no longer available', code: 'provider_not_found', requestId: 'req_socket0000000001' })
  await opening
  expect(shownStep(conversation)).toMatchObject({ load: 'failed', transcript: ['shown'], agents: ['reader running'] })

  const held = holdOpening()
  void store.reloadSession(FIRST)
  await waitFor(() => held.arrived.includes('transcript'), () => 'no transcript read')

  expect(shownStep(conversation)).toEqual({
    load: 'loading', transcript: [], agents: [], phase: 'idle', commands: [], status: conversation.status,
  })
  expect(conversation.lastError).toBeNull()
})

// Every first session is an opening: a new conversation's first send whose
// socket cannot be made says it connects, never that it reconnects to a session
// it never had. Cost: one socket, no wait.
test('a new conversation whose first session cannot connect is opening, not reconnecting', async () => {
  const store = useConversations()
  const id = store.create()
  await store.activate(id)
  const conversation = store.items.find((item) => item.id === id)!
  addStubProvider()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path === `/api/conversations/${id}` && init?.method === 'PATCH') {
      return Response.json({ conversation: records.find((item) => item.id === id), results: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  const sockets = playPageSockets()
  const loads: string[] = []
  const stop = watch(() => conversation.load, (load) => loads.push(load), { flush: 'sync' })
  try {
    await useProduct().loadModels(true)
    await store.changeModel(conversation, { model: { providerId: 'stub', modelId: 'stub' } })
    conversation.draft = 'First message'
    void store.send(conversation)
    await openSent(sockets)
    sockets[0]!.end()
    await waitFor(() => conversation.load === 'connecting', () => `it is ${conversation.load}`)
    expect(loads).toEqual(['opening', 'connecting'])
  } finally {
    stop()
    submit.mockRestore()
  }
})

// A conversation opened while archived starts its first session once it is
// restored, which is an opening too. Cost: one opening, one socket, no wait.
test('a conversation restored from the archive whose first session cannot connect is opening, not reconnecting', async () => {
  const store = useConversations()
  records.find((item) => item.id === FIRST)!.archived = true
  const { conversation, sockets, opening } = await startOpening(FIRST)
  await opening
  // Archived, it opens no session.
  expect(conversation.load).toBe('ready')
  expect(sockets).toEqual([])
  const loads: string[] = []
  const stop = watch(() => conversation.load, (load) => loads.push(load), { flush: 'sync' })
  try {
    void store.restore([FIRST])
    await openSent(sockets)
    sockets[0]!.end()
    await waitFor(() => conversation.load === 'connecting', () => `it is ${conversation.load}`)
    expect(loads).toEqual(['opening', 'connecting'])
  } finally {
    stop()
  }
})

// A first opening whose socket cannot be made is still an opening: it shows
// the transcript with its agents and says it connects, never a session it
// never had. Cost: two sockets, no wait.
test('a first opening that cannot connect shows its transcript, connects, and only then its session', async () => {
  records.find((item) => item.id === FIRST)!.status = 'running'
  const { conversation, sockets, opening } = await startOpening(FIRST, { subagents: [child('reader', 1, 'running')] })
  await waitFor(() => sockets[0]?.sent.some((frame) => frame.type === 'open') === true, () => 'no open sent')
  sockets[0]!.end()
  await waitFor(() => conversation.load !== 'opening', () => 'the lost socket left the opening as it was')

  expect(shownStep(conversation)).toEqual({
    load: 'connecting', transcript: ['shown'], agents: ['reader running'], phase: 'idle', commands: [], status: 'active',
  })

  await pageReturned()
  await waitFor(() => sockets[1]?.sent.some((frame) => frame.type === 'open') === true, () => 'no second open')
  sockets[1]!.receive({ type: 'opened' }, { type: 'phase', phase: 'running' })
  await opening
  expect(shownStep(conversation)).toMatchObject({ load: 'ready', phase: 'running', status: 'active' })
})

// History does not wait for the model list; the session does, and the
// conversation shows its first step until then. Cost: one opening, no wait.
test('a conversation whose model list has not answered shows its first step until its session opens', async () => {
  const models = deferred<void>()
  const { conversation, sockets, opening } = await startOpening(FIRST, {
    handshake: [{ type: 'phase', phase: 'idle' }],
    subagents: [child('reader', 1, 'completed')],
    models: models.promise,
  })
  await waitFor(() => conversation.load !== 'loading', () => 'the transcript waited for the model list')

  expect(shownStep(conversation)).toMatchObject({ load: 'opening', transcript: ['shown'], agents: ['reader completed'] })
  expect(sockets.flatMap((socket) => socket.sent)).toEqual([])

  models.resolve()
  await opening
  expect(conversation.load).toBe('ready')
})

// A conversation that opens no session stops at the first step, its status
// the summary's: here a turn the process died under, which its transcript
// alone would not tell. Cost: one opening.
test('a conversation that opens no session shows its transcript with its agents and keeps the summary’s status', async () => {
  records.find((item) => item.id === FIRST)!.status = 'interrupted'
  await changed(FIRST)
  const store = useConversations()
  const restoreFetch = answerOpening(FIRST)
  const served = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.endsWith('/subagents')) return Response.json({ subagents: [child('reader', 1, 'completed')] })
    if (path.endsWith('/transcript')) return Response.json({ start: 0, length: 1, blocks: [shownBlock], instructions: [], summaries: [] })
    return served(input, init)
  }) as typeof fetch
  try {
    await store.activate(FIRST)
    const conversation = store.items.find((item) => item.id === FIRST)!
    expect(shownStep(conversation)).toEqual({
      load: 'ready', transcript: ['shown'], agents: ['reader completed'], phase: 'idle', commands: [], status: 'error',
    })
  } finally {
    restoreFetch()
  }
})

// A conversation the backend made without a model, such as an automation's
// run's: production showed it as "Couldn't load this conversation" when the
// backend refused its session. It opens to its transcript and waits for a
// model, which the composer's choice writes. Cost: one opening.
test('a conversation whose record holds no model opens to its transcript, not as failed', async () => {
  records.find((item) => item.id === FIRST)!.model = null
  await changed(FIRST)
  addStubProvider()
  const answering = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json({ start: 0, length: 1, blocks: [shownBlock], instructions: [], summaries: [] })
    return answering(input, init)
  }) as typeof fetch
  try {
    const sockets = playPageSockets()
    await useProduct().loadModels(true)
    const store = useConversations()
    const opening = store.activate(FIRST)
    // A socket the page opened anyway is refused, as the backend refuses it.
    void openSent(sockets).then(() => sockets[0]!.receive({
      type: 'error', message: 'Choose a model for the conversation before opening it', code: 'model_not_selected', requestId: 'req_socket0000000001',
    }), () => {})
    await opening.catch(() => {})
    const conversation = store.items.find((item) => item.id === FIRST)!
    expect(conversation.load).toBe('ready')
    expect(conversation.lastError).toBeNull()
    expect(shownStep(conversation).transcript).toEqual(['shown'])
    // No session sends a child's transcript: the Agents window reads it.
    expect(store.streamsSubagents(conversation)).toBe(false)
  } finally {
    globalThis.fetch = answering
  }
})

const transcriptReads = () => requests.filter((request) => request.path.endsWith('/transcript')).length

// What a reopened socket brings back: production
// showed a subagent Running with its transcript stopped where the page's
// socket dropped, though it had finished while the page was away, until a
// reload. Each open's snapshot lists every child, every running command
// and every node's usage, and the page takes them whole: the child ended,
// one that started and ended meanwhile appears, a command ended or is
// gone, and a usage the open cannot tell clears the figure. Cost: two
// openings, no wait.
test('a conversation that opens again after its socket dropped shows what ended while it was away', async () => {
  const running = {
    subagentId: 'reader', number: 1, parentSessionId: FIRST, onPath: true, description: 'Read the notes',
    profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work',
    phase: 'running' as const, startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
  }
  const brief = { ...shownBlock, id: 'brief' }
  const calls = (status: 'executing' | 'completed') =>
    ['cmd-1', 'cmd-2'].map((id) => execCall(`call-${id}`, `serve ${id}`, status))
  const usage = { tokens: 10, window: 100, compactFrom: 50, rows: [], account: null }
  const command = (commandId: string) => ({
    type: 'shell_output' as const,
    subagentId: 'reader',
    status: { status: 'running' as const, commandId, toolUseId: `call-${commandId}`, title: `Serve ${commandId}`, script: `serve ${commandId}`, tail: 'one\n', chars: 4, runningMs: 10 },
  })
  const handshake: ServerFrame[] = [
    { type: 'subagents', subagents: [{ ...running, revision: 3 }] },
    { type: 'subagent', event: 'started', job: running },
    { type: 'subagent_transcript_reset', subagentId: 'reader', start: 0, length: 3, blocks: [brief, ...calls('executing')], revision: 3 },
    { type: 'phase', subagentId: 'reader', phase: 'running' },
    { type: 'waiting', subagentId: 'reader', waitedMs: 1_000 },
    command('cmd-1'),
    command('cmd-2'),
    { type: 'context_usage', node: FIRST, usage },
  ]
  const { conversation, sockets } = await openWithSession(FIRST, handshake)
  const reader = () => conversation.subagents.find((agent) => agent.id === 'reader')!
  expect(reader()).toMatchObject({ phase: 'running', sessionPhase: 'running' })
  expect(conversation.terminals.map((terminal) => terminal.id)).toEqual(['cmd-1', 'cmd-2'])
  expect(conversation.contextUsage).toEqual(usage)

  // While the page is away the reader finishes, agent 2 starts and
  // finishes, and the reader's commands end, the second one forgotten
  // with its shells.
  handshake.splice(
    0,
    handshake.length,
    {
      type: 'subagents',
      subagents: [
        { ...running, phase: 'completed', endedAt: '2026-09-13T00:02:00.000Z', revision: 9 },
        { ...running, subagentId: 'counter', number: 2, description: 'Count', phase: 'completed', endedAt: '2026-09-13T00:03:00.000Z', revision: 4 },
      ],
    },
    {
      type: 'shell_output', subagentId: 'reader',
      status: { status: 'exited', commandId: 'cmd-1', toolUseId: 'call-cmd-1', title: 'Serve cmd-1', script: 'serve cmd-1', tail: 'one\ntwo\n', chars: 8, runningMs: 20, exitCode: 0 },
    },
    { type: 'context_usage', node: FIRST, usage: null },
  )
  sockets[0]!.end()
  await pageReturned()
  await waitFor(
    () => sockets[1]?.sent.length === 1 && conversation.load === 'ready',
    () => `it did not open again: ${sockets.length} sockets`,
  )

  expect(reader()).toMatchObject({
    phase: 'completed', endedAt: '2026-09-13T00:02:00.000Z', sessionPhase: 'idle', waitingSince: null,
  })
  // Its transcript moved past what the page holds: it is read again when shown.
  expect(reader().history.windows).toEqual([])
  expect(conversation.subagents.map((agent) => [agent.id, agent.phase])).toEqual([['reader', 'completed'], ['counter', 'completed']])
  expect(toRaw(conversation.terminals).map((terminal) => [terminal.id, terminal.phase])).toEqual([['cmd-1', 'exited']])
  expect(conversation.contextUsage).toBeNull()
  expect(conversation.status).not.toBe('active')
})

// Production: a phone started a Resume, the turn ran on, the phone slept
// through it, and on return the dead socket's silence rejected the Resume's
// wait with a toast "The conversation did not accept the request.", though
// the session had taken it. A connection lost while the session ran an
// action is the Connecting state, then the reopened snapshot, never a refusal.
// Cost: two openings, no wait.
test('a page that returns from sleep after a Resume started shows no refusal and opens the conversation again', async () => {
  const { conversation, sockets } = await openWithSession(FIRST)
  const store = useConversations()
  store.start(conversation)
  await waitFor(() => sockets[0]!.sent.some((frame) => frame.type === 'resume'), () => 'no resume sent')
  // The session takes the action: it leaves idle.
  sockets[0]!.receive({ type: 'phase', phase: 'running' })
  const before = toasts.length
  // The phone sleeps past the 75 seconds the socket may stay silent; on
  // return the page finds the socket broken and replaces it.
  setSystemTime(new Date(Date.now() + 80_000))
  try {
    await pageReturned()
    await waitFor(
      () => sockets[1]?.sent.length === 1 && conversation.load === 'ready',
      () => `it did not open again: ${sockets.length} sockets`,
    )
  } finally {
    setSystemTime()
  }
  expect(sockets[0]!.closed).toBe(true)
  expect(toasts.slice(before).map((toast) => toast.title)).toEqual([])
})

// A request the drop interrupts: a Stop the session
// had not answered when the socket dropped is neither refused nor sent
// again; the toast says the connection was lost. Cost: one opening.
test('a Stop whose socket drops before the session answers it shows Connection Lost and is not sent again', async () => {
  const { conversation, sockets } = await openWithSession(FIRST)
  const store = useConversations()
  sockets[0]!.receive({ type: 'phase', phase: 'running' })
  const before = toasts.length
  const stopping = store.stop(conversation)
  await waitFor(() => sockets[0]!.sent.some((frame) => frame.type === 'abort'), () => 'no abort sent')
  sockets[0]!.end()
  expect(await stopping).toBe(false)
  expect(toasts.slice(before)).toMatchObject([{
    title: 'Connection Lost',
    message: 'Demi may not have received your request. Check the conversation once it reconnects.',
  }])
  reconnectNow()
  await waitFor(() => sockets[1]?.sent.some((frame) => frame.type === 'open') === true, () => 'it did not reopen')
  expect(sockets.flatMap((socket) => socket.sent).filter((frame) => frame.type === 'abort')).toHaveLength(1)
})

// The same section: a request made as the page returns waits for the
// return's check, so it never goes on the socket the check is about to
// replace. Cost: two openings, no wait.
test('a Stop pressed as the page returns goes on the new socket, never on the one the return finds dead', async () => {
  const { conversation, sockets } = await openWithSession(FIRST)
  const store = useConversations()
  sockets[0]!.receive({ type: 'phase', phase: 'running' })
  setSystemTime(new Date(Date.now() + 80_000))
  try {
    const returned = pageReturned()
    const stopping = store.stop(conversation)
    await returned
    await waitFor(() => sockets[1]?.sent.some((frame) => frame.type === 'abort') === true, () => 'no abort on the new socket')
    expect(sockets[1]!.sent.map((frame) => frame.type)).toEqual(['open', 'abort'])
    sockets[1]!.receive({ type: 'abort_result', result: { target: null, canAbortAgain: false } })
    expect(await stopping).toBe(true)
  } finally {
    setSystemTime()
  }
  expect(sockets[0]!.sent.filter((frame) => frame.type === 'abort')).toEqual([])
})

// A reload while the agent waits for the model, with a steer waiting: the
// page shows the conversation once, as the session has it, the transcript
// with the steer and the Requesting clock counting on from the wait the
// session tells, never a moment without the steer, with nothing running or
// with a clock at 0s. Before, the transcript showed as soon as it was read,
// a round trip before the handshake. Cost: one opening.
test('a conversation opened during a turn shows its transcript with its steer and its wait together', async () => {
  const store = useConversations()
  const conversation = () => store.items.find((item) => item.id === FIRST)!
  const steer = { id: 'steer-1', turnId: 'turn-1', model, content: [{ type: 'text' as const, text: 'Also look at the tests.' }] }
  const shown: string[] = []
  const stop = watch(
    () => {
      const current = conversation()
      const wait = current.waitingSince === null ? 'no wait' : `waited ${Math.floor((Date.now() - current.waitingSince) / 1000)}s`
      const steers = current.pendingSteers.map((pending) => pending.id).join() || 'no steer'
      return current.load === 'ready' ? `${current.phase}, ${steers}, ${wait}` : current.load
    },
    (state) => shown.push(state),
    { flush: 'sync' },
  )
  await openWithSession(FIRST, [
    { type: 'transcript_reset', start: 0, length: 1, blocks: [shownBlock], version: { epoch: 'epoch', revision: 1 } },
    { type: 'phase', phase: 'running' },
    { type: 'queue', queue: [] },
    { type: 'pending_steers', pendingSteers: [steer] },
    { type: 'pending_calls', pendingCalls: [] },
    { type: 'waiting', waitedMs: 11_000 },
  ])
  stop()
  // The transcript shows first, then the session's turn with its steer and its wait in one change.
  expect(shown.filter((state) => state !== 'loading')).toEqual(['opening', 'running, steer-1, waited 11s'])
  expect(shownRows(conversation()).waiting).toEqual(['message:steer-1'])
})

test('archiving the open conversation keeps its transcript, and Undo connects it again from the blocks it holds, never loading', async () => {
  const store = useConversations()
  const { conversation, sockets, loads } = await openWithSession(FIRST)
  const backend = holdAnswers((path) => path === '/api/conversations/batch')
  const archived = store.archive([FIRST])
  // In the same turn it is read-only, with its blocks, and its socket goes.
  expect(conversation.archived).toBe(true)
  expect(latestBlocks(conversation.history).map((block) => block.id)).toEqual(['shown'])
  expect(sockets[0]!.closed).toBe(true)
  await backend.arrived(1)
  backend.release()
  expect(await archived).toBe(true)
  toasts.findLast((toast) => toast.title === 'Conversation Archived')!.action!.run()
  expect(conversation.archived).toBe(false)
  await backend.arrived(1)
  // The backend refuses an archived conversation's socket: it connects once the restore lands.
  expect(sockets).toHaveLength(1)
  backend.release()
  await waitFor(() => sockets[1]?.sent.length === 1, () => `it did not connect again: ${sockets.length} sockets`)
  // Its `open` names the blocks it holds, as the first did.
  expect(sockets[1]!.sent[0]).toEqual(sockets[0]!.sent[0]!)
  expect(latestBlocks(conversation.history).map((block) => block.id)).toEqual(['shown'])
  expect(transcriptReads()).toBe(1)
  expect(loads).toEqual([])
})

test('an archive the backend refuses keeps the open conversation\'s transcript and connects it again, never loading', async () => {
  const store = useConversations()
  // The stub's backend refuses to archive the second conversation, as one that runs.
  const { conversation, sockets, loads } = await openWithSession(SECOND)
  expect(await store.archive([SECOND])).toBe(false)
  expect(conversation.archived).toBe(false)
  await waitFor(() => sockets[1]?.sent.length === 1, () => `it did not connect again: ${sockets.length} sockets`)
  expect(sockets[1]!.sent[0]).toEqual(sockets[0]!.sent[0]!)
  expect(latestBlocks(conversation.history).map((block) => block.id)).toEqual(['shown'])
  expect(transcriptReads()).toBe(1)
  expect(loads).toEqual([])
})

// Production: research subagents' commands showed in the Running window as
// `70460`, `70477`…: the first frame of a subagent's command came while its
// call still ran, the page held no transcript of the child, and the read of
// the call by the command found none yet and was never tried again. A
// command's tab takes its title and script from its frames, which carry
// them from the command's record: a page that holds no call of it titles it
// before the call returns and after it opens again, and reads nothing for
// it. Cost: two openings, no wait.
test('a subagent\'s running command is titled from its frames, whether or not the page holds its call', async () => {
  const view = {
    commandId: '70460', toolUseId: 'call-clone', title: 'Clone the repository',
    script: 'git clone https://example.test/repo.git', runningMs: 10,
  }
  const frame = (tail: string, chars: number) => ({
    type: 'shell_output' as const, subagentId: 'researcher',
    status: { status: 'running' as const, ...view, tail, chars },
  })
  const handshake: ServerFrame[] = [
    { type: 'subagents', subagents: [{ ...child('researcher', 1, 'running'), revision: 2 }] },
    frame('Cloning\n', 8),
  ]
  const reads: string[] = []
  const answering = globalThis.fetch
  globalThis.fetch = ((input, init) => {
    const path = String(input)
    if (path.includes('/commands/') || path.includes('/transcript/blocks/'))
      reads.push(path)
    return answering(input, init)
  }) as typeof fetch
  const { conversation, sockets } = await openWithSession(FIRST, handshake)
  const tabs = () => toRaw(conversation.terminals).map(({ id, title, script, subagentId }) => ({ id, title, script, subagentId }))
  const titled = { id: '70460', title: 'Clone the repository', script: 'git clone https://example.test/repo.git', subagentId: 'researcher' }
  expect(conversation.subagents.find((agent) => agent.id === 'researcher')?.history.windows).toEqual([])
  expect(tabs()).toEqual([titled])

  // The page opens again, as after a reload, and the command's next frame
  // names it as the first did.
  handshake.splice(1, 1, frame('Cloning\nReceiving objects\n', 26))
  sockets[0]!.end()
  await pageReturned()
  await waitFor(
    () => sockets[1]?.sent.length === 1 && conversation.load === 'ready',
    () => `it did not open again: ${sockets.length} sockets`,
  )
  expect(tabs()).toEqual([titled])
  expect(reads).toEqual([])
})

test('deleted rows leave at once, and a deletion that fails brings its row back and says why', async () => {
  const store = useConversations()
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (init?.method === 'DELETE') {
      return String(input) === `/api/conversations/${SECOND}`
        ? Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Storage unavailable' }, { status: 500 })
        : new Response(null, { status: 204 })
    }
    return fetch(input, init)
  }) as typeof fetch
  const backend = holdAnswers((_, method) => method === 'DELETE')
  const listed = () => store.items.map((item) => item.id)
  const removed = store.remove([FIRST, SECOND])
  expect(listed()).not.toContain(FIRST)
  expect(listed()).not.toContain(SECOND)
  await backend.arrived(1)
  backend.release()
  await backend.arrived(1)
  backend.release()
  expect(await removed).toBe(false)
  expect(listed()).not.toContain(FIRST)
  expect(listed()).toContain(SECOND)
  expect(toasts.at(-1)).toMatchObject({ title: 'Could Not Delete “second”', message: 'Something went wrong on the server.' })
})

// An outcome already reached is no failure: a conversation another page deleted
// meanwhile is deleted, with no toast. Before, it toasted "Could Not Delete".
test('deleting a conversation another page deleted already is done, with no toast', async () => {
  const store = useConversations()
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (init?.method === 'DELETE' && String(input) === `/api/conversations/${SECOND}`) {
      return Response.json({ requestId: 'req_test000000000001', code: 'conversation_not_found', message: 'No such conversation' }, { status: 404 })
    }
    return fetch(input, init)
  }) as typeof fetch
  const shown = toasts.length
  expect(await store.remove([SECOND])).toBe(true)
  expect(store.items.map((item) => item.id)).not.toContain(SECOND)
  // This test's browser keeps no drafts, which a toast of its own says.
  expect(toasts.slice(shown).filter((toast) => toast.title !== 'Could Not Save Drafts')).toEqual([])
})

// A move naming a conversation another page deleted meanwhile lost a race
// and says nothing; an order the backend refuses as out of partition is a
// defect of the page's and still toasts.
test.each([
  { answer: 'conversation_not_found', status: 404, toast: false },
  { answer: 'invalid_order', status: 409, toast: true },
] as const)('a reorder the backend answers $answer toasts: $toast', async ({ answer, status, toast }) => {
  const store = useConversations()
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (String(input) === '/api/sidebar/reorder') {
      return Response.json({ requestId: 'req_test000000000001', code: answer, message: 'Refused' }, { status })
    }
    return fetch(input, init)
  }) as typeof fetch
  const shown = toasts.length
  await store.reorder(FIRST, null)
  const titles = toasts.slice(shown).map((each) => each.title).filter((title) => title !== 'Could Not Save Drafts')
  expect(titles).toEqual(toast ? ['Could Not Reorder Conversations'] : [])
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

test('a request that waits is read with the conversation: the mark settles while the request waits, and a later request lights it again', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { visibilityState: 'visible' } })
  try {
    const store = useConversations()
    const item = () => store.items.find((candidate) => candidate.id === SECOND)!
    const server = records.find((candidate) => candidate.id === SECOND)!
    item().load = 'ready'

    // The request is raised: the row shows the needs-you mark until it is read.
    Object.assign(server, { permissionRequests: 1, unreadPermissionRequests: 1 })
    await changed(SECOND)
    expect(item().needsYou).toBe(true)

    // Reading acknowledges it at once, though nothing else of the conversation is unread.
    await store.markRead(SECOND)
    expect(requests.at(-1)?.path).toEndWith(`/conversations/${SECOND}/read`)
    expect(item().needsYou).toBe(false)

    // The backend's summary settles too, and the request still waits; the mark stays settled.
    Object.assign(server, { unreadPermissionRequests: 0 })
    await changed(SECOND)
    expect(item().needsYou).toBe(false)

    // A request raised after the reading lights the mark again; nothing to read when none waits.
    Object.assign(server, { permissionRequests: 2, unreadPermissionRequests: 1 })
    await changed(SECOND)
    expect(item().needsYou).toBe(true)
    const sent = requests.length
    Object.assign(server, { permissionRequests: 0, unreadPermissionRequests: 0 })
    await changed(SECOND)
    await store.markRead(SECOND)
    expect(requests.length).toBe(sent)
  } finally {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: original })
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

/** Serves each opening's transcript, held by `gate`, and the attached hosts `hosts` names when asked. */
function serveHistory(gate?: ReturnType<typeof deferred<void>>, hosts: () => AttachedHost[] = () => []) {
  const requested = deferred<void>()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/hosts') || path.endsWith('/transcript')) {
      requests.push({ path, body: null })
      if (path.endsWith('/hosts')) {
        return Response.json({ hosts: hosts() })
      }
      requested.resolve()
      await gate?.promise
      return Response.json(EMPTY_PAGE)
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
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    const read = /\/(transcript|hosts|draft)$/.exec(path)?.[1]
    if (read && (init?.method ?? 'GET') === 'GET') {
      held.arrived.push(read)
      if (read === 'draft') {
        await held.draft.promise
        return originalFetch(input, init)
      }
      await held[read as 'transcript' | 'hosts'].promise
      return read === 'hosts' ? Response.json({ hosts: [] }) : Response.json(EMPTY_PAGE)
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

// A conversation opened while Demi restarts shows it loading under the
// connection banner, never a failure about the connection, and loads once
// the page reaches the backend again. Cost: one opening, no timer.
test('a conversation opened while the backend is away, kept for opening offline nowhere, loads once it is back, without failing', async () => {
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  let away = true
  let refused = 0
  const restoreFetch = answerOpening(FIRST)
  const served = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (away && String(input).endsWith('/transcript')) {
      refused += 1
      // What the development server's proxy answers while no backend runs behind it.
      return new Response('Bad Gateway', { status: 502 })
    }
    return served(input, init)
  }) as typeof fetch
  try {
    channels.last().end(1001, 'backend_closing')
    expect(useProduct().connection).toBe('restarting')
    const opened = store.activate(FIRST)
    await waitFor(() => refused === 1, () => 'no transcript request')
    // This web browser keeps no copy of it: it says so meanwhile, and fails
    // nothing.
    await waitFor(() => first.load === 'offline', () => `the conversation is ${first.load}`)
    expect(first.lastError).toBeNull()
    away = false
    await pageReturned()
    channels.last().connect(backendState())
    await opened
    expect(first.load).toBe('ready')
    expect(first.lastError).toBeNull()
  } finally {
    restoreFetch()
  }
})

// Only the backend's own answer shows the failure pane, at once.
test('a conversation the backend does not have fails to open at once', async () => {
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  const restoreFetch = answerOpening(FIRST)
  const served = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    if (String(input).endsWith('/transcript')) {
      return Response.json({ requestId: 'req_test000000000001', code: 'conversation_not_found', message: 'Conversation not found' }, { status: 404 })
    }
    return served(input, init)
  }) as typeof fetch
  try {
    await store.activate(FIRST)
    expect(first.load).toBe('failed')
    expect({ ...first.lastError }).toEqual({ message: 'The conversation wasn’t found.', requestId: 'req_test000000000001' })
  } finally {
    restoreFetch()
  }
})

// The backend's message names its files and an operator's command: the
// page says it in its own words and keeps the message for the console.
test('a conversation the server cannot read fails to open in the page’s words', async () => {
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  const restoreFetch = answerOpening(FIRST)
  const served = globalThis.fetch
  const message = '/srv/demi/conversations/c.sqlite was made by a newer release or another build of Demi: return to the release that made it, or restore the databases of this release with `demi-server rollback`'
  globalThis.fetch = (async (input, init) => {
    if (String(input).endsWith('/transcript')) {
      return Response.json({ requestId: 'req_test000000000001', code: 'conversation_unreadable', message }, { status: 500 })
    }
    return served(input, init)
  }) as typeof fetch
  const logged = spyOn(console, 'error').mockImplementation(() => {})
  try {
    await store.activate(FIRST)
    expect(first.load).toBe('failed')
    // The page's words, and the request whose answer it read.
    expect(first.lastError).toEqual({ message: codeText('conversation_unreadable')!, requestId: 'req_test000000000001' })
    expect(logged.mock.calls.some((call) => call.some((value) => value instanceof Error && value.message === message))).toBe(true)
  } finally {
    logged.mockRestore()
    restoreFetch()
  }
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

test('the list waits for the new conversations this web browser keeps, so a reload finds the one its address names', async () => {
  useConversations().stopAll()
  useProduct().stop()
  const signOut = signIn()
  // A new conversation, typed in and never sent: only this web browser's storage has it.
  const draftId = '00000000-0000-4000-8000-000000000003'
  const listed = deferred<Awaited<ReturnType<typeof draftStorage.readLocalDrafts>>>()
  const read = spyOn(draftStorage, 'readLocalDrafts').mockReturnValue(listed.promise)
  try {
    const store = useConversations()
    const initialized = store.initialize()
    // The channel's first state arrives before the storage answers.
    await connect()
    expect(store.listStatus).toBe('loading')
    const now = '2026-09-09T00:00:00.000Z'
    listed.resolve({ drafts: [{
      messageEdit: null, pendingSends: [], base: null, model: null, files: [], scroll: null, notSaved: [], comments: [],
      text: 'Fix the login bug',
      local: {
        phase: 'draft',
        conversation: { id: draftId, title: 'New conversation', pinned: false, archived: false, target: { kind: 'cloud' }, createdAt: now, updatedAt: now },
      },
    }], unreadable: [] })
    await initialized
    expect(store.listStatus).toBe('ready')
    expect(store.items.find((item) => item.id === draftId)?.draft).toBe('Fix the login bug')
  } finally {
    read.mockRestore()
    signOut()
  }
})

test('unsent drafts lead the list, newest first, live and after a reload whether their storage or the channel answers first', async () => {
  const third = '00000000-0000-4000-8000-000000000003'
  let store = useConversations()
  const older = store.create()
  store.items.find((item) => item.id === older)!.draft = 'Older draft'
  const newer = store.create()
  store.items.find((item) => item.id === newer)!.draft = 'Newer draft'
  // Another page creates a conversation, which takes the front of the saved order.
  records.unshift(record(third))
  await changed(third)
  channels.last().send({ type: 'conversation_order', ids: records.map((item) => item.id) })
  await nextTick()
  const expected = [newer, older, third, FIRST, SECOND]
  expect(store.items.map((item) => item.id)).toEqual(expected)

  // The drafts as this web browser's storage gives them back, in no particular order.
  const kept = [older, newer].map((id): draftStorage.SavedDraft => ({
    messageEdit: null, pendingSends: [], base: null, model: null, files: [], scroll: null, notSaved: [], comments: [],
    text: id === older ? 'Older draft' : 'Newer draft',
    local: {
      phase: 'draft',
      conversation: {
        id, title: 'New conversation', pinned: false, archived: false, target: { kind: 'cloud' },
        createdAt: id === older ? '2026-10-01T00:00:00.000Z' : '2026-10-02T00:00:00.000Z',
        updatedAt: '2026-10-02T00:00:00.000Z',
      },
    },
  }))
  for (const storageFirst of [true, false]) {
    store.stopAll()
    useProduct().stop()
    disposePinia(pinia)
    pinia = createPinia()
    setActivePinia(pinia)
    const signOut = signIn()
    const listed = deferred<Awaited<ReturnType<typeof draftStorage.readLocalDrafts>>>()
    const read = spyOn(draftStorage, 'readLocalDrafts').mockReturnValue(listed.promise)
    try {
      store = useConversations()
      const initialized = store.initialize()
      if (storageFirst) {
        listed.resolve({ drafts: kept, unreadable: [] })
        await initialized
        channels.last().connect(backendState())
        await nextTick()
      } else {
        await connect()
        listed.resolve({ drafts: kept, unreadable: [] })
        await initialized
      }
      expect(store.items.map((item) => item.id)).toEqual(expected)
    } finally {
      read.mockRestore()
      signOut()
    }
  }
})

test('a draft moves only among the drafts, which keep the front', async () => {
  const signOut = signIn()
  try {
    const store = useConversations()
    const older = store.create()
    store.items.find((item) => item.id === older)!.draft = 'Older draft'
    const newer = store.create()
    store.items.find((item) => item.id === newer)!.draft = 'Newer draft'
    await store.reorder(newer, SECOND)
    expect(store.items.map((item) => item.id)).toEqual([older, newer, FIRST, SECOND])
    await store.reorder(newer, older)
    expect(store.items.map((item) => item.id)).toEqual([newer, older, FIRST, SECOND])
    expect(requests.some((request) => request.path === '/api/sidebar/reorder')).toBe(false)
  } finally {
    signOut()
  }
})

test('the first load at a new conversation\'s address that its history entry marks reads nothing of it', async () => {
  useConversations().stopAll()
  useProduct().stop()
  // An empty new conversation: not even this web browser's storage has it.
  const draftId = '00000000-0000-4000-8000-000000000004'
  const store = useConversations()
  await store.activate(draftId, { newConversation: true })
  await connect()
  await settle()
  expect(requests.filter((request) => request.path.includes(draftId))).toEqual([])
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

test('output that advances an open conversation\'s revision reads nothing; a raised hosts revision reads and shows its hosts without opening it again', async () => {
  let hosts: AttachedHost[] = []
  serveHistory(undefined, () => hosts)
  const store = useConversations()
  await store.activate(FIRST)
  const reads = (route: string) => requests.filter((item) => item.path === `/api/conversations/${FIRST}/${route}`).length
  expect(reads('hosts')).toBe(1)
  // A background job's turn: each saved step advances the revision.
  for (const revision of [1, 2, 3]) {
    records[0]!.revision = revision
    await changed(FIRST)
  }
  expect(reads('hosts')).toBe(1)
  // A shell on the attached host ended in another directory.
  hosts = [{ deviceId: 'ci', name: 'ci', cwd: '/work/sub', state: 'online', attachedAt: '2026-09-09T00:00:00.000Z' }]
  records[0]!.hostsRevision = 1
  await changed(FIRST)
  const first = store.items.find((item) => item.id === FIRST)!
  await waitFor(() => first.attachedHosts.shown.value?.[0]?.cwd === '/work/sub', () => 'the hosts were not read again')
  expect([reads('hosts'), reads('transcript'), first.load]).toEqual([2, 1, 'ready'])
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

test('a raised hosts revision reads the hosts of a conversation opened before and nothing of one never opened; retry explicitly reloads', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate(FIRST)
  await store.activate(SECOND)
  const hostReads = (id: string) => requests.filter((item) => item.path === `/api/conversations/${id}/hosts`).length
  const transcripts = () => requests.filter((item) => item.path.endsWith('/transcript')).length
  records[0]!.hostsRevision = 1
  await changed(FIRST)
  await waitFor(() => hostReads(FIRST) === 2, () => 'the hosts were not read again')
  await store.activate(FIRST)
  expect(transcripts()).toBe(2)
  await store.reloadSession(SECOND)
  expect(transcripts()).toBe(3)
  // Another page made a conversation this one never opened.
  const third = record('00000000-0000-4000-8000-000000000003')
  records.push(third)
  await changed(third.id)
  expect(store.items.some((item) => item.id === third.id)).toBe(true)
  third.hostsRevision = 1
  await changed(third.id)
  expect(hostReads(third.id)).toBe(0)
})

test('a conversation removed and brought back reads its history again', async () => {
  serveHistory()
  const store = useConversations()
  await store.activate(FIRST)
  await store.activate(null)
  const removed = records.shift()!
  await reconnected()
  records.unshift(removed)
  await reconnected()
  await store.activate(FIRST)
  expect(requests.filter((item) => item.path.endsWith('/transcript'))).toHaveLength(2)
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
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json({ ...EMPTY_PAGE, length: 1, blocks: [block] })
    return originalFetch(input, init)
  }) as typeof fetch
  const models = useProduct().loadModels(true).catch(error => error)
  const opening = store.activate(current.id)
  await new Promise(resolve => setTimeout(resolve, 0))
  expect(current.lastError).toBeNull()
  expect(current.load).toBe('ready')
  expect(latestBlocks(current.history)[0]?.id).toBe('visible-history')
  modelResponse.reject(new Error('Catalog offline'))
  expect(await models).toBeInstanceOf(Error)
  await opening
  expect(current.load).toBe('ready')
  expect(latestBlocks(current.history)[0]?.id).toBe('visible-history')
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
    serverDrafts.set(FIRST, { revision: 1, text: 'Fix the login', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 1
    const store = useConversations()
    const current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    expect([current.draft, current.record.revision('draft')]).toEqual(['Fix the login', 1])

    // Nothing typed here: the newer draft takes the composer's place.
    const shown = current.draftShown
    await savedElsewhere(FIRST, 'Fix the login test')
    await until(() => current.draft === 'Fix the login test', 'the other page\'s draft shows')
    expect([current.record.revision('draft'), current.draftShown]).toEqual([2, shown + 1])

    // Typed here, not saved yet: the composer keeps it when another page saves.
    current.draft = 'Fix the login bug'
    await savedElsewhere(FIRST, 'Fix the login test, again')
    await until(() => current.record.revision('draft') === 3, 'the newer draft is read')
    expect(current.draft).toBe('Fix the login bug')
    expect(draftSaves()).toEqual([])

    // Half a second after the typing pauses the page saves, built on the
    // revision its text came from, so the backend keeps what it replaces.
    jest.advanceTimersByTime(DRAFT_SAVE_DELAY_MS)
    await until(() => current.record.revision('draft') === 4, 'the save is answered')
    expect(draftSaves().map((request) => request.body)).toEqual([{ base: 2, text: 'Fix the login bug', files: [], comments: [] }])
    expect([current.draft, current.record.pending('draft'), current.record.read.value?.draft.replaced?.text]).toEqual(['Fix the login bug', [], 'Fix the login test, again'])

    // Restore brings the replaced version back and offers the one it displaced.
    store.restoreReplaced(current)
    await until(() => current.draft === 'Fix the login test, again', 'the restored draft shows')
    expect(current.record.read.value?.draft.replaced?.text).toBe('Fix the login bug')
    store.dismissReplaced(current)
    await until(() => current.record.read.value?.draft.replaced === null, 'the offer is dismissed')
    expect([current.draft, current.record.revision('draft')]).toEqual(['Fix the login test, again', 6])
  } finally {
    jest.useRealTimers()
    signOut()
  }
})

// Send to Conversation: the words go to the end of the chosen conversation's
// draft as a paste there adds them, sent before it opened, so they wait for the
// draft the backend has, and nothing goes to the agent.
test.each([
  { source: 'the user’s whole message, its file with it', words: () => contentCopy(withPicture('m').content), text: `Fix the login\n\nCompare ${ATTACHMENT_MARK} with the old one`, files: 1 },
  { source: 'text selected in it', words: () => textWords('the cookie rename'), text: 'Fix the login\n\nthe cookie rename', files: 0 },
  { source: 'text selected in the agent’s reply', words: () => quotedWords('Point it at packages/utils/src.\nThen rebuild.'), text: 'Fix the login\n\n> Point it at packages/utils/src.\n> Then rebuild.', files: 0 },
])('$source goes to the end of another conversation’s draft once it is read, and nothing is sent', async ({ words, text, files }) => {
  const signOut = signIn()
  const restoreClaims = answerClaims(Promise.resolve())
  const claimsFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
    return claimsFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit')
  try {
    serverDrafts.set(FIRST, { revision: 1, text: 'Fix the login', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 1
    const store = useConversations()
    const current = store.items.find((item) => item.id === FIRST)!
    store.appendToDraft(current, words())
    expect(current.draft).toBe('')
    await store.activate(FIRST)
    await until(() => current.draft !== '' && current.draft !== 'Fix the login', 'the words are added')
    expect(current.draft).toBe(text)
    expect(current.attachmentIds).toHaveLength(files)
    expect([submit.mock.calls.length, current.pendingSends.length]).toEqual([0, 0])
  } finally {
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = claimsFetch
    restoreClaims()
    signOut()
  }
})

test('a send empties the draft everywhere once the backend accepts it, and a change another page made meanwhile stays to restore', async () => {
  const signOut = signIn()
  const store = useConversations()
  const current = store.items.find((item) => item.id === FIRST)!
  addStubProvider()
  serverDrafts.set(FIRST, { revision: 1, text: 'Ship it', files: [], comments: [], replaced: null })
  records[0]!.draftRevision = 1
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
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
    serverDrafts.set(FIRST, { revision: 2, text: 'Ship it today', files: [], comments: [], replaced: null })
    jest.advanceTimersByTime(DRAFT_SAVE_DELAY_MS)
    await settle()
    expect(draftSaves()).toEqual([])
    accepted.resolve()
    await sending
    await until(() => draftSaves().length === 1, 'the draft is cleared')
    expect(draftSaves()[0]!.body).toEqual({ base: 1, text: '', files: [], comments: [] })
    await until(() => current.record.revision('draft') === 3, 'the clear is answered')
    const read = current.record.read.value?.draft
    expect([current.draft, read?.text, read?.replaced?.text]).toEqual(['', '', 'Ship it today'])
  } finally {
    jest.useRealTimers()
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
    signOut()
  }
})

// Comments wait in the draft as its text does: another page's show here, one
// saved here is saved at once, and Send carries them, in the order the
// conversation reads them, with the typed text, emptying the draft once the
// backend accepts it.
test('waiting comments sync with the draft, and a send or a steer carries them in reading order and empties them everywhere', async () => {
  const signOut = signIn()
  const store = useConversations()
  const current = store.items.find((item) => item.id === FIRST)!
  addStubProvider()
  const later = { id: 'c-later', blockId: 'text-1', start: 40, end: 66, at: 40, quote: 'the same 30-second timeout', text: 'Its own timeout?' }
  const earlier = { id: 'c-earlier', blockId: 'text-1', start: 4, end: 29, at: 4, quote: 'Upload in chunks of 4 MB.', text: 'Smaller?' }
  serverDrafts.set(FIRST, { revision: 1, text: 'Otherwise go ahead', files: [], comments: [], replaced: null })
  records[0]!.draftRevision = 1
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path.endsWith('/hosts')) return Response.json({ hosts: [] })
    if (path.endsWith('/subagents')) return Response.json({ subagents: [] })
    if (path.endsWith('/transcript')) return Response.json(EMPTY_PAGE)
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
  const steer = spyOn(ConversationRuntime.prototype, 'steer').mockResolvedValue()
  try {
    await useProduct().loadModels(true)
    await store.activate(FIRST)
    // Another page saves a comment: this page's composer shows it waiting.
    serverDrafts.set(FIRST, { ...serverDrafts.get(FIRST)!, revision: 2, comments: [later] })
    records[0]!.draftRevision = 2
    await changed(FIRST)
    await until(() => current.comments.length === 1, 'the other page\'s comment waits here')
    // A comment saved here goes to the backend at once, with the text.
    store.commenting(current).add(earlier)
    await until(() => draftSaves().length === 1, 'the comment is saved')
    expect(draftSaves()[0]!.body).toEqual({ base: 2, text: 'Otherwise go ahead', files: [], comments: [later, earlier] })
    await until(() => current.record.revision('draft') === 3, 'the save is answered')

    await store.send(current)
    expect(submit.mock.calls.map((call) => [call[0], call[2]])).toEqual([
      [[{ type: 'text', text: 'Otherwise go ahead' }], [earlier, later]],
    ])
    expect(current.comments).toEqual([])
    await until(() => draftSaves().length === 2, 'the draft is cleared')
    expect(draftSaves()[1]!.body).toEqual({ base: 3, text: '', files: [], comments: [] })

    // While the agent works, a steer carries its comments as a message does.
    await until(() => current.record.revision('draft') === 4, 'the clearing is answered')
    store.commenting(current).add(later)
    current.draft = 'Then this'
    await store.send(current, 'steer')
    expect(steer.mock.calls.map((call) => [call[0], call[2]])).toEqual([[[{ type: 'text', text: 'Then this' }], [later]]])
    expect(submit.mock.calls).toHaveLength(1)
    expect(current.comments).toEqual([])
  } finally {
    steer.mockRestore()
    submit.mockRestore()
    connect.mockRestore()
    globalThis.fetch = originalFetch
    signOut()
  }
})

// A pasted file whose attaching the page did not see end is saved with the
// draft as its stored file, and attaches again when the draft is restored,
// or falls back as a paste does.
test('a draft saved while its pasted files attach restores them attaching, and their claims finish or fall back', async () => {
  let signOut = signIn()
  const saved = new Map<string, draftStorage.SavedDraft>()
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    saved.set(id, structuredClone(draft))
  })
  // The first page's claims never answer: it closes while they are under way.
  let restoreFetch = answerClaims(new Promise(() => {}))
  try {
    serverDrafts.set(FIRST, { revision: 1, text: '', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 1
    let store = useConversations()
    let current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    const held = store.claimFile(current, { type: 'stored', name: 'before.png', sha256: SHA, mediaType: 'image/png' }, null)
    const gone = store.claimFile(current, { type: 'stored', name: 'gone.png', sha256: 'c'.repeat(64), mediaType: 'image/png' }, null)
    store.arrangeFiles(current, [held.item.id, gone.item.id])
    current.draft = `Compare ${ATTACHMENT_MARK} with ${ATTACHMENT_MARK}`
    store.saveDrafts()
    expect(saved.get(FIRST)?.files).toMatchObject([
      { name: 'before.png', upload: null, claim: { file: { sha256: SHA }, picture: null } },
      { name: 'gone.png', upload: null, claim: { file: { sha256: 'c'.repeat(64) } } },
    ])
    signOut()
    store.stopAll()
    useProduct().stop()
    disposePinia(pinia)
    restoreFetch()

    // The next page restores the draft; the backend answers its claims now.
    restoreFetch = answerClaims(Promise.resolve())
    pinia = createPinia()
    setActivePinia(pinia)
    signOut = signIn(saved.get(FIRST)!)
    store = useConversations()
    await connect()
    current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    const restored = current.files.find((file) => file.name === 'before.png')
    await until(() => restored?.kind === 'file' && restored.phase === 'ready', 'the restored claim finishes')
    expect(restored).toMatchObject({ upload: { id: 'att-held', sha256: SHA } })
    // The file the user's blobs do not hold, without a picture, is its name in the text.
    await until(() => current.draft === `Compare ${ATTACHMENT_MARK} with gone.png`, `the draft is ${current.draft}`)
    expect(current.attachmentIds).toEqual([restored!.id])
  } finally {
    restoreFetch()
    writes.mockRestore()
    signOut()
  }
})

test('a page that closes while typing leaves its text to the next page, which takes the backend\'s revision without saving it again', async () => {
  // The first page types, and closes before its save and its IndexedDB write land.
  let signOut = signIn()
  const kept = spyOn(draftStorage, 'writeLastWords').mockImplementation(() => {})
  let words: draftStorage.LastWords | undefined
  try {
    serverDrafts.set(FIRST, { revision: 1, text: 'Fix the lo', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 1
    let store = useConversations()
    let current = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    current.draft = 'Fix the login bug'
    store.keepLastWords()
    words = kept.mock.calls.find(([, id]) => id === FIRST)?.[2]
    expect(words).toEqual({ composer: { base: 1, text: 'Fix the login bug', attachmentIds: [] }, pendingSends: [], files: [] })
    signOut()
    store.stopAll()
    useProduct().stop()
    disposePinia(pinia)

    // Its save on close arrived; its IndexedDB record holds an older text.
    serverDrafts.set(FIRST, { revision: 2, text: 'Fix the login bug', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 2
    pinia = createPinia()
    setActivePinia(pinia)
    signOut = signIn({
      messageEdit: null, pendingSends: [], local: null, base: 1, text: 'Fix the lo',
      model: null, files: [], scroll: null, notSaved: [], comments: [],
    })
    const taken = spyOn(draftStorage, 'takeLastWords').mockReturnValue(words ?? null)
    jest.useFakeTimers()
    try {
      store = useConversations()
      await connect()
      current = store.items.find((item) => item.id === FIRST)!
      await store.activate(FIRST)
      expect([current.draft, current.record.revision('draft'), current.record.pending('draft')]).toEqual(['Fix the login bug', 2, []])
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

// A child's context card shows the child's own context: its usage lands on its
// record and leaves the root's, which the runtime keeps for the composer,
// alone.
test('a child\'s usage frame sets its own record\'s usage', () => {
  const conversation = useConversations().items[0]!
  conversation.contextUsage = null
  applyConversationEvent(conversation, {
    type: 'subagent',
    event: 'started',
    job: {
      subagentId: 'child', number: 1, parentSessionId: conversation.id, onPath: true, description: 'Serve',
      profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work', phase: 'running', startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
    },
  })
  const usage = { tokens: 60_000, window: 100_000, compactFrom: 50_000, rows: [], account: null }
  applyConversationEvent(conversation, { type: 'context_usage', node: 'child', usage })
  expect(conversation.subagents.find((agent) => agent.id === 'child')!.contextUsage).toEqual(usage)
  applyConversationEvent(conversation, { type: 'context_usage', node: conversation.id, usage: { ...usage, tokens: 1_000 } })
  expect(conversation.subagents.find((agent) => agent.id === 'child')!.contextUsage).toEqual(usage)
  expect(conversation.contextUsage).toBeNull()
})

/**
 * A conversation opening with a session whose `/cost` reads wait for the
 * test: each read is a request the test answers with a cost, or with a
 * response, as a failure, and whose signal it can look at. The opening's
 * own read has been sent; the opening ends once the test answers it.
 */
async function costScene() {
  const totals = (input: number, price: number | null) => ({
    inputTokens: input, cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 0,
    price: { total: price, input: price, cacheWrite: null, cacheRead: null, output: null },
  })
  const call = { startedAt: '2026-09-13T00:00:00.000Z', inputTokens: 10, cacheWriteTokens: 0, cacheReadTokens: 90, price: 0.5, afterCompaction: false, miss: null }
  const frame = (rows: number, agent: number, price: number | null): ServerFrame => ({
    type: 'cost',
    rows,
    totals: totals(10, price),
    agent,
    profile: null,
    call,
  })
  const cost = (rows: number, input: number, price: number) => ({
    rows, totals: totals(input, price), agents: [{ agent: 0, profile: null, price }], calls: [call],
  })
  const reads: Array<{ answer: ReturnType<typeof deferred<unknown>>, signal: AbortSignal | null, agent: string | null }> = []
  const answering = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const url = new URL(String(input), 'https://demi.test')
    if (!url.pathname.endsWith(`/conversations/${FIRST}/cost`))
      return answering(input, init)
    const read = { answer: deferred<unknown>(), signal: init?.signal ?? null, agent: url.searchParams.get('agent') }
    reads.push(read)
    const answer = await read.answer.promise
    return answer instanceof Response ? answer : Response.json(answer)
  }) as typeof fetch
  const { conversation, sockets, opening } = await startOpening(FIRST, { handshake: [] })
  await waitFor(() => reads.length === 1, () => 'the opening did not read the cost')
  return { conversation, sockets, opening, store: useConversations(), totals, call, frame, cost, reads }
}

// Production: the context card showed without its Cost, which popped in
// when the read it sent as it opened returned. The page reads the cost with
// the conversation's other reads, and the transcript shows once it holds
// it, so the card is whole the moment it opens
// (`packages/web-ui/src/agent/context-cost.ts`). Cost: one opening, one read,
// no wait.
test('a conversation shows once the page holds its cost, read with its transcript', async () => {
  const { conversation, opening, cost, reads } = await costScene()
  const shownWith: unknown[] = []
  watch(() => conversation.load, (load) => {
    if (load !== 'loading')
      shownWith.push(conversation.cost)
  }, { flush: 'sync' })
  await nextTick()
  expect(conversation.load).toBe('loading')
  reads[0]!.answer.resolve(cost(3, 100, 1))
  await opening
  expect(shownWith[0]).toEqual({ status: 'read', cost: cost(3, 100, 1) })
  expect(reads).toHaveLength(1)
})

// The page adds each `cost` frame that follows what it holds
// (`packages/web-ui/src/agent/context-cost.ts`): a frame it holds is dropped, one past
// a gap, as after frames lost while the connection was down, reads the cost
// again, and of the frames that came while that read was on its way, one
// its answer includes is dropped and one it does not is added after it.
// Cost: one opening, two reads, no wait.
test('the cost a page holds adds each frame that follows it, drops one it holds and reads again past a gap', async () => {
  const { conversation, sockets, opening, totals, call, frame, reads } = await costScene()
  reads[0]!.answer.resolve({ rows: 1, totals: totals(100, 1), agents: [{ agent: 0, profile: null, price: 1 }], calls: [call] })
  await opening

  sockets[0]!.receive(frame(2, 2, 0.25), frame(1, 0, 0.5))
  await nextTick()
  expect(conversation.cost).toEqual({
    status: 'read',
    cost: {
      rows: 2,
      totals: totals(110, 1.25),
      agents: [
        { agent: 0, profile: null, price: 1 },
        { agent: 2, profile: null, price: 0.25 },
      ],
      calls: [call],
    },
  })

  // Row 3 was lost: row 4 reads the cost again, and the card shows what it
  // holds until the answer comes. Rows 5 and 6 are written meanwhile, and
  // the answer includes row 5 but not row 6.
  sockets[0]!.receive(frame(4, 0, 0.5))
  await waitFor(() => reads.length === 2, () => 'a gap did not read the cost again')
  expect(conversation.cost).toMatchObject({ status: 'read', cost: { rows: 2 } })
  sockets[0]!.receive(frame(5, 0, 0.5), frame(6, 3, null))
  reads[1]!.answer.resolve({ rows: 5, totals: totals(140, 2), agents: [{ agent: 0, profile: null, price: 2 }], calls: [call, call] })
  await waitFor(() => conversation.cost?.status === 'read' && conversation.cost.cost.rows === 6, () => 'the answer was not kept with the frame after it')
  expect(conversation.cost).toEqual({
    status: 'read',
    cost: { rows: 6, totals: totals(150, 2), agents: [{ agent: 0, profile: null, price: 2 }], calls: [call, call] },
  })
  expect(reads).toHaveLength(2)
})

// A subagent's context card shows the child's own cost: the page reads it,
// `?agent=` the child's number, as the child's window opens, adds a frame of
// that child, counts a frame of any other agent without adding it, reads
// again past a gap, and lets it go as the window closes
// (`decisions/0001-agent-cost.md`). Cost: one opening, four reads, no wait.
test("a shown child reads its own cost, ignores another agent's frame and reads again past a gap", async () => {
  const { conversation, sockets, opening, store, totals, call, frame, reads } = await costScene()
  reads[0]!.answer.resolve({ rows: 1, totals: totals(100, 1), agents: [{ agent: 0, profile: null, price: 1 }], calls: [call] })
  await opening
  sockets[0]!.receive({
    type: 'subagent',
    event: 'started',
    job: {
      subagentId: 'child', number: 1, parentSessionId: conversation.id, onPath: true, description: 'Serve',
      profile: null, model: RUNS_ON, round: 1, forkedFrom: null, cwd: '/work', phase: 'running', startedAt: '2026-09-13T00:00:00.000Z', endedAt: null,
    },
  })
  await nextTick()
  expect(reads).toHaveLength(1)

  store.showSubagents(conversation, ['child'])
  await waitFor(() => reads.length === 2, () => "the window did not read the child's cost")
  expect(reads[1]!.agent).toBe('1')
  reads[1]!.answer.resolve({ rows: 1, totals: totals(0, null), agents: [], calls: [] })
  await waitFor(() => conversation.subagentCosts['child']?.status === 'read', () => "the child's cost was not kept")

  // The main conversation's row is counted, the child's added.
  sockets[0]!.receive(frame(2, 0, 0.5), frame(3, 1, 0.25))
  await nextTick()
  expect(conversation.subagentCosts['child']).toEqual({
    status: 'read',
    cost: { rows: 3, totals: totals(10, 0.25), agents: [{ agent: 1, profile: null, price: 0.25 }], calls: [call] },
  })
  // The conversation's cost adds both, and the main conversation's call alone.
  expect(conversation.cost?.status === 'read' && [conversation.cost.cost.rows, conversation.cost.cost.calls.length]).toEqual([3, 2])

  // Row 4 was lost: row 5 reads both costs again.
  sockets[0]!.receive(frame(5, 0, 0.5))
  await waitFor(() => reads.filter((read) => read.agent === '1').length === 2, () => "a gap did not read the child's cost again")
  expect(reads.filter((read) => read.agent === null)).toHaveLength(2)

  store.showSubagents(conversation, [])
  expect(reads.at(-1)!.signal?.aborted).toBe(true)
  expect(conversation.subagentCosts).toEqual({})
})

// A call that ends while the page's connection is down leaves the cost
// stale with no later frame to show it: the connection that attaches again
// brings the conversation's row count, and a page that holds fewer reads the
// cost again; one that holds as many reads nothing
// (`packages/web-ui/src/agent/context-cost.ts`). Cost: two sockets, two reads,
// no wait.
test('a page whose connection missed the last row reads the cost again as it attaches again', async () => {
  const { conversation, sockets, opening, cost, reads } = await costScene()
  reads[0]!.answer.resolve(cost(2, 100, 1))
  await opening

  // The connection drops; row 3 is written meanwhile, its frame lost.
  sockets[0]!.end()
  reconnectNow()
  await waitFor(() => sockets[1]?.sent.some((sent) => sent.type === 'open') === true, () => 'no second open')
  sockets[1]!.receive({ type: 'opened' }, { type: 'phase', phase: 'idle' }, { type: 'cost_rows', rows: 3 })
  await waitFor(() => reads.length === 2, () => 'the attach did not read the cost again')
  reads[1]!.answer.resolve(cost(3, 110, 1.5))
  await waitFor(() => conversation.cost?.status === 'read' && conversation.cost.cost.rows === 3, () => 'the second answer was not kept')

  // Attached again with nothing missed, the page reads nothing.
  sockets[1]!.end()
  reconnectNow()
  await waitFor(() => sockets[2]?.sent.some((sent) => sent.type === 'open') === true, () => 'no third open')
  sockets[2]!.receive({ type: 'opened' }, { type: 'phase', phase: 'idle' }, { type: 'cost_rows', rows: 3 })
  await nextTick()
  expect(reads).toHaveLength(2)
})

// A read of the cost that fails shows the failure with its request's id,
// never an empty section a user would take for no calls, and the
// conversation opens all the same; Retry reads it again
// (`packages/web-ui/src/agent/ContextUsageIndicator.vue`). Cost: one opening, two
// reads, no wait.
test('a failed read of the cost shows its failure, and Retry reads it again', async () => {
  const { conversation, opening, store, cost, reads } = await costScene()
  reads[0]!.answer.resolve(new Response(
    JSON.stringify({ code: 'internal_error', message: 'The database is busy' }),
    { status: 500, headers: { 'Content-Type': 'application/json', 'X-Request-Id': 'req_cost00000000001' } },
  ))
  await opening
  expect(conversation.load).toBe('ready')
  expect(conversation.cost).toMatchObject({ status: 'failed', error: { requestId: 'req_cost00000000001' } })

  store.retryCost(conversation)
  await waitFor(() => reads.length === 2, () => 'Retry did not read the cost')
  reads[1]!.answer.resolve(cost(1, 100, 1))
  await waitFor(() => conversation.cost?.status === 'read', () => 'the answer was not kept')
  expect(conversation.cost).toEqual({ status: 'read', cost: cost(1, 100, 1) })
})

// A conversation that goes from the page while a read of its cost has no
// answer yet, its opening's or the one a gap started, cancels that read,
// and an answer that comes anyway is shown nowhere: each read lives as long
// as the conversation's stay in the page's cache
// (`packages/web-ui/src/agent/context-cost.ts`). Cost: one opening and one
// read, then one opening and two
// reads, no wait.
test('a conversation that goes while its opening reads its cost cancels the read', async () => {
  const { conversation, opening, cost, reads } = await costScene()
  channels.last().send({ type: 'conversation_deleted', id: FIRST })
  await nextTick()
  expect(reads[0]!.signal?.aborted).toBe(true)
  // The stub answers whatever the signal says, as a response already on its way does.
  reads[0]!.answer.resolve(cost(1, 100, 1))
  await opening.catch(() => {})
  expect(conversation.cost).toBeNull()
})

test('a conversation that goes cancels the read of its cost a gap started', async () => {
  const { conversation, sockets, opening, cost, frame, reads } = await costScene()
  reads[0]!.answer.resolve(cost(1, 100, 1))
  await opening
  sockets[0]!.receive(frame(3, 0, 0.5))
  await waitFor(() => reads.length === 2, () => 'a gap did not read the cost again')
  channels.last().send({ type: 'conversation_deleted', id: FIRST })
  await nextTick()
  expect(reads[1]!.signal?.aborted).toBe(true)
  reads[1]!.answer.resolve(cost(3, 100, 2))
  await nextTick()
  expect(conversation.cost).toEqual({ status: 'read', cost: cost(1, 100, 1) })
})

// A page that signs out lets the cost's read go: the read is cancelled, and
// an answer that comes anyway is kept nowhere
// (`packages/web-ui/src/agent/context-cost.ts`). Cost: one opening, two reads,
// no wait.
test('the cost\'s read goes as the page signs out', async () => {
  const { conversation, sockets, opening, store, cost, frame, reads } = await costScene()
  reads[0]!.answer.resolve(cost(1, 100, 1))
  await opening
  sockets[0]!.receive(frame(3, 0, 0.5))
  await waitFor(() => reads.length === 2, () => 'a gap did not read the cost again')
  store.stopAll()
  expect(reads[1]!.signal?.aborted).toBe(true)
  reads[1]!.answer.resolve(cost(3, 100, 1))
  await nextTick()
  expect(conversation.cost).toEqual({ status: 'read', cost: cost(1, 100, 1) })
})

// Under 50 ms each: the backend is the stub above.
test('the messages saved for later show as the conversation opens, and a raised revision reads them again, once', async () => {
  serveHistory()
  serverSaved.set(FIRST, [savedMessage('dns', 'Check prod’s DNS next')])
  records[0]!.savedRevision = 1
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  const reads = (id: string) => requests.filter((item) => item.path === `/api/conversations/${id}/saved`).length
  await store.activate(FIRST)
  await until(() => first.record.revision('saved') === 1, 'the list is read as the conversation opens')
  expect(listed(first)).toEqual(['Check prod’s DNS next'])

  // Another page saved one: the summary's revision moves ahead, the page
  // reads the list again; the same summary again reads nothing.
  serverSaved.set(FIRST, [savedMessage('ipv6', 'Then the IPv6'), ...serverSaved.get(FIRST)!])
  records[0]!.savedRevision = 2
  await changed(FIRST)
  await changed(FIRST)
  await until(() => first.record.revision('saved') === 2, 'the list is read again')
  expect(listed(first)).toEqual(['Then the IPv6', 'Check prod’s DNS next'])
  await settle()
  expect(reads(FIRST)).toBe(2)

  // A conversation this page never opened reads nothing.
  records[1]!.savedRevision = 1
  await changed(SECOND)
  await settle()
  expect(reads(SECOND)).toBe(0)
})

/**
 * Holds the writes of messages saved for later and of the conversation's
 * record: each reaches the backend and waits for the test, which answers it
 * with a response of its own, passes it on to the stub backend, or has the
 * backend make it now and the answer arrive later.
 */
function holdWrites(options: { drafts?: boolean } = {}) {
  const waiting: {
    method: string
    path: string
    body: unknown
    answer: (response: Response) => void
    pass: () => Promise<void>
    apply: () => Promise<() => void>
  }[] = []
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const method = init?.method ?? 'GET'
    const holds = method !== 'GET' && (/\/saved(\/|$)/.test(path) || /^\/api\/conversations\/(batch|[^/]+)$/.test(path) ||
      (options.drafts === true && path.endsWith('/draft')))
    if (!holds) {
      return originalFetch(input, init)
    }
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    if (!path.endsWith('/draft')) {
      requests.push({ path, body })
    }
    return new Promise<Response>((answer) => waiting.push({
      method,
      path,
      body,
      answer,
      pass: async () => answer(await originalFetch(input, init)),
      apply: async () => {
        const response = await originalFetch(input, init)
        return () => answer(response)
      },
    }))
  }) as typeof fetch
  return { waiting, restore: () => void (globalThis.fetch = originalFetch) }
}

const refusal = (code: string, status = 409) =>
  Response.json({ requestId: 'req_test000000000001', code, message: `The backend refuses: ${code}` }, { status })
const savedMessage = (id: string, text: string): SavedMessage => ({ id, text, files: [], savedAt: '2026-09-09T00:00:00.000Z' })
/** The texts of the list the conversation's composer shows, without the messages not saved. */
const listed = (conversation: { record: { shown: { value: { saved: SavedMessage[] } | null } } }) =>
  (conversation.record.shown.value?.saved ?? []).map((message) => message.text)

/** The conversation `FIRST` opened with its draft and its list of messages saved for later read. */
async function openWithSaved(saved: SavedMessage[]) {
  serveHistory()
  serverSaved.set(FIRST, saved)
  records[0]!.savedRevision = 1
  const store = useConversations()
  const first = store.items.find((item) => item.id === FIRST)!
  await store.activate(FIRST)
  await until(() => first.record.revision('saved') === 1 && first.record.revision('draft') === 0, 'the list and the draft are read')
  return { store, first }
}

/** The backend's summary of `FIRST` reaches the page, which reads what changed, and holds it. */
async function readAgain(first: { record: { revision: (part: string) => number } }): Promise<void> {
  await changed(FIRST)
  await until(
    () => first.record.revision('saved') === records[0]!.savedRevision && first.record.revision('draft') === records[0]!.draftRevision,
    'the draft and the list are read again',
  )
}

test('Save for Later lists the composer\'s message at once under the id its request names, sent on the revision it was built on', async () => {
  const signOut = signIn()
  try {
    serveHistory()
    serverDrafts.set(FIRST, { revision: 1, text: 'Check prod’s DNS', files: [], comments: [], replaced: null })
    records[0]!.draftRevision = 1
    const store = useConversations()
    const first = store.items.find((item) => item.id === FIRST)!
    await store.activate(FIRST)
    await until(() => first.record.revision('draft') === 1, 'the draft is read')
    first.draft = 'Check prod’s DNS next'
    const held = holdWrites()
    try {
      const saving = store.saveForLater(first)
      await until(() => held.waiting.length === 1, 'the save reaches the backend')
      expect([first.draft, listed(first)]).toEqual(['', ['Check prod’s DNS next']])
      expect(held.waiting[0]!.body).toEqual({ id: first.record.shown.value!.saved[0]!.id, base: 1, text: 'Check prod’s DNS next', files: [] })
      await held.waiting[0]!.pass()
      await saving
    } finally {
      held.restore()
    }
    await readAgain(first)
    expect([first.draft, listed(first), first.record.pending()]).toEqual(['', ['Check prod’s DNS next'], []])
  } finally {
    signOut()
  }
})

// Cost: no timer but the page's own save delay; each held request is passed on by the test.
test('a comment saved while an earlier draft save is on its way survives Save for Later', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([])
  const held = holdWrites({ drafts: true })
  const earlier = { id: 'c0', blockId: 'text-1', start: 8, end: 12, at: 8, quote: 'fail', text: 'Since when?' }
  const comment = { id: 'c1', blockId: 'text-1', start: 0, end: 7, at: 0, quote: 'Uploads', text: 'Which ones?' }
  try {
    // A comment is saved at once; its save is on its way.
    store.commenting(first).add(earlier)
    await until(() => held.waiting.length === 1, 'the first comment\'s save reaches the backend')
    // The user types, saves another comment, and saves the message for later before the first save is answered.
    first.draft = 'Check the logs'
    store.commenting(first).add(comment)
    const saving = store.saveForLater(first)
    await until(() => listed(first).length === 1, 'the message stands in the list')
    expect([first.draft, listed(first), first.comments]).toEqual(['', ['Check the logs'], [earlier, comment]])
    await held.waiting[0]!.pass()
    await until(() => held.waiting.length === 2, 'the save for later reaches the backend')
    await held.waiting[1]!.pass()
    await saving
    await until(() => held.waiting.length === 3, 'the comment follows in a save of its own')
    expect(held.waiting[2]!.body).toMatchObject({ text: '', files: [], comments: [earlier, comment] })
    await held.waiting[2]!.pass()
    await readAgain(first)
    expect([first.draft, listed(first), first.comments, serverDrafts.get(FIRST)!.comments])
      .toEqual(['', ['Check the logs'], [earlier, comment], [earlier, comment]])
  } finally {
    held.restore()
    signOut()
  }
})

// The reviewer's first sequence: Save A, type B before the answer, the save is refused.
test('a save refused after its user typed on loses nothing: the message stands as Not Saved and the composer keeps what was typed', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next')])
  const held = holdWrites()
  try {
    first.draft = 'Look at the IPv6 setup'
    const saving = store.saveForLater(first)
    await until(() => held.waiting.length === 1, 'the save reaches the backend')
    expect([first.draft, listed(first)]).toEqual(['', ['Look at the IPv6 setup', 'Check prod’s DNS next']])
    first.draft = 'Then the CDN'
    held.waiting[0]!.answer(refusal('saved_full'))
    await saving
    expect(first.draft).toBe('Then the CDN')
    expect(listed(first)).toEqual(['Check prod’s DNS next'])
    expect(first.notSaved.map(({ text }) => text)).toEqual(['Look at the IPv6 setup'])
  } finally {
    held.restore()
    signOut()
  }
})

// The reviewer's second sequence: composer D, Edit A, Edit B, A refused, B
// accepted, B pressed while A still waits for the save of D.
test('two edits in a row, the first refused: nothing is lost and no message is listed twice', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([savedMessage('a', 'Message A'), savedMessage('b', 'Message B')])
  const held = holdWrites({ drafts: true })
  try {
    first.draft = 'Draft D'
    await store.editSaved(first, 'a')
    await until(() => held.waiting.length === 1, 'the save of what the composer held reaches the backend first')
    expect(held.waiting[0]!.body).toMatchObject({ text: 'Draft D' })
    expect([first.draft, listed(first)]).toEqual(['Message A', ['Draft D', 'Message B']])
    await store.editSaved(first, 'b')
    expect([first.draft, listed(first)]).toEqual(['Message B', ['Message A', 'Draft D']])

    await held.waiting[0]!.pass()
    await until(() => held.waiting.length === 2, 'the first edit reaches the backend')
    held.waiting[1]!.answer(refusal('saved_not_found', 404))
    await until(() => held.waiting.length === 3, 'the second edit reaches the backend')
    // A stays listed; B's edit now moves the draft as the backend holds it, D.
    expect([first.draft, listed(first), first.notSaved]).toEqual(['Message B', ['Draft D', 'Message A'], []])
    await held.waiting[2]!.pass()
    await readAgain(first)
    expect([first.draft, listed(first), first.notSaved, first.record.pending()]).toEqual(['Message B', ['Draft D', 'Message A'], [], []])
  } finally {
    held.restore()
    signOut()
  }
})

test('an edit refused leaves the message in the list and what the composer held as Not Saved, shown once', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([savedMessage('a', 'Message A')])
  const held = holdWrites()
  try {
    first.draft = 'Draft K'
    await store.editSaved(first, 'a')
    await until(() => held.waiting.length === 1, 'the edit reaches the backend')
    held.waiting[0]!.answer(refusal('conversation_archived'))
    await until(() => first.notSaved.length === 1, 'the composer\'s message stands as Not Saved')
    expect([first.draft, listed(first), first.notSaved.map(({ text }) => text)]).toEqual(['', ['Message A'], ['Draft K']])
  } finally {
    held.restore()
    signOut()
  }
})

// The reviewer's fifth finding: the list the edit made arrives before its answer.
test('an edit whose list is read before its answer lists what the composer held once', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([savedMessage('a', 'Message A')])
  const held = holdWrites()
  try {
    first.draft = 'Draft K'
    await store.editSaved(first, 'a')
    await until(() => held.waiting.length === 1, 'the edit reaches the backend')
    const answer = await held.waiting[0]!.apply()
    await readAgain(first)
    expect([first.draft, listed(first)]).toEqual(['Message A', ['Draft K']])
    answer()
    await until(() => first.record.pending().length === 0, 'the edit leaves')
    expect([first.draft, listed(first)]).toEqual(['Message A', ['Draft K']])
  } finally {
    held.restore()
    signOut()
  }
})

test('Retry on a message not saved saves it under its own id, and it is listed as saved once read', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([])
  const held = holdWrites()
  try {
    first.draft = 'Look at the IPv6 setup'
    const saving = store.saveForLater(first)
    await until(() => held.waiting.length === 1, 'the save reaches the backend')
    held.waiting[0]!.answer(refusal('saved_full'))
    await saving
    const [row] = first.notSaved
    store.retrySaved(first, row!.id)
    await until(() => held.waiting.length === 2, 'the retry reaches the backend')
    expect(held.waiting[1]!.body).toEqual({ id: row!.id, text: 'Look at the IPv6 setup', files: [] })
    expect(listed(first)).toEqual(['Look at the IPv6 setup'])
    await held.waiting[1]!.pass()
    await readAgain(first)
    expect([listed(first), first.notSaved, first.record.pending()]).toEqual([['Look at the IPv6 setup'], [], []])
  } finally {
    held.restore()
    signOut()
  }
})

/** The model the stub provider offers, chosen for `FIRST`, whose runtime does not connect. */
async function withStubModel(first: Parameters<ReturnType<typeof useConversations>['changeModel']>[0]) {
  addStubProvider()
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) return Response.json(stubCatalog())
    if (path === `/api/conversations/${FIRST}` && init?.method === 'PATCH') {
      return Response.json({ conversation: records[0], results: [] })
    }
    return originalFetch(input, init)
  }) as typeof fetch
  const connect = spyOn(ConversationRuntime.prototype, 'connect').mockResolvedValue()
  await useProduct().loadModels(true)
  await useConversations().changeModel(first, { model: { providerId: 'stub', modelId: 'stub' } })
  return () => {
    connect.mockRestore()
    globalThis.fetch = originalFetch
  }
}

test('sending a saved message shows it on its way at once as a turn of its own, out of the list; one whose send fails stays on its way with Retry', async () => {
  const { store, first } = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next'), savedMessage('ipv6', 'Then the IPv6')])
  const restore = await withStubModel(first)
  const answer = deferred<boolean>()
  const send = spyOn(ConversationRuntime.prototype, 'sendSaved').mockImplementation(() => answer.promise)
  try {
    store.sendSaved(first, 'dns', 'send')
    expect(listed(first)).toEqual(['Then the IPv6'])
    expect(first.pendingSends.map(({ text, saved }) => ({ text, saved }))).toEqual([{ text: 'Check prod’s DNS next', saved: 'dns' }])
    await until(() => send.mock.calls.length === 1, 'the session is asked to send it')
    const turnId = first.pendingSends[0]!.id
    expect(send.mock.calls[0]).toEqual(['dns', turnId, 'message'])
    expect(turnId).not.toBe('dns')

    answer.reject(new Error('The session cannot take it.'))
    await until(() => first.pendingSends[0]?.error !== null, 'the send fails')
    expect(listed(first)).toEqual(['Then the IPv6'])
    expect(first.pendingSends.map(({ id, error }) => ({ id, error }))).toEqual([{ id: turnId, error: 'The session cannot take it.' }])
  } finally {
    send.mockRestore()
    restore()
  }
})

// Edit on a message from the list that was not delivered edits it as the
// list's Edit does: it goes into the composer and is neither on its way nor
// listed; Delete takes it from the list.
test('Edit on a saved message not delivered puts it in the composer, and Delete takes it from the list', async () => {
  const { store, first } = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next'), savedMessage('ipv6', 'Then the IPv6')])
  const restore = await withStubModel(first)
  const send = spyOn(ConversationRuntime.prototype, 'sendSaved').mockRejectedValue(new Error('The session cannot take it.'))
  try {
    store.sendSaved(first, 'dns', 'send')
    await until(() => first.pendingSends[0]?.error != null, 'the send fails')
    store.editSend(first, first.pendingSends[0]!.id)
    await until(() => first.draft === 'Check prod’s DNS next', 'the message is in the composer')
    expect([listed(first), first.pendingSends]).toEqual([['Then the IPv6'], []])
    store.sendSaved(first, 'ipv6', 'send')
    await until(() => first.pendingSends[0]?.error != null, 'the send fails')
    store.deleteSend(first, first.pendingSends[0]!.id)
    expect([listed(first), first.pendingSends]).toEqual([[], []])
    await until(() => (serverSaved.get(FIRST) ?? []).length === 0, 'the backend lists neither')
  } finally {
    send.mockRestore()
    restore()
  }
})

test('a saved message the session took leaves the list for good once read', async () => {
  const { store, first } = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next')])
  const restore = await withStubModel(first)
  const send = spyOn(ConversationRuntime.prototype, 'sendSaved').mockImplementation(async () => {
    // The backend takes it off its list as the session takes it.
    serverSaved.set(FIRST, [])
    records[0]!.savedRevision += 1
    return true
  })
  try {
    store.sendSaved(first, 'dns', 'send')
    await until(() => first.pendingSends.length === 0, 'the session holds it')
    expect(listed(first)).toEqual([])
    await readAgain(first)
    expect([listed(first), first.record.pending()]).toEqual([[], []])
  } finally {
    send.mockRestore()
    restore()
  }
})

// A Send pressed on the row of a save still on its way.
test('a message sent from the list while its save is on its way goes once the save is answered, not before', async () => {
  const signOut = signIn()
  const { store, first } = await openWithSaved([])
  const restore = await withStubModel(first)
  const held = holdWrites()
  const send = spyOn(ConversationRuntime.prototype, 'sendSaved').mockResolvedValue(true)
  try {
    first.draft = 'Check prod’s DNS next'
    const saving = store.saveForLater(first)
    await until(() => held.waiting.length === 1, 'the save reaches the backend')
    const id = first.record.shown.value!.saved[0]!.id
    store.sendSaved(first, id, 'send')
    expect([listed(first), first.pendingSends.map(({ text }) => text)]).toEqual([[], ['Check prod’s DNS next']])
    await settle()
    expect(send).not.toHaveBeenCalled()
    await held.waiting[0]!.pass()
    await saving
    await until(() => send.mock.calls.length === 1, 'the send goes once the save is answered')
    expect(send.mock.calls[0]![0]).toBe(id)
    await until(() => first.pendingSends.length === 0, 'the session holds it')
  } finally {
    send.mockRestore()
    held.restore()
    restore()
    signOut()
  }
})

// The reviewer's second finding: a reload during a saved send. The page
// did not hear the backend's answer, so the message is still on its way and
// goes again by itself, from the list, as the same turn.
test('a saved send interrupted by a reload goes again from the list as the same turn by itself, never as a new message', async () => {
  let signOut = signIn()
  const kept = new Map<string, draftStorage.SavedDraft>()
  const writes = spyOn(draftStorage, 'writeDraft').mockImplementation(async (_user, id, draft) => {
    kept.set(id, structuredClone(draft))
  })
  let opened = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next')])
  let restore = await withStubModel(opened.first)
  const never = spyOn(ConversationRuntime.prototype, 'sendSaved').mockImplementation(() => new Promise(() => {}))
  try {
    opened.store.sendSaved(opened.first, 'dns', 'send')
    await until(() => never.mock.calls.length === 1, 'the session is asked to send it')
    const turnId = opened.first.pendingSends[0]!.id
    expect(kept.get(FIRST)?.pendingSends).toMatchObject([{ id: turnId, saved: 'dns' }])
    never.mockRestore()
    restore()
    signOut()
    opened.store.stopAll()
    useProduct().stop()
    disposePinia(pinia)

    // The next page: the backend still lists the message, which is on its way here.
    const submit = spyOn(ConversationRuntime.prototype, 'submit').mockResolvedValue()
    const again = spyOn(ConversationRuntime.prototype, 'sendSaved').mockResolvedValue(true)
    try {
      pinia = createPinia()
      setActivePinia(pinia)
      signOut = signIn(kept.get(FIRST)!)
      useConversations()
      await connect()
      opened = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next')])
      restore = await withStubModel(opened.first)
      const { first } = opened
      expect(listed(first)).toEqual([])
      await until(() => first.pendingSends.length === 0, 'the message goes again by itself')
      expect(again.mock.calls).toEqual([['dns', turnId, 'message']])
      expect(submit).not.toHaveBeenCalled()
    } finally {
      again.mockRestore()
      submit.mockRestore()
    }
  } finally {
    restore()
    writes.mockRestore()
    signOut()
  }
})

// The reviewer's third finding: Undo after a deletion must not bring back a used id.
test('a deleted saved message leaves at once with its Undo toast, and Undo saves it again under a new id; a refused deletion brings it back and the toast goes', async () => {
  const { store, first } = await openWithSaved([savedMessage('dns', 'Check prod’s DNS next')])
  toasts.splice(0)
  const held = holdWrites()
  try {
    const refused = store.deleteSaved(first, 'dns')
    await until(() => held.waiting.length === 1, 'the delete reaches the backend')
    expect(listed(first)).toEqual([])
    expect(toasts.map((toast) => toast.title)).toEqual(['Saved Message Deleted'])
    held.waiting[0]!.answer(refusal('internal_error', 500))
    await refused
    expect(listed(first)).toEqual(['Check prod’s DNS next'])
    expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Delete the Message'])
    toasts.splice(0)

    const deleting = store.deleteSaved(first, 'dns')
    await until(() => held.waiting.length === 2, 'the second delete reaches the backend')
    toasts[0]!.action!.run()
    expect(listed(first)).toEqual(['Check prod’s DNS next'])
    await held.waiting[1]!.pass()
    await deleting
    await until(() => held.waiting.length === 3, 'the Undo reaches the backend after the delete')
    const undo = held.waiting[2]!.body as { id: string; text: string }
    expect(undo.text).toBe('Check prod’s DNS next')
    expect(undo.id).not.toBe('dns')
  } finally {
    held.restore()
  }
})

test('a change of the model shows at once and a refusal gives the stored settings back', async () => {
  const { store, first } = await openWithSaved([])
  records[0]!.model = { providerId: 'stub', modelId: 'stub', thinkingEffort: null, serviceTierId: null }
  await changed(FIRST)
  expect(first.model.serviceTierId).toBeNull()
  const held = holdWrites()
  try {
    const changing = store.changeModel(first, { serviceTierId: 'priority' })
    await until(() => held.waiting.length === 1, 'the change reaches the backend')
    expect(first.model.serviceTierId).toBe('priority')
    expect(useProduct().snapshot!.conversations[0]!.model?.serviceTierId).toBe('priority')
    held.waiting[0]!.answer(refusal('internal_error', 500))
    await changing
    expect(first.model.serviceTierId).toBeNull()
  } finally {
    held.restore()
  }
})

test('moving a conversation into a project on its Host shows the project at once; a refusal moves it back', async () => {
  const { store, first } = await openWithSaved([])
  const product = useProduct()
  const cloud = productState({ devices: [{
    id: 'cloud', name: 'Cloud', kind: 'managed', platform: 'linux', claimedAt: '2026-09-10T00:00:00.000Z', lastSeenAt: null,
    state: 'online', home: null, installed: [], startCommand: null, autostartUnavailable: null, os: null, runnerVersion: null, downloadPercent: null, mark: null, route: 'automatic',
  }] }).devices
  product.read = {
    ...product.read!,
    devices: cloud,
    workspaces: [{ id: 'notes', deviceId: 'cloud', path: '/notes', name: 'notes', createdAt: '2026-09-10T00:00:00.000Z' }],
  }
  const held = holdWrites()
  try {
    // The move is on its way, and the page answers that it moved.
    expect(await store.switchTarget(FIRST, { kind: 'workspace', workspaceId: 'notes' })).toBe(true)
    await until(() => held.waiting.length === 1, 'the move reaches the backend')
    expect([first.target, first.projectId, first.cwd]).toEqual([{ kind: 'workspace', workspaceId: 'notes' }, 'notes', '/notes'])
    held.waiting[0]!.answer(refusal('internal_error', 500))
    await until(() => first.projectId === null, 'the refusal moves it back')
    expect(first.target).toEqual({ kind: 'cloud' })
  } finally {
    held.restore()
  }
})
