import { computed, getCurrentScope, onScopeDispose, reactive, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { schemaRefusal } from '@demicodes/protocol'
import { errorMessage, reportError } from '@demicodes/web-ui/infra/errors'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { Slot, type SlotAnswer } from '@demicodes/web-ui/infra/slot'
import { browserOnline, openSocket, pingOn, reconnectNow, waitToReconnect, waitWhileRestarting, watchSilence, type ReconnectWait, type SilenceWatch } from '@demicodes/web-ui/transport/liveness'
import { backendUnreachable, connectionCause, connectionState, type ConnectionState } from '@demicodes/web-ui/transport/connection'
import { t } from './locales/text'
import { ApiError, apiRequest, apiUrl, logReadAnswered, logReadFailed, notifySessionEnded, readResponse, unreachable as backendNotReached, waitForBackendWith } from '../api/client'
import {
  productStateSchema,
  syncEventSchema,
  vendorCatalogSchema,
  type ConversationSummary,
  type PageMessage,
  type SyncEvent,
  type SyncFollow,
  type SyncFollowed,
  type VendorCatalog,
} from '../api/generated/web-api'
import { LIST_ROUTES, isListPart, pageStateSchema, type ListPart, type PageState, type Untagged } from './lists'

/** The state as the page holds it, which a message or a write's answer replaces by part. */
type ProductState = PageState

/** The close code of a channel whose session ended. */
const SESSION_ENDED = 4002
/** The close of a channel whose backend shuts down, and will be back. */
const BACKEND_CLOSING = 1001
/** The close the web browser reports for a socket that ended without a close frame, as one that could not connect. */
const ABNORMAL_CLOSURE = 1006

/** A message that carries one part of the product state, as a write's answer can. */
export type PartEvent = Untagged<Exclude<SyncEvent, { type: 'heartbeat' | 'pong' | 'synced' | 'stale' | 'automations_unread' }>>

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

/** A list the page holds, which a change of it presupposes: the view that changes it holds it. */
function heldList<T>(value: T | undefined, part: Part): T {
  if (value === undefined) {
    throw new Error(`The page changes the ${part} before it read them`)
  }
  return value
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
      return { type: 'subagents', subagents: heldList(state.subagents, part) }
    case 'instructions':
      return { type: 'instructions', instructions: heldList(state.instructions, part) }
    case 'credentials':
      return { type: 'credentials', credentials: heldList(state.credentials, part) }
    case 'passkeys':
      return { type: 'passkeys', ...heldList(state.passkeys, part) }
    case 'sessions':
      return { type: 'sessions', sessions: heldList(state.sessions, part) }
    case 'users':
      return { type: 'users', users: heldList(state.users, part) }
    case 'automations':
      return { type: 'automations', automations: heldList(state.automations, part), automationsUnread: state.automationsUnread }
    case 'port_forwards':
      return { type: 'port_forwards', portForwards: heldList(state.portForwards, part) }
    case 'exposed_ports':
      return { type: 'exposed_ports', exposedPorts: heldList(state.exposedPorts, part) }
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
 * The build the state names, read before the rest of it: a state of another
 * build, whose other parts this page may not read, still says which build to
 * load.
 */
const stateBuildSchema = productStateSchema.pick({ webBuild: true })

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

/**
 * How long an open channel that brought nothing must stay open before it
 * counts as connected: a backend that accepts the upgrade and closes it at
 * once is not retried at the rate of a quiet network.
 */
const ESTABLISHED_MS = 1_000

/** How long a read of the state after a break may wait, at a random point, so that pages told at once do not read at once. */
const RESTART_SPREAD_MS = 2_000

/** A state of another build that this page cannot read, which only says which build to load. */
class StateOfAnotherBuild extends Error {}

/** Resolves after `ms`, or when `signal` aborts. */
function sleep(ms: number, signal: AbortSignal): Promise<void> {
  if (ms <= 0) {
    return Promise.resolve()
  }
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    signal.addEventListener('abort', () => {
      clearTimeout(timer)
      resolve()
    }, { once: true })
  })
}

/** The field of the state that each list is kept in. */
const LIST_FIELDS = {
  subagents: 'subagents',
  instructions: 'instructions',
  credentials: 'credentials',
  passkeys: 'passkeys',
  sessions: 'sessions',
  users: 'users',
  automations: 'automations',
  port_forwards: 'portForwards',
  exposed_ports: 'exposedPorts',
} as const satisfies Record<ListPart, keyof PageState>

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
 * Holds the list `part` for the view whose scope this runs in, until the
 * scope ends (`useProduct().hold`): a store of a list, whose views call it as
 * they are made, reads and follows the list while one shows.
 */
export function useHeldList(part: ListPart): void {
  if (getCurrentScope()) {
    onScopeDispose(useProduct().hold(part))
  }
}

/**
 * The product state this page shows around its conversations: the one module
 * that follows it, through the synchronization channel and `GET /state`.
 * The page reads the state, which names the version it is current as of, then
 * opens the channel with that version, and the channel carries the parts that
 * changed since, whole, each followed by `synced` with the version they bring
 * the state to. After the channel breaks, its next opening follows from the
 * version the page holds, with no read; only a version the backend no longer
 * keeps, told as `stale`, is read again, after a random wait of up to two
 * seconds. Each message replaces one part of the copy; each state of the page
 * follows the copy by its own rule. The copy is a slot (`web-ui/infra/slot`)
 * read in parts. A list that only a view of settings shows is read when the
 * view holds it (`hold`), by its own route, and followed until the view lets it
 * go. Nothing asks for this state on a timer or after a write: a write's answer
 * goes through `answered`, or lands the change the write made (`change`).
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
   * with the product state and its owner announces anew.
   */
  const catalogLoad = computed(() => snapshot.value === null ? 'loading' : 'ready')
  const activeConversationId = ref<string | null>(null)
  /**
   * The backend closed the channel because it shuts down, and the state has
   * not been read since.
   */
  const restarting = ref(false)
  /** Consecutive connections that ended, and reads of the state that failed, before the state was read. */
  const failures = ref(0)
  const connectionFacts = () => ({
    online: browserOnline.value,
    restarting: restarting.value,
    failedAttempts: failures.value,
    hasSnapshot: snapshot.value !== null,
  })
  /** Whether the page cannot reach the backend. */
  const unreachable = computed(() => backendUnreachable(connectionFacts()))
  /** Every change of where the connection stands is a line, with how long it stood there. */
  let connectionSince = Date.now()
  watch(() => connectionState(connectionFacts()), (to: ConnectionState, from: ConnectionState) => {
    const now = Date.now()
    pageLog.info('connection.state_changed', {
      from,
      to,
      cause: connectionCause(connectionFacts()),
      ms_in_from: now - connectionSince,
      attempt: failures.value,
    })
    connectionSince = now
  })
  /**
   * The page has no state yet and could not reach the backend so far, as a
   * page loaded while Demi restarts: it says it connects where the app will
   * be.
   */
  const connecting = computed(() => snapshot.value === null && (failures.value > 0 || !browserOnline.value))
  /** The build of the web app the backend serves, as the last read of the state named it. */
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
  /** Whether the open channel's `open` went: a value followed from then on goes in a `follow`, before it in the `open`. */
  let channelOpen = false
  /** The wait before the channel connects again, while it is closed. */
  let retry: ReconnectWait | null = null
  /** The open channel's silence watch. */
  let silence: SilenceWatch | null = null
  /** Ends the open channel's grace, which `established` ends first when it comes. */
  let grace: ReturnType<typeof setTimeout> | null = null
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
    channelOpen = false
    silence?.stop()
    silence = null
    if (grace !== null) {
      clearTimeout(grace)
      grace = null
    }
    return channel
  }

  /**
   * The channel stayed open, or brought a message: the backend is reached,
   * so the banner goes and each other socket of the page that waits to
   * connect connects at once.
   */
  function established(): void {
    if (grace !== null) {
      clearTimeout(grace)
      grace = null
    }
    const returned = failures.value > 0 || restarting.value
    failures.value = 0
    restarting.value = false
    if (returned) {
      reconnectNow()
    }
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

  /**
   * What the page follows and holds the version of: the state from the version
   * it was read at, unless it is being read again, and each held list from the
   * tag of its last read, empty when it has none.
   */
  function follows(): SyncFollow[] {
    const result: SyncFollow[] = []
    const version = read.value?.version
    if (version && stateRead === null && !choosing) {
      result.push({ value: 'state', holds: { kind: 'version', version } })
    }
    for (const part of holders.keys()) {
      if (listValue(part) !== undefined && !listReads.has(part)) {
        result.push({ value: part, holds: { kind: 'tag', tag: listTags.get(part) ?? '' } })
      }
    }
    return result
  }

  function send(channel: WebSocket, message: PageMessage): void {
    channel.send(JSON.stringify(message))
  }

  /** Follows `value` from what the page holds of it, on the open channel; a closed one follows it in its next `open`. */
  function follow(value: SyncFollowed): void {
    if (!socket || !channelOpen) {
      return
    }
    const held = follows().find((each) => each.value === value)
    if (held) {
      send(socket, { type: 'follow', ...held })
    }
  }

  function connect(): void {
    if (!controller || socket) {
      return
    }
    clearRetry()
    const url = new URL(apiUrl('/sync'), window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    // A page of this build says so: the backend sends one that names none a
    // single message with its build and closes.
    url.searchParams.set('build', pageBuild() ?? '')
    const channel = openSocket(url, 'synchronization channel')
    socket = channel
    let opened = false
    channel.onopen = () => {
      if (socket !== channel) {
        return
      }
      opened = true
      // A channel that brings nothing, heartbeats included, for too long is broken.
      silence?.stop()
      silence = watchSilence('synchronization channel', replace, pingOn(channel))
      // The page read what it shows before it opened the channel: the open
      // names each value with the version it holds, and the backend sends what
      // changed since.
      send(channel, { type: 'open', follows: follows() })
      channelOpen = true
      if (showing !== null) {
        sendShowing(channel)
      }
      grace = setTimeout(established, ESTABLISHED_MS)
    }
    channel.onmessage = (message) => {
      if (socket !== channel) {
        return
      }
      silence?.heard()
      established()
      const parsed = syncEventSchema.safeParse(parse(message.data))
      if (parsed.success) {
        receive(parsed.data)
        return
      }
      // Another build's contract may say what this page cannot read; the
      // page loads that build once the state named it.
      if (newBuild.value !== null) {
        return
      }
      // A message outside the contract changes nothing; a new connection
      // follows from what the page holds. The console says what did not read.
      reportError(t('failures.readSync'), schemaRefusal(parsed.error), { site: 'failures.readSync' })
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
      // Before the state is read, only the backend's own failure, a close it
      // sent, fails the regions; a channel that could not reach it, or that
      // the backend closed to restart, keeps them loading while it connects
      // again.
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
    send(channel, { type: 'showing', showing })
  }

  /** Says the page shows `conversation`, focused and visible, or none. */
  function show(conversation: string | null): void {
    if (conversation === showing) {
      return
    }
    showing = conversation
    if (socket?.readyState === WebSocket.OPEN && channelOpen) {
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
    if (event.type === 'synced') {
      // The state is current as of the version: the page follows it from there.
      slot.receive('version', (state) => ({ ...state, version: event.version }))
      return
    }
    if (event.type === 'stale') {
      pageLog.info('follow.stale', { value: event.value })
      if (event.value === 'state') {
        readState(true)
      } else if (isListPart(event.value)) {
        readList(event.value, true)
      }
      return
    }
    if (event.type === 'automations_unread') {
      const unread = event.automationsUnread
      slot.receive('automations', (state) => ({ ...state, automationsUnread: unread }))
      return
    }
    if (event.type === 'user' && read.value === null) {
      // The channel of an account still to choose its password carries the
      // account alone, until the choice, on this page or another; the state
      // is read once it is chosen.
      firstUser.value = event.user
      if (!event.user.choosesPassword && choosing) {
        choosing = false
        readState(false)
      }
      return
    }
    if (isListPart(event.type)) {
      receiveList(event.type, event)
      return
    }
    slot.receive(partOf(event), (state) => withPart(state, event))
  }

  /**
   * Whether the page waits for the account's password to be chosen, after the
   * state's read was refused for it.
   */
  let choosing = false
  /** The read of the state that is in flight, which a new read replaces. */
  let stateRead: AbortController | null = null
  /** The wait before the state is read again after a failure, while it waits. */
  let stateWait: ReconnectWait | null = null

  /**
   * Reads the state, which depends on no channel, then follows it from the
   * version it names. A read after the backend said the version the page holds
   * is stale waits a random time of up to two seconds first, so that the pages
   * told at once do not read at once; one that fails is tried again after the
   * page's waits.
   */
  function readState(stale: boolean): void {
    if (!controller) {
      return
    }
    stateRead?.abort()
    stateWait?.cancel()
    const run = new AbortController()
    stateRead = run
    void readStateLoop(run, stale)
  }

  async function readStateLoop(run: AbortController, stale: boolean): Promise<void> {
    const { signal } = run
    if (stale) {
      await sleep(Math.random() * RESTART_SPREAD_MS, signal)
    }
    for (;;) {
      if (signal.aborted) {
        return
      }
      const began = performance.now()
      try {
        const at = slot.now()
        const { state, response } = await fetchState(signal)
        signal.throwIfAborted()
        logReadAnswered(response, began, { read: 'state', stale })
        failures.value = 0
        restarting.value = false
        slot.replace(withHeldLists(state), { since: at })
        loaded = true
        load.value = 'ready'
        servedBuild.value = state.webBuild ?? null
        firstUser.value = null
        stateRead = null
        readHeldLists()
        if (socket) {
          follow('state')
        } else {
          connect()
        }
        return
      } catch (error) {
        if (signal.aborted) {
          return
        }
        if (error instanceof StateOfAnotherBuild) {
          stateRead = null
          return
        }
        if (error instanceof ApiError && error.code === 'password_choice_required') {
          // The account chooses its password first: the channel says when.
          choosing = true
          stateRead = null
          connect()
          return
        }
        failures.value += 1
        logReadFailed(error, began, { read: 'state', failures: failures.value })
        // The backend's own failure, which it answered, fails the regions
        // until the page has a state; a read that could not reach it keeps
        // them loading.
        if (error instanceof ApiError && !backendNotReached(error) && !snapshot.value) {
          load.value = 'failed'
        }
        await new Promise<void>((resolve) => {
          stateWait = waitToReconnect(failures.value, resolve)
          signal.addEventListener('abort', () => stateWait?.cancel(), { once: true })
        })
      }
    }
  }

  /** Reads the state. */
  async function fetchState(signal: AbortSignal): Promise<{ state: PageState; response: Response }> {
    const response = await apiRequest('/state', { signal, waits: false })
    const body = await readResponse(response, z.unknown())
    signal.throwIfAborted()
    const named = stateBuildSchema.safeParse(body)
    if (named.success) {
      servedBuild.value = named.data.webBuild ?? null
    }
    const parsed = pageStateSchema.safeParse(body)
    if (!parsed.success) {
      // A state of another build, whose contract may say what this page
      // cannot read, only names the build to load.
      if (newBuild.value !== null) {
        failures.value = 0
        restarting.value = false
        throw new StateOfAnotherBuild()
      }
      throw new Error(`The state did not read:\n${schemaRefusal(parsed.error)}`)
    }
    return { state: parsed.data, response }
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
   * that could not reach it: when the backend is reachable again, while it is
   * not; otherwise after the page's reconnect wait for that many failures, cut
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
      if (!unreachable.value) {
        wait = waitToReconnect(attempt, ready)
        return
      }
      stopWatching = watch(unreachable, (away) => {
        if (!away) {
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

  /** Whether the state was read since the page started, which a copy of the offline cache does not count. */
  let loaded = false

  /** `state` with the lists the page holds, which the state does not carry. */
  function withHeldLists(state: PageState): PageState {
    const current = read.value
    if (!current) {
      return state
    }
    const lists: Partial<PageState> = {}
    for (const field of Object.values(LIST_FIELDS)) {
      if (current[field] !== undefined) {
        Object.assign(lists, { [field]: current[field] })
      }
    }
    return { ...lists, ...state }
  }

  /** How many views hold each list: it is read and followed while one does. */
  const holders = new Map<ListPart, number>()
  /** Where each held list's read stands, for the view that shows it. */
  const listLoads = reactive<Partial<Record<ListPart, 'loading' | 'ready' | 'failed'>>>({})
  /** The reads of lists that are in flight. */
  const listReads = new Map<ListPart, AbortController>()
  /** The entity tag the page holds of each list, as its last read or message carried it; empty when its route sends none. */
  const listTags = new Map<ListPart, string>()

  /** What the page holds of `part`, or none before the list was read. */
  function listValue(part: ListPart): unknown {
    return read.value?.[LIST_FIELDS[part]]
  }

  /**
   * Holds the list `part` for a view that shows it: the page reads it by its
   * route when the first view holds it, and applies the changes the channel
   * brings of it until the last view lets it go. A list is not read before
   * its view needs it, and a change of one no view holds is ignored. Answers
   * the function that lets it go.
   */
  function hold(part: ListPart): () => void {
    const count = (holders.get(part) ?? 0) + 1
    holders.set(part, count)
    if (count === 1) {
      listLoads[part] = listValue(part) === undefined ? 'loading' : 'ready'
      readList(part, false)
    }
    let released = false
    return () => {
      if (released) {
        return
      }
      released = true
      const left = (holders.get(part) ?? 1) - 1
      if (left > 0) {
        holders.set(part, left)
        return
      }
      holders.delete(part)
      listReads.get(part)?.abort()
      listReads.delete(part)
      listTags.delete(part)
      if (socket && channelOpen) {
        send(socket, { type: 'unfollow', value: part })
      }
    }
  }

  /** Where the read of the held list `part` stands. */
  function listLoad(part: ListPart): 'loading' | 'ready' | 'failed' {
    return listLoads[part] ?? 'loading'
  }

  /** The user's Retry of a list whose read failed. */
  function retryList(part: ListPart): void {
    if (holders.has(part)) {
      if (listValue(part) === undefined) {
        listLoads[part] = 'loading'
      }
      readList(part, false)
    }
  }

  /** Reads each list a view holds that the page holds none of yet, as after the state was read. */
  function readHeldLists(): void {
    for (const part of holders.keys()) {
      if (listValue(part) === undefined && !listReads.has(part)) {
        readList(part, false)
      }
    }
  }

  /**
   * Reads the list `part`, which a view holds, once the state is there, by its
   * route, which depends on no channel, then follows it from the tag the
   * answer names; a read that fails is tried again after the page's waits. A
   * read after the backend said its tag is stale waits a random time first.
   */
  function readList(part: ListPart, stale: boolean): void {
    listReads.get(part)?.abort()
    if (read.value === null || !controller) {
      // The state's read starts every held list's.
      return
    }
    const run = new AbortController()
    listReads.set(part, run)
    void (async () => {
      const route = LIST_ROUTES[part]
      if (stale) {
        await sleep(Math.random() * RESTART_SPREAD_MS, run.signal)
      }
      for (let failed = 0; ; ) {
        const began = performance.now()
        try {
          const at = slot.now()
          const response = await apiRequest(route.route, { signal: run.signal })
          const tag = response.headers.get('ETag') ?? ''
          const body = await readResponse(response, route.schema as z.ZodType<never>)
          run.signal.throwIfAborted()
          logReadAnswered(response, began, { read: part, stale })
          slot.receive(part, (state) => withPart(state, (route.event as (body: never, state: ProductState) => PartEvent)(body, state)), { since: at })
          listTags.set(part, tag)
          listReads.delete(part)
          listLoads[part] = 'ready'
          follow(part)
          return
        } catch (error) {
          if (run.signal.aborted) {
            return
          }
          if (listValue(part) === undefined) {
            listLoads[part] = 'failed'
          }
          failed += 1
          logReadFailed(error, began, { read: part, failures: failed })
          await new Promise<void>((resolve) => {
            const wait = waitToReconnect(failed, resolve)
            run.signal.addEventListener('abort', () => wait.cancel(), { once: true })
          })
          if (run.signal.aborted) {
            return
          }
        }
      }
    })()
  }

  /**
   * A message of a list: applied while a view holds it, and the tag it
   * carries is the one the page follows the list from. A list no view holds
   * is ignored.
   */
  function receiveList(part: ListPart, event: PartEvent): void {
    if (!holders.has(part)) {
      return
    }
    if ('etag' in event && event.etag !== undefined) {
      listTags.set(part, event.etag)
    }
    slot.receive(partOf(event), (state) => withPart(state, event))
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
    // The state is read first, and the channel opens after it, to follow
    // what it read. A copy that shows meanwhile and holds a version is
    // followed from instead (`showCopy`).
    readState(false)
    // Its failure is the Add Provider dialog's region state, with its own Retry.
    void loadVendors().catch(() => {})
  }

  /**
   * Shows `state`, the account's copy kept for opening offline, which the page
   * follows from the version it holds with no read, as after a break: what
   * changed since comes on the channel, or is read again when the backend no
   * longer keeps that version. A page that holds a state already keeps it.
   */
  function showCopy(state: ProductState): void {
    if (read.value !== null) {
      return
    }
    slot.replace(state)
    load.value = 'ready'
    if (controller && !loaded && !choosing) {
      stateRead?.abort()
      stateWait?.cancel()
      stateRead = null
      loaded = true
      connect()
    }
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
    if (!loaded && !choosing) {
      // The state's read failed, and goes again at once.
      readState(false)
      return
    }
    if (!socket) {
      connect()
    }
  }

  function stop(): void {
    controller?.abort()
    controller = null
    stopWaiting?.()
    stopWaiting = null
    clearRetry()
    stateRead?.abort()
    stateRead = null
    stateWait?.cancel()
    dropChannel()?.close()
    loaded = false
    choosing = false
    for (const run of listReads.values()) {
      run.abort()
    }
    listReads.clear()
    listTags.clear()
    for (const part of Object.keys(listLoads) as ListPart[]) {
      delete listLoads[part]
    }
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
    unreachable,
    newBuild,
    catalog,
    automaticSmallModel,
    vendors,
    vendorLoad,
    catalogLoad,
    activeConversationId,
    show,
    hold,
    listLoad,
    retryList,
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
