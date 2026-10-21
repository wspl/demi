import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { schemaRefusal } from '@demicodes/protocol'
import { errorMessage, reportError } from '@demicodes/web-ui/infra/errors'
import { Slot, type SlotAnswer } from '@demicodes/web-ui/infra/slot'
import { browserOnline, openSocket, pingOn, reconnectNow, waitToReconnect, waitWhileRestarting, watchSilence, type ReconnectWait, type SilenceWatch } from '@demicodes/web-ui/transport/liveness'
import { connectionProblem } from '@demicodes/web-ui/transport/connection'
import { t } from './locales/text'
import { apiRequest, apiUrl, notifySessionEnded, readResponse, waitForBackendWith } from '../api/client'
import {
  productStateSchema,
  syncEventSchema,
  vendorCatalogSchema,
  type ConversationSummary,
  type PageShowing,
  type ProductState,
  type SyncEvent,
  type VendorCatalog,
} from '../api/generated/web-api'

/** The close code of a channel whose session ended. */
const SESSION_ENDED = 4002
/** The close of a channel whose backend shuts down, and will be back. */
const BACKEND_CLOSING = 1001
/** The close the web browser reports for a socket that ended without a close frame, as one that could not connect. */
const ABNORMAL_CLOSURE = 1006

/** A message that carries one part of the product state, as a write's answer can. */
export type PartEvent = Exclude<SyncEvent, { type: 'snapshot' | 'heartbeat' | 'pong' }>

/** A part of the product state, which a message or a write's answer replaces whole. */
export type Part = `conversation:${string}` | `plugin:${string}` | Exclude<PartEvent['type'], 'conversation' | 'conversation_deleted' | 'plugin'>

/**
 * What a write's answer says of a part: the part's value, or how to make it
 * of the state the page last read, as for an answer that carries only one
 * item of a list.
 */
export type Answer = PartEvent | ((read: ProductState) => PartEvent)

/**
 * A change of this page's own that its write has not landed yet: the page shows
 * it over the state it last read from the moment the user made it.
 */
export interface ProductChange {
  /** Its write goes out now; a value the channel brings of its part from here may be newer than the write's. */
  send(): void
  /**
   * The write was answered: `answer`, or, when the answer carries nothing,
   * the change itself, applies as `answered` applies it, and the change
   * leaves. When the channel brought its part since the write went out, the
   * change stays until that part's next value from the channel.
   */
  land(answer?: Answer): void
  /**
   * The write failed or the backend refused it: the change leaves at once,
   * and `answer`, what the backend has, applies as `answered` applies it.
   */
  drop(answer?: Answer): void
}

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

/** The value `state` holds of `part`, as a message that carries it says it. */
function partEvent(state: ProductState, part: Part): PartEvent {
  switch (part) {
    case 'conversation_order':
      return { type: 'conversation_order', ids: state.conversations.map((conversation) => conversation.id) }
    case 'preferences':
      return { type: 'preferences', preferences: state.preferences }
    case 'user':
      return { type: 'user', user: state.user }
    case 'workspaces':
      return { type: 'workspaces', workspaces: state.workspaces }
    case 'devices':
      return { type: 'devices', devices: state.devices }
    case 'plugins':
      return { type: 'plugins', plugins: state.plugins }
    case 'providers':
      return { type: 'providers', providers: state.providers }
    case 'cloud':
      return { type: 'cloud', cloud: state.cloud }
    case 'logins':
      return { type: 'logins', logins: state.logins }
    case 'subagents':
      return { type: 'subagents', subagents: state.subagents }
    case 'instructions':
      return { type: 'instructions', instructions: state.instructions }
    case 'credentials':
      return { type: 'credentials', credentials: state.credentials }
    case 'passkeys':
      return { type: 'passkeys', ...state.passkeys }
    case 'sessions':
      return { type: 'sessions', sessions: state.sessions }
    case 'users':
      return { type: 'users', users: state.users ?? [] }
    case 'automations':
      return { type: 'automations', automations: state.automations, automationsUnread: state.automationsUnread }
    case 'port_forwards':
      return { type: 'port_forwards', portForwards: state.portForwards }
    case 'exposed_ports':
      return { type: 'exposed_ports', exposedPorts: state.exposedPorts }
    case 'models':
      return { type: 'models', models: state.models }
  }
  if (part.startsWith('plugin:')) {
    const plugin = part.slice('plugin:'.length)
    return { type: 'plugin', plugin, state: state.pluginStates[plugin] ?? null }
  }
  const id = part.slice('conversation:'.length)
  const conversation = state.conversations.find((candidate) => candidate.id === id)
  return conversation ? { type: 'conversation', conversation } : { type: 'conversation_deleted', id }
}

/** `state` with the part `event` carries replaced. */
export function withPart(state: ProductState, event: PartEvent): ProductState {
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
    case 'logins':
      return { ...state, logins: event.logins }
    case 'subagents':
      return { ...state, subagents: event.subagents }
    case 'instructions':
      return { ...state, instructions: event.instructions }
    case 'credentials':
      return { ...state, credentials: event.credentials }
    case 'passkeys':
      return { ...state, passkeys: { passkeys: event.passkeys, userHandle: event.userHandle } }
    case 'sessions':
      return { ...state, sessions: event.sessions }
    case 'users':
      return { ...state, users: event.users }
    case 'automations':
      return { ...state, automations: event.automations, automationsUnread: event.automationsUnread }
    case 'port_forwards':
      return { ...state, portForwards: event.portForwards }
    case 'exposed_ports':
      return { ...state, exposedPorts: event.exposedPorts }
    case 'models':
      return { ...state, models: event.models }
  }
}

/**
 * The build a snapshot names, read before the rest of it: a snapshot of
 * another build, whose other parts this page may not read, still says which
 * build to load.
 */
const snapshotBuildSchema = z.object({
  type: z.literal('snapshot'),
  state: productStateSchema.pick({ webBuild: true }),
})

/** This page's web app build, which `vite build` writes in; none in development, where Vite serves the sources. */
function pageBuild(): string | null {
  return import.meta.env.DEMI_WEB_BUILD ?? null
}

/** Where this browser keeps the vendor catalog, with the build it was read by. */
const VENDORS_KEY = 'demi.vendors'
const keptVendorsSchema = z.object({ build: z.string(), catalog: vendorCatalogSchema })

/**
 * The vendor catalog this browser read with this build, which the backend
 * that serves it answers alike for every user; none in development, where
 * the backend changes without a build.
 */
function keptVendors(): VendorCatalog | null {
  const build = pageBuild()
  if (build === null) {
    return null
  }
  try {
    const kept = keptVendorsSchema.safeParse(JSON.parse(localStorage.getItem(VENDORS_KEY) ?? 'null'))
    return kept.success && kept.data.build === build ? kept.data.catalog : null
  } catch {
    // The browser's storage is optional: without it the page reads the
    // catalog as it starts.
    return null
  }
}

function keepVendors(catalog: VendorCatalog): void {
  const build = pageBuild()
  if (build === null) {
    return
  }
  try {
    localStorage.setItem(VENDORS_KEY, JSON.stringify({ build, catalog }))
  } catch {
    // A full or absent storage costs one read on the next start.
  }
}

function parse(data: unknown): unknown {
  try {
    return JSON.parse(String(data))
  } catch {
    return null
  }
}

/** What a write's answer says of its part, as the slot takes it. */
function answerOf(answer: Answer): SlotAnswer<ProductState> & { value: (read: ProductState) => ProductState } {
  return { value: (state) => withPart(state, typeof answer === 'function' ? answer(state) : answer) }
}

/**
 * The product state this page shows around its conversations: the one module
 * that follows it, through the synchronization channel. The channel's snapshot
 * is the whole copy, and each later message replaces one part of it; each state
 * of the page follows the copy by its own rule. The copy is a slot
 * (`web-ui/infra/slot`) read in parts. Nothing asks for this state on a timer
 * or after a write: a write's answer goes through `answered`, or lands the
 * change the write made (`change`).
 */
export const useProduct = defineStore('product', () => {
  /**
   * The state as the channel and the writes' answers last left it, with this
   * page's changes not landed yet, in the order the user made them, on top.
   */
  const slot = new Slot<ProductState>()
  const read = slot.read
  /** The account a channel of an account still to choose its password sent first, before any state; null otherwise. */
  const firstUser = ref<ProductState['user'] | null>(null)
  /** What the page shows: the state it last read with its own changes not landed yet applied on top. */
  const snapshot = slot.shown
  const load = ref<'loading' | 'ready' | 'failed'>('loading')
  /**
   * What the page can add, read once in the background as the page starts
   * unless this browser kept it for this build, so Settings › Providers
   * never waits for it.
   */
  const vendors = ref<VendorCatalog | null>(keptVendors())
  const vendorError = ref<string | null>(null)
  const vendorLoad = computed(() =>
    vendors.value !== null ? 'ready' : vendorError.value ? 'failed' : 'loading',
  )
  /**
   * Whether the page holds the catalog of the entries' models, which comes
   * with the product state and its owner announces anew
   * (`docs/decisions/0029-a-conversation-opens-whole.md`).
   */
  const catalogLoad = computed(() => snapshot.value === null ? 'loading' : 'ready')
  const activeConversationId = ref<string | null>(null)
  /**
   * The backend closed the channel because it shuts down, and has not
   * brought a snapshot since.
   */
  const restarting = ref(false)
  /** Consecutive connections that ended before their snapshot. */
  const failures = ref(0)
  /**
   * Why the page cannot reach the backend, which its connection banner says;
   * null while it can.
   */
  const connection = computed(() => connectionProblem({
    online: browserOnline.value,
    restarting: restarting.value,
    failedAttempts: failures.value,
    hasSnapshot: snapshot.value !== null,
  }))
  /**
   * The page has no state yet and could not reach the backend so far, as a
   * page loaded while Demi restarts: it says it connects where the app will
   * be.
   */
  const connecting = computed(() => snapshot.value === null && (failures.value > 0 || !browserOnline.value))
  /** The build of the web app the backend serves, as the last snapshot named it. */
  const servedBuild = ref<string | null>(null)
  /**
   * The build the backend serves when it is another than this page's, which
   * a reload loads; null otherwise.
   */
  const newBuild = computed(() => {
    const page = pageBuild()
    const served = servedBuild.value
    return page !== null && served !== null && served !== page ? served : null
  })
  const catalog = computed(() => snapshot.value?.models.providers ?? [])
  /**
   * The selection the small model's Automatic resolves to, which Settings ›
   * Agent shows beside it (`crates/backend-providers/src/llm/small_model.rs`);
   * null when the backend resolves none for the page, or before the state
   * is read.
   */
  const automaticSmallModel = computed(() => snapshot.value?.models.smallModel ?? null)
  let vendorRequest: Promise<void> | null = null
  /** Set while the page is signed in and follows the state. */
  let controller: AbortController | null = null
  /** Undoes the requests' wait for the backend, which `start` sets. */
  let stopWaiting: (() => void) | null = null
  let socket: WebSocket | null = null
  /** The wait before the channel connects again, while it is closed. */
  let retry: ReconnectWait | null = null
  /** The open channel's silence watch. */
  let silence: SilenceWatch | null = null
  /**
   * The conversation this page shows, focused and visible, which the channel
   * tells the backend so this session's browser is not notified of it; each
   * new connection tells it again, as the backend forgets it with the last.
   */
  let showing: string | null = null
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
    const channel = openSocket(url, 'synchronization channel')
    socket = channel
    let opened = false
    channel.onopen = () => {
      if (socket !== channel) {
        return
      }
      opened = true
      // A channel that brings nothing, heartbeats included, for too long is broken.
      silence = watchSilence('synchronization channel', replace, pingOn(channel))
      if (showing !== null) {
        sendShowing(channel)
      }
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
      reportError(t('failures.readSync'), schemaRefusal(parsed.error))
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
      // Before the first snapshot, only the backend's own failure, a close
      // it sent, fails the regions; a channel that could not reach it, or
      // that the backend closed to restart, keeps them loading while it
      // connects again.
      if (!snapshot.value && close.code !== ABNORMAL_CLOSURE && close.code !== BACKEND_CLOSING) {
        load.value = 'failed'
      }
      // The web browser does not say why an upgrade failed: an ended session
      // answers this with 401, which ends it on the page as any 401 does.
      // It does not wait for the backend: the channel is how the page learns
      // that it reaches the backend, and a failure only means it does not yet.
      if (!opened && controller) {
        void apiRequest('/auth/me', { signal: controller.signal, waits: false }).catch(() => {})
      }
      failures.value += 1
      scheduleRetry()
    }
  }

  function sendShowing(channel: WebSocket): void {
    channel.send(JSON.stringify({ showing } satisfies PageShowing))
  }

  /** Says the page shows `conversation`, focused and visible, or none. */
  function show(conversation: string | null): void {
    if (conversation === showing) {
      return
    }
    showing = conversation
    if (socket?.readyState === WebSocket.OPEN) {
      sendShowing(socket)
    }
  }

  function receive(event: SyncEvent): void {
    if (event.type === 'heartbeat') {
      return
    }
    if (event.type === 'pong') {
      silence?.ponged(event.id)
      return
    }
    if (event.type === 'snapshot') {
      // The backend is back after it was away: each other socket of the page
      // that waits to connect connects at once, as at the page's return, so
      // nothing still says it connects once the banner goes.
      const returned = failures.value > 0 || restarting.value
      failures.value = 0
      restarting.value = false
      if (returned) {
        reconnectNow()
      }
      // The snapshot is the next value of every part.
      slot.replace(event.state)
      load.value = 'ready'
      return
    }
    if (!slot.receive(partOf(event), (state) => withPart(state, event)) && event.type === 'user') {
      // The channel of an account still to choose its password carries the
      // account alone, until the choice, on this page or another.
      firstUser.value = event.user
    }
  }

  /** Where the channel stands as a write is sent, which its answer is measured against. */
  function sent(): number {
    return slot.now()
  }

  /**
   * Applies the part a write's answer carries, as the write left it, unless
   * the channel brought a value of that part since the write was sent `at`:
   * that value may be newer, and if it is older, the value the write caused
   * is still to come on the channel. Answers whether it applied.
   */
  function answered(at: number, answer: Answer): boolean {
    const state = read.value
    if (!state) {
      return false
    }
    const event = typeof answer === 'function' ? answer(state) : answer
    return slot.answered([partOf(event)], at, answerOf(event))
  }

  /**
   * Shows `apply`, a change the user made of `part`, over the state at once,
   * until its write lands. The caller sends the write, in order with its other
   * writes, and says how it ended through the change it gets.
   */
  function change(part: Part, apply: (state: ProductState) => ProductState): ProductChange {
    const entry = slot.change(apply, { parts: [part] })
    return {
      send: () => entry.send(),
      land: (answer = (state) => partEvent(apply(state), part)) => void entry.land(answerOf(answer)),
      drop: (answer) => entry.drop(answer && answerOf(answer)),
    }
  }

  /**
   * Resolves once the copy holds what `check` looks for, such as an entry a
   * write just made, which the channel brings moments after it commits;
   * rejects when `signal` aborts first.
   */
  function until(check: (state: ProductState) => boolean, signal: AbortSignal): Promise<void> {
    return holds(() => snapshot.value !== null && check(snapshot.value), signal)
  }

  /** Resolves once `condition` holds, as the state it reads changes; rejects when `signal` aborts first. */
  function holds(condition: () => boolean, signal: AbortSignal): Promise<void> {
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
      if (condition()) {
        found()
        return
      }
      stop = watch(condition, (held) => {
        if (held) {
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
   * Asks the backend to read the entries' models again, as the user's
   * explicit refresh does; the new catalog comes on the channel, and an
   * entry's failure with its health.
   */
  async function reloadModels(): Promise<void> {
    await apiRequest('/models/refresh', { method: 'POST' })
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
        keepVendors(next)
      } catch (cause) {
        if (!current.signal.aborted) {
          vendorError.value =
            errorMessage(cause)
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
    const lifetime = new AbortController()
    controller = lifetime
    // Every request of the page waits for the backend while the channel
    // cannot reach it, until the page stops following the state.
    stopWaiting = waitForBackendWith((attempt, signal) => reachable(attempt, AbortSignal.any([signal, lifetime.signal])))
    connect()
    // Its failure is the Add Provider dialog's region state, with its own Retry.
    void loadVendors().catch(() => {})
  }

  /**
   * Shows `state`, the account's copy kept for opening offline, until the
   * channel's first snapshot replaces it, as a later snapshot replaces the
   * copy. A page that holds a state already keeps it.
   */
  function showCopy(state: ProductState): void {
    if (read.value !== null) {
      return
    }
    slot.replace(state)
    load.value = 'ready'
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
    stopWaiting?.()
    stopWaiting = null
    clearRetry()
    dropChannel()?.close()
    failures.value = 0
    restarting.value = false
    servedBuild.value = null
    slot.clear()
    firstUser.value = null
    vendors.value = keptVendors()
    vendorRequest = null
    vendorError.value = null
    activeConversationId.value = null
    showing = null
    load.value = 'loading'
  }

  /** The account as the channel last told it, before the state arrived or with it. */
  const account = computed(() => snapshot.value?.user ?? firstUser.value)

  return {
    snapshot,
    account,
    read,
    load,
    connection,
    newBuild,
    catalog,
    automaticSmallModel,
    vendors,
    vendorLoad,
    catalogLoad,
    activeConversationId,
    show,
    sent,
    answered,
    change,
    until,
    connecting,
    reloadModels,
    loadVendors,
    start,
    showCopy,
    reconnect,
    stop,
  }
})
