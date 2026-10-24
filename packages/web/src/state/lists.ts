import { z } from 'zod'
import {
  credentialDtoSchema,
  credentialListSchema,
  exposedPortDtoSchema,
  exposedPortListSchema,
  instructionsAnswerSchema,
  listedUserSchema,
  passkeyListSchema,
  portForwardDtoSchema,
  portForwardListSchema,
  productStateSchema,
  sessionDtoSchema,
  sessionListSchema,
  subagentSettingsSchema,
  usersSchema,
  type ProductState,
  type SyncEvent,
} from '../api/generated/web-api'

/**
 * The lists only a view of settings shows, which the product state does not
 * carry: the page reads a list by its route when a view that shows it opens,
 * and follows its changes from the channel until the view closes. Each is
 * absent from the page's state until it was read, and kept in the offline copy
 * with the state.
 */
export const heldListsShape = {
  subagents: subagentSettingsSchema.optional(),
  instructions: z.string().optional(),
  credentials: z.array(credentialDtoSchema).optional(),
  passkeys: passkeyListSchema.optional(),
  sessions: z.array(sessionDtoSchema).optional(),
  users: z.array(listedUserSchema).optional(),
  portForwards: z.array(portForwardDtoSchema).optional(),
  exposedPorts: z.array(exposedPortDtoSchema).optional(),
}

/** The state a read of `GET /state` gives: the whole product state, with the lists the page holds. */
export const readStateSchema = productStateSchema.extend(heldListsShape)
export type ReadState = z.infer<typeof readStateSchema>

/**
 * The state the page holds: each part is missing until it is known, from
 * the state's read, the offline copy's or a list's own read.
 */
export type PageState = Partial<ReadState>

/** A list the page holds only while a view shows it, named as the message that carries it. */
export type ListPart = 'subagents' | 'instructions' | 'credentials' | 'passkeys' | 'sessions' | 'users' | 'port_forwards' | 'exposed_ports'

/** `event` without the entity tag a list's message carries, which an answer of a write has none of. */
export type Untagged<Event> = Event extends { etag: string } ? Omit<Event, 'etag'> & { etag?: string } : Event

type PartEventOf<Type extends SyncEvent['type']> = Untagged<Extract<SyncEvent, { type: Type }>>

/** How a list is read: its route, and the message it makes of the answer, as the channel's message of that list says it. */
interface ListRoute<Type extends ListPart, Body> {
  route: string
  schema: z.ZodType<Body>
  /** The part of `state` the answer makes, which the answer replaces whole. */
  event: (body: Body, state: ProductState) => PartEventOf<Type>
}

function route<Type extends ListPart, Body>(read: ListRoute<Type, Body>): ListRoute<Type, Body> {
  return read
}

export const LIST_ROUTES = {
  subagents: route({
    route: '/settings/subagents',
    schema: subagentSettingsSchema,
    event: (subagents) => ({ type: 'subagents', subagents }),
  }),
  instructions: route({
    route: '/settings/instructions',
    schema: instructionsAnswerSchema,
    event: ({ instructions }) => ({ type: 'instructions', instructions }),
  }),
  credentials: route({
    route: '/credentials',
    schema: credentialListSchema,
    event: ({ credentials }) => ({ type: 'credentials', credentials }),
  }),
  passkeys: route({
    route: '/auth/passkeys',
    schema: passkeyListSchema,
    event: ({ passkeys, userHandle }) => ({ type: 'passkeys', passkeys, userHandle }),
  }),
  sessions: route({
    route: '/auth/sessions',
    schema: sessionListSchema,
    event: ({ sessions }) => ({ type: 'sessions', sessions }),
  }),
  users: route({
    route: '/users',
    schema: usersSchema,
    event: ({ users }) => ({ type: 'users', users }),
  }),
  port_forwards: route({
    route: '/port-forwards',
    schema: portForwardListSchema,
    event: ({ forwards }) => ({ type: 'port_forwards', portForwards: forwards }),
  }),
  exposed_ports: route({
    route: '/exposed-ports',
    schema: exposedPortListSchema,
    event: ({ ports }) => ({ type: 'exposed_ports', exposedPorts: ports }),
  }),
}

/** Whether `type` names a list the page holds only while a view shows it. */
export function isListPart(type: string): type is ListPart {
  return type in LIST_ROUTES
}
