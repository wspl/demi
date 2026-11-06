// The product state as a client holds it, and how a part replaces its own
// in it: what the synchronization channel's messages and a write's answers
// change, alike in the web app and a program's SDK.
import { z } from 'zod'
import {
  cloudIsolationSchema,
  credentialDtoSchema,
  passkeyListSchema,
  portForwardDtoSchema,
  productStateSchema,
  sessionDtoSchema,
  subagentSettingsSchema,
  usersSchema,
  type ConversationSummary,
  type SyncEvent,
} from './generated/web-api'

/**
 * The lists only a view of settings shows, which the product state does not
 * carry: a client reads a list by its route when a view that shows it opens,
 * and follows its changes from the channel until the view closes. Each is
 * absent from the state until it was read.
 */
export const heldListsShape = {
  subagents: subagentSettingsSchema.optional(),
  instructions: z.string().optional(),
  credentials: z.array(credentialDtoSchema).optional(),
  passkeys: passkeyListSchema.optional(),
  sessions: z.array(sessionDtoSchema).optional(),
  users: usersSchema.optional(),
  portForwards: z.array(portForwardDtoSchema).optional(),
  cloudIsolation: cloudIsolationSchema.optional(),
}

/** The state a read of `GET /state` gives: the whole product state, with the lists a client holds. */
export const readStateSchema = productStateSchema.extend(heldListsShape)
export type ReadState = z.infer<typeof readStateSchema>

/**
 * The state a client holds: each part is missing until it is known, from
 * the state's read, a kept copy or a list's own read.
 */
export type PageState = Partial<ReadState>

/** `event` without the entity tag a list's message carries, which an answer of a write has none of. */
export type Untagged<Event> = Event extends { etag: string } ? Omit<Event, 'etag'> & { etag?: string } : Event

/** A message that carries one part of the product state, as a write's answer can. */
export type PartEvent = Untagged<Exclude<SyncEvent, { type: 'synced' | 'stale' | 'update' }>>

/** A part of the product state, which a message or a write's answer replaces whole. */
export type Part = `conversation:${string}` | `plugin:${string}` | Exclude<PartEvent['type'], 'conversation' | 'conversation_deleted' | 'plugin'>

/** The part `event` carries. */
export function partOf(event: PartEvent): Part {
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
    return conversations.map((conversation, at) => (at === index ? summary : conversation))
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
 * leave at once; a plugin turned on sends its state afresh.
 */
function withoutOff(
  states: ReadState['pluginStates'] | undefined,
  plugins: ReadState['plugins'],
): ReadState['pluginStates'] | undefined {
  if (!states) {
    return undefined
  }
  const off = new Set(plugins.filter((plugin) => !plugin.enabled).map((plugin) => plugin.id))
  return Object.fromEntries(Object.entries(states).filter(([id]) => !off.has(id)))
}

/**
 * A part the client holds, which what reads it presupposes: a change of it
 * comes from what shows it, and a message of one item of it from the
 * channel, which opens only once the state is read.
 */
export function known<T>(value: T | undefined, part: string): T {
  if (value === undefined) {
    throw new Error(`The ${part} were read before they were known`)
  }
  return value
}

/** The value `state` holds of `part`, as a message that carries it says it. */
export function partEvent(state: PageState, part: Part): PartEvent {
  switch (part) {
    case 'conversation_order':
      return { type: 'conversation_order', ids: known(state.conversations, part).map((conversation) => conversation.id) }
    case 'preferences':
      return { type: 'preferences', preferences: known(state.preferences, part) }
    case 'instance':
      return { type: 'instance', mode: known(state.mode, part), manages: known(state.manages, part) }
    case 'user':
      return { type: 'user', user: known(state.user, part) }
    case 'workspaces':
      return { type: 'workspaces', workspaces: known(state.workspaces, part) }
    case 'projects':
      return { type: 'projects', projects: known(state.projects, part) }
    case 'devices':
      return { type: 'devices', devices: known(state.devices, part) }
    case 'plugins':
      return { type: 'plugins', plugins: known(state.plugins, part) }
    case 'providers':
      return { type: 'providers', providers: known(state.providers, part) }
    case 'cloud':
      return { type: 'cloud', cloud: known(state.cloud, part) }
    case 'logins':
      return { type: 'logins', logins: known(state.logins, part) }
    case 'subagents':
      return { type: 'subagents', subagents: known(state.subagents, part) }
    case 'instructions':
      return { type: 'instructions', instructions: known(state.instructions, part) }
    case 'credentials':
      return { type: 'credentials', credentials: known(state.credentials, part) }
    case 'passkeys':
      return { type: 'passkeys', ...known(state.passkeys, part) }
    case 'sessions':
      return { type: 'sessions', sessions: known(state.sessions, part) }
    case 'users':
      return { type: 'users', ...known(state.users, part) }
    case 'port_forwards':
      return { type: 'port_forwards', portForwards: known(state.portForwards, part) }
    case 'cloud_isolation':
      return { type: 'cloud_isolation', isolation: known(state.cloudIsolation, part) }
    case 'models':
      return { type: 'models', models: known(state.models, part) }
    case 'server_upgrade':
      return { type: 'server_upgrade', upgrade: known(state.serverUpgrade ?? undefined, part) }
  }
  if (part.startsWith('plugin:')) {
    const plugin = part.slice('plugin:'.length)
    return { type: 'plugin', plugin, state: known(state.pluginStates, part)[plugin] ?? null }
  }
  const id = part.slice('conversation:'.length)
  const conversation = known(state.conversations, part).find((candidate) => candidate.id === id)
  return conversation ? { type: 'conversation', conversation } : { type: 'conversation_deleted', id }
}

/** `state` with the part `event` carries replaced. */
export function withPart(state: PageState, event: PartEvent): PageState {
  switch (event.type) {
    case 'conversation':
      return { ...state, conversations: withSummary(known(state.conversations, 'conversations'), event.conversation) }
    case 'conversation_deleted':
      return { ...state, conversations: known(state.conversations, 'conversations').filter((conversation) => conversation.id !== event.id) }
    case 'conversation_order':
      return { ...state, conversations: inOrder(known(state.conversations, 'conversations'), event.ids) }
    case 'preferences':
      return { ...state, preferences: event.preferences }
    case 'instance':
      return { ...state, mode: event.mode, manages: event.manages }
    case 'user':
      // A user's channel never carries the server's updates: an account made
      // a user drops what it held as an administrator.
      return { ...state, user: event.user, serverUpgrade: event.user.role === 'user' ? null : state.serverUpgrade }
    case 'workspaces':
      return { ...state, workspaces: event.workspaces }
    case 'projects':
      return { ...state, projects: event.projects }
    case 'devices':
      return { ...state, devices: event.devices }
    case 'plugins':
      return { ...state, plugins: event.plugins, pluginStates: withoutOff(state.pluginStates, event.plugins) }
    case 'plugin':
      return { ...state, pluginStates: { ...known(state.pluginStates, 'plugin states'), [event.plugin]: event.state } }
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
      return { ...state, users: { users: event.users, cloudStorageLimits: event.cloudStorageLimits } }
    case 'port_forwards':
      return { ...state, portForwards: event.portForwards }
    case 'cloud_isolation':
      return { ...state, cloudIsolation: event.isolation }
    case 'models':
      return { ...state, models: event.models }
    case 'server_upgrade':
      return { ...state, serverUpgrade: event.upgrade }
  }
}
