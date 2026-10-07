import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { browserOnline, openSocket, waitToReconnect, waitWhileRestarting, watchSilence, type ReconnectWait, type SilenceWatch } from '@demicodes/web-ui/transport/liveness'
import { connectionProblem } from '@demicodes/web-ui/transport/connection'
import { apiRequest, apiUrl, notifySessionEnded, readResponse, unreachable } from '../api/client'
import {
  modelCatalogSchema,
  productStateSchema,
  syncEventSchema,
  vendorCatalogSchema,
  type CatalogProvider,
  type ConversationSummary,
  type ProductState,
  type SyncEvent,
  type VendorCatalog,
} from '../api/generated/web-api'

/** The close code of a channel whose session ended (`web-api.md` § Page synchronization). */
const SESSION_ENDED = 4002
/** The close of a channel whose backend shuts down, and will be back (`web-api.md` § Page synchronization). */
const BACKEND_CLOSING = 1001

/** A message that carries one part of the product state, as a write's answer can. */
export type PartEvent = Exclude<SyncEvent, { type: 'snapshot' | 'heartbeat' }>

/** A part of the product state, which a message or a write's answer replaces whole. */
type Part = `conversation:${string}` | `plugin:${string}` | Exclude<PartEvent['type'], 'conversation' | 'conversation_deleted' | 'plugin'>

function partOf(event: PartEvent): Part {
  switch (event.type) {
    case 'conversation':
      return `conversation:${event.conversation.id}`
    case 'conversation_deleted':
      return `conversation:${event.id}`
    case 'plugin':
      return `plugin:${event.plugin}`
    default:
      return event.type
  }
}

/** The conversations with `summary` in its place, or, new, at the front of the active or the end of the archived ones. */
function withSummary(conversations: ConversationSummary[], summary: ConversationSummary): ConversationSummary[] {
  const index = conversations.findIndex((conversation) => conversation.id === summary.id)
  if (index >= 0) {
    return conversations.toSpliced(index, 1, summary)
  }
  return summary.archived ? [...conversations, summary] : [summary, ...conversations]
}

/**
 * The conversations in the order `ids` gives. One the order does not name
 * yet was created after the order was read and keeps the front; its own
 * order comes next.
 */
function inOrder(conversations: ConversationSummary[], ids: string[]): ConversationSummary[] {
  const place = new Map(ids.map((id, index) => [id, index]))
  const unplaced = conversations.filter((conversation) => !place.has(conversation.id))
  const placed = conversations
    .filter((conversation) => place.has(conversation.id))
    .sort((a, b) => place.get(a.id)! - place.get(b.id)!)
  return [...unplaced, ...placed]
}

/**
 * The plugin states without those of the plugins the user turned off, which
 * leave the pages at once; a plugin turned on sends its state afresh.
 */
function withoutOff(
  states: ProductState['pluginStates'],
  plugins: ProductState['plugins'],
): ProductState['pluginStates'] {
  const off = new Set(plugins.filter((plugin) => !plugin.enabled).map((plugin) => plugin.id))
  return Object.fromEntries(Object.entries(states).filter(([id]) => !off.has(id)))
}

/** `state` with the part `event` carries replaced. */
function withPart(state: ProductState, event: PartEvent): ProductState {
  switch (event.type) {
    case 'conversation':
      return { ...state, conversations: withSummary(state.conversations, event.conversation) }
    case 'conversation_deleted':
      return { ...state, conversations: state.conversations.filter((conversation) => conversation.id !== event.id) }
    case 'conversation_order':
      return { ...state, conversations: inOrder(state.conversations, event.ids) }
    case 'preferences':
      return { ...state, preferences: event.preferences }
    case 'user':
      return { ...state, user: event.user }
    case 'workspaces':
      return { ...state, workspaces: event.workspaces }
    case 'devices':
      return { ...state, devices: event.devices }
    case 'plugins':
      return { ...state, plugins: event.plugins, pluginStates: withoutOff(state.pluginStates, event.plugins) }
    case 'plugin':
      return { ...state, pluginStates: { ...state.pluginStates, [event.plugin]: event.state } }
    case 'providers':
      return { ...state, providers: event.providers }
    case 'cloud':
      return { ...state, cloud: event.cloud }
    case 'subagents':
      return { ...state, subagents: event.subagents }
  }
}

/**
 * The build a snapshot names, read before the rest of it: a snapshot of
 * another build, whose other parts this page may not read, still says which
 * build to load (`web-application.md` § A page of another build).
 */
const snapshotBuildSchema = z.object({
  type: z.literal('snapshot'),
  state: productStateSchema.pick({ webBuild: true }),
})

/** This page's web app build, which `vite build` writes in; none in development, where Vite serves the sources. */
function pageBuild(): string | null {
  return import.meta.env.DEMI_WEB_BUILD ?? null
}

function parse(data: unknown): unknown {
  try {
    return JSON.parse(String(data))
  } catch {
    return null
  }
}

/**
 * The product state this page shows around its conversations
 * (`web-application.md` § Page synchronization): the one module that follows
 * it, through the synchronization channel. The channel's snapshot is the
 * whole copy, and each later message replaces one part of it; each state of
 * the page follows the copy by its own rule. Nothing asks for this state on
 * a timer or after a write: a write's answer goes through `answered`.
 */
export const useProduct = defineStore('product', () => {
  const snapshot = ref<ProductState | null>(null)
  const load = ref<'loading' | 'ready' | 'failed'>('loading')
  const modelSnapshot = ref<{ key: string; providers: CatalogProvider[]; checkedAt: number } | null>(null)
  const vendors = ref<VendorCatalog | null>(null)
  const modelError = ref<{ key: string; retryAt: number } | null>(null)
  const vendorError = ref<string | null>(null)
  const vendorLoad = computed(() =>
    vendors.value !== null ? 'ready' : vendorError.value ? 'failed' : 'loading',
  )
  const catalogLoad = computed(() => modelSnapshot.value?.key === catalogKey.value
    ? 'ready' : modelError.value?.key === catalogKey.value ? 'failed' : 'loading')
  const activeConversationId = ref<string | null>(null)
  /**
   * The backend closed the channel because it shuts down, and has not
   * brought a snapshot since (`web-application.md` § A page of another build).
   */
  const restarting = ref(false)
  /** Consecutive connections that ended before their snapshot. */
  const failures = ref(0)
  /**
   * Why the page cannot reach the backend, which its connection banner says;
   * null while it can (`web-application.md` § A page of another build).
   */
  const connection = computed(() => connectionProblem({
    online: browserOnline.value,
    restarting: restarting.value,
    failedAttempts: failures.value,
    hasSnapshot: snapshot.value !== null,
  }))
  /** The build of the web app the backend serves, as the last snapshot named it. */
  const servedBuild = ref<string | null>(null)
  /**
   * The build the backend serves when it is another than this page's, which
   * a reload loads; null otherwise (`web-application.md` § A page of another
   * build).
   */
  const newBuild = computed(() => {
    const page = pageBuild()
    const served = servedBuild.value
    return page !== null && served !== null && served !== page ? served : null
  })
  const catalogKey = computed(() => JSON.stringify((snapshot.value?.providers ?? []).map(provider => {
    const { details, ...config } = provider
    return { ...config, account: details.type === 'read' ? details.active : null }
  })))
  const catalog = computed(() => modelSnapshot.value?.key === catalogKey.value
    ? modelSnapshot.value.providers : [])
  let modelRequest: { key: string; promise: Promise<void>; controller: AbortController } | null = null
  let vendorRequest: Promise<void> | null = null
  /** Set while the page is signed in and follows the state. */
  let controller: AbortController | null = null
  let socket: WebSocket | null = null
  /** The wait before the channel connects again, while it is closed. */
  let retry: ReconnectWait | null = null
  /** The open channel's silence watch (`web-application.md` § Liveness and reconnection). */
  let silence: SilenceWatch | null = null
  /** Messages received, which numbers them. */
  let received = 0
  /** The number of the last snapshot, and of each part's last value. */
  let snapshotAt = 0
  const partAt = new Map<Part, number>()

  function clearRetry(): void {
    retry?.cancel()
    retry = null
  }

  /** Forgets the open channel, which then closes without telling this module. */
  function dropChannel(): WebSocket | null {
    const channel = socket
    socket = null
    silence?.stop()
    silence = null
    return channel
  }

  /** Closes the channel and connects again after the wait. */
  function replace(): void {
    dropChannel()?.close()
    failures.value += 1
    scheduleRetry()
  }

  /**
   * Connects again after the wait while the backend restarts, or else after
   * the wait for this many failures, or at the page's return.
   */
  function scheduleRetry(): void {
    if (!controller) {
      return
    }
    clearRetry()
    const again = () => {
      retry = null
      connect()
    }
    retry = restarting.value ? waitWhileRestarting(again) : waitToReconnect(failures.value, again)
  }

  function connect(): void {
    if (!controller || socket) {
      return
    }
    clearRetry()
    const url = new URL(apiUrl('/sync'), window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    const channel = openSocket(url)
    socket = channel
    let opened = false
    channel.onopen = () => {
      if (socket !== channel) {
        return
      }
      opened = true
      // A channel that brings nothing, heartbeats included, for too long is broken.
      silence = watchSilence(replace)
    }
    channel.onmessage = (message) => {
      if (socket !== channel) {
        return
      }
      silence?.heard()
      const data = parse(message.data)
      const build = snapshotBuildSchema.safeParse(data)
      if (build.success) {
        servedBuild.value = build.data.state.webBuild ?? null
      }
      const parsed = syncEventSchema.safeParse(data)
      if (parsed.success) {
        receive(parsed.data)
        return
      }
      if (newBuild.value !== null) {
        // Another build's contract may say what this page cannot read; the
        // page loads that build, and the backend is back once it named it.
        if (build.success) {
          failures.value = 0
          restarting.value = false
        }
        return
      }
      // A message outside the contract changes nothing; a new connection
      // starts again from a snapshot. The console says what did not read.
      reportError('Could not read a message of the synchronization channel.', z.prettifyError(parsed.error))
      replace()
    }
    channel.onclose = (close) => {
      if (socket !== channel) {
        return
      }
      dropChannel()
      if (close.code === SESSION_ENDED) {
        notifySessionEnded()
        return
      }
      if (close.code === BACKEND_CLOSING) {
        restarting.value = true
      }
      if (!snapshot.value) {
        load.value = 'failed'
      }
      // The web browser does not say why an upgrade failed: an ended session
      // answers this with 401, which ends it on the page as any 401 does.
      if (!opened && controller) {
        void apiRequest('/auth/me', { signal: controller.signal }).catch(() => {})
      }
      failures.value += 1
      scheduleRetry()
    }
  }

  function receive(event: SyncEvent): void {
    received += 1
    if (event.type === 'heartbeat') {
      return
    }
    if (event.type === 'snapshot') {
      snapshotAt = received
      failures.value = 0
      restarting.value = false
      snapshot.value = event.state
      load.value = 'ready'
      // Model discovery never holds the page; it records its own failure.
      void loadModels().catch(() => {})
      return
    }
    partAt.set(partOf(event), received)
    if (snapshot.value) {
      snapshot.value = withPart(snapshot.value, event)
    }
    if (event.type === 'providers') {
      providersChanged()
    }
  }

  /**
   * The channel brought the providers part: the catalog is loaded again,
   * whatever changed, an API key no view shows included (`models.md`
   * § Catalog cache). A load on its way may predate the change, so it gives
   * way; the catalog shown stays until the new one arrives.
   */
  function providersChanged(): void {
    modelRequest?.controller.abort()
    modelRequest = null
    modelError.value = null
    if (modelSnapshot.value) {
      modelSnapshot.value = { ...modelSnapshot.value, checkedAt: 0 }
    }
    void loadModels().catch(() => {})
  }

  /** Where the channel stands as a write is sent, which its answer is measured against. */
  function sent(): number {
    return received
  }

  /**
   * Applies the part a write's answer carries, as the write left it, unless
   * the channel brought a value of that part since the write was sent `at`:
   * that value may be newer, and if it is older, the value the write caused
   * is still to come on the channel. Answers whether it applied.
   */
  function answered(at: number, event: PartEvent): boolean {
    if (!snapshot.value || Math.max(snapshotAt, partAt.get(partOf(event)) ?? 0) > at) {
      return false
    }
    snapshot.value = withPart(snapshot.value, event)
    return true
  }

  /**
   * Resolves once the copy holds what `check` looks for, such as an entry a
   * write just made, which the channel brings moments after it commits;
   * rejects when `signal` aborts first.
   */
  function until(check: (state: ProductState) => boolean, signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      if (signal.aborted) {
        reject(signal.reason)
        return
      }
      let stop: (() => void) | null = null
      const aborted = () => {
        stop?.()
        reject(signal.reason)
      }
      const found = () => {
        stop?.()
        signal.removeEventListener('abort', aborted)
        resolve()
      }
      if (snapshot.value && check(snapshot.value)) {
        found()
        return
      }
      stop = watch(snapshot, (state) => {
        if (state && check(state)) {
          found()
        }
      })
      signal.addEventListener('abort', aborted, { once: true })
    })
  }

  /**
   * Resolves once the backend is worth asking again after `attempt` requests
   * that could not reach it: when the connection banner goes, while it shows;
   * otherwise after the page's reconnect wait for that many failures, cut
   * short when the page returns. Rejects when `signal` aborts first.
   */
  function reachable(attempt: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      if (signal.aborted) {
        reject(signal.reason)
        return
      }
      let wait: ReconnectWait | null = null
      let stopWatching: (() => void) | null = null
      const release = () => {
        wait?.cancel()
        stopWatching?.()
        signal.removeEventListener('abort', aborted)
      }
      const aborted = () => {
        release()
        reject(signal.reason)
      }
      const ready = () => {
        release()
        resolve()
      }
      signal.addEventListener('abort', aborted, { once: true })
      if (connection.value === null) {
        wait = waitToReconnect(attempt, ready)
        return
      }
      stopWatching = watch(connection, (problem) => {
        if (problem === null) {
          ready()
        }
      })
    })
  }

  /**
   * Sends with `send` until the backend answers it (`web-application.md`
   * § A page of another build): a try that could not reach the backend, as
   * `waits` judges (by default `unreachable`), is tried again once the
   * backend is worth asking again, so the connection banner says what
   * happens and the caller never sees a failure about the connection. Any
   * other failure rejects as it was, and so does every failure while the
   * page follows no state, as before sign-in. Rejects with the abort's
   * reason, without trying again, once `signal` aborts or the page stops
   * following the state, as a sign-out does.
   */
  async function untilReached<T>(
    send: () => Promise<T>,
    signal?: AbortSignal,
    waits: (error: unknown) => boolean = unreachable,
  ): Promise<T> {
    for (let attempt = 1; ; attempt += 1) {
      try {
        return await send()
      } catch (error) {
        if (signal?.aborted) {
          throw signal.reason
        }
        const lifetime = controller?.signal
        if (!lifetime || !waits(error)) {
          throw error
        }
        await reachable(attempt, signal ? AbortSignal.any([signal, lifetime]) : lifetime)
      }
    }
  }

  function clearModels(): void {
    modelRequest?.controller.abort()
    modelRequest = null
    modelSnapshot.value = null
    modelError.value = null
  }

  async function loadModels(force = false): Promise<void> {
    const current = controller
    if (!current)
      return
    const key = catalogKey.value
    if (modelRequest?.key === key)
      return modelRequest.promise
    if (!force && modelSnapshot.value?.key === key && Date.now() - modelSnapshot.value.checkedAt < 60_000)
      return
    if (!force && modelError.value?.key === key && Date.now() < modelError.value.retryAt)
      return
    modelRequest?.controller.abort()
    const requestController = new AbortController()
    const signal = AbortSignal.any([current.signal, requestController.signal])
    modelError.value = null
    const request = (async () => {
      try {
        const response = await apiRequest(force ? '/models?refresh=true' : '/models', {
          signal,
        })
        const next = await readResponse(response, modelCatalogSchema)
        signal.throwIfAborted()
        if (key === catalogKey.value)
          modelSnapshot.value = { key, providers: next.providers, checkedAt: Date.now() }
      } catch (cause) {
        if (!signal.aborted && key === catalogKey.value) {
          modelError.value = {
            key,
            retryAt: Date.now() + 60_000,
          }
        }
        throw cause
      }
    })()
    modelRequest = { key, promise: request, controller: requestController }
    try {
      await request
    } finally {
      if (modelRequest?.promise === request)
        modelRequest = null
    }
  }

  /**
   * Loads the catalog again, asking the backend to refresh it, as the
   * user's explicit refresh does (`models.md` § Catalog cache); the catalog
   * records a failure itself.
   */
  async function reloadModels(): Promise<void> {
    clearModels()
    await loadModels(true).catch(() => {})
  }

  async function loadVendors(): Promise<void> {
    const current = controller
    if (!current || vendors.value !== null) {
      return
    }
    if (vendorRequest) {
      return vendorRequest
    }
    vendorError.value = null
    const request = (async () => {
      try {
        const response = await apiRequest('/providers/catalog', {
          signal: current.signal,
        })
        const next = await readResponse(response, vendorCatalogSchema)
        current.signal.throwIfAborted()
        vendors.value = next
      } catch (cause) {
        if (!current.signal.aborted) {
          vendorError.value =
            cause instanceof Error ? cause.message : String(cause)
        }
        throw cause
      }
    })()
    vendorRequest = request
    try {
      await request
    } finally {
      if (vendorRequest === request) {
        vendorRequest = null
      }
    }
  }

  /** Follows the state from now on, until `stop`. */
  function start(): void {
    if (controller) {
      return
    }
    controller = new AbortController()
    connect()
  }

  /** The user's Retry: connects at once when the channel is not open. */
  function reconnect(): void {
    if (!controller) {
      return
    }
    // The regions waiting for the first snapshot show it loading at once, a
    // channel already on its way included (`RegionStatus`).
    if (!snapshot.value) {
      load.value = 'loading'
    }
    if (socket) {
      return
    }
    connect()
  }

  function stop(): void {
    controller?.abort()
    controller = null
    clearRetry()
    dropChannel()?.close()
    failures.value = 0
    restarting.value = false
    servedBuild.value = null
    received = 0
    snapshotAt = 0
    partAt.clear()
    snapshot.value = null
    clearModels()
    vendors.value = null
    vendorRequest = null
    vendorError.value = null
    activeConversationId.value = null
    load.value = 'loading'
  }

  return {
    snapshot,
    load,
    connection,
    newBuild,
    catalog,
    vendors,
    vendorLoad,
    catalogLoad,
    activeConversationId,
    sent,
    answered,
    until,
    untilReached,
    loadModels,
    reloadModels,
    loadVendors,
    start,
    reconnect,
    stop,
  }
})
