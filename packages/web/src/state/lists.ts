import type { z } from 'zod'
import {
  cloudIsolationSchema,
  credentialListSchema,
  instructionsAnswerSchema,
  passkeyListSchema,
  portForwardListSchema,
  sessionListSchema,
  subagentSettingsSchema,
  usersSchema,
  type ProductState,
  type SyncEvent,
  type Untagged,
} from '@demicodes/protocol'

/**
 * A list the page holds only while a view shows it, named as the message
 * that carries it (`heldListsShape`): the page reads it by its route when a
 * view that shows it opens, follows its changes from the channel until the
 * view closes, and keeps it in the offline copy with the state.
 */
export type ListPart =
  | 'subagents'
  | 'instructions'
  | 'credentials'
  | 'passkeys'
  | 'sessions'
  | 'users'
  | 'port_forwards'
  | 'cloud_isolation'

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
    event: ({ users, cloudStorageLimits }) => ({ type: 'users', users, cloudStorageLimits }),
  }),
  port_forwards: route({
    route: '/port-forwards',
    schema: portForwardListSchema,
    event: ({ forwards }) => ({ type: 'port_forwards', portForwards: forwards }),
  }),
  cloud_isolation: route({
    route: '/settings/cloud-isolation',
    schema: cloudIsolationSchema,
    event: (isolation) => ({ type: 'cloud_isolation', isolation }),
  }),
}

/** Whether `type` names a list the page holds only while a view shows it. */
export function isListPart(type: string): type is ListPart {
  return type in LIST_ROUTES
}
