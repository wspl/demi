import type {
  AllowanceRequestView,
  CredentialRequestView,
  PermissionCategoryView,
  PermissionRequestView,
} from '@demicodes/web-ui/permissions/types'

/** Manage Skills, as `plugin-skills` declares it. */
export const manageSkills: PermissionCategoryView = {
  id: 'skills.manage',
  action: 'manage skills',
  description:
    'Manage Skills lets the agents of this conversation add, update and remove skill sources and turn skills on or off. Your skills reach every conversation, and the skills that are on are installed on every Host your conversations use.',
}

/** A category the user's command set no longer declares: the page knows its id alone. */
export const retiredCategory: PermissionCategoryView = {
  id: 'conversations.read',
  action: null,
  description: null,
}

/** Organize Conversations, as the product's `demi conversation` declares it. */
export const organizeConversations: PermissionCategoryView = {
  id: 'conversation.organize',
  action: 'organize conversations',
  description:
    'Organize Conversations lets the agents of this conversation rename it, list your projects, make a project of a directory, and move this conversation into or out of one. Projects show in your sidebar on every device.',
}

/** A move into a project on a host the conversation may not reach, without Organize Conversations: one request of the category and the host. */
export function moveRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-move',
    categories: [organizeConversations],
    host: { kind: 'device', deviceId: 'device-build-box', name: 'build-box' },
    command: 'demi conversation move photo-album',
    subagent: null,
  }
}

/** A move to a host the conversation may not reach: a request of the host alone. */
export function hostRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-host',
    categories: [],
    host: { kind: 'device', deviceId: 'device-build-box', name: 'build-box' },
    command: 'demi host move build-box /srv/website',
    subagent: null,
  }
}

/** A pairing, which asks every time. */
export function pairingRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-pair',
    categories: [],
    host: { kind: 'pairing' },
    command: 'demi host pair K7QM-2XWD',
    subagent: null,
  }
}

/** The root's request, as the design's example raises it. */
export function rootRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-root',
    categories: [manageSkills],
    host: null,
    command: 'demi skills add vercel-labs/agent-skills --skill web-design-guidelines',
    subagent: null,
  }
}

/** A subagent's request. */
export function subagentRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-subagent',
    categories: [manageSkills],
    host: null,
    command: 'demi skills enable vercel-labs/agent-skills --skill react-best-practices',
    subagent: { number: 2, description: 'Frontend review' },
  }
}

/** Three requests waiting at once: two of Manage Skills, from the root and a subagent, and one of a category no longer declared. */
export function queuedRequests(): PermissionRequestView[] {
  return [
    rootRequest(),
    subagentRequest(),
    {
      kind: 'permission',
      id: 'pr-retired',
      categories: [retiredCategory],
      host: null,
      command: 'demi conversations show 12',
      subagent: { number: 3, description: 'History search' },
    },
  ]
}

/** The design's example: an API key to add Google as a provider, one hidden field, kept for an hour unless the user says otherwise. */
export function googleKeyRequest(): CredentialRequestView {
  return {
    kind: 'credential',
    id: 'cr-google',
    name: 'Google API key',
    reason: 'To add Google as a provider',
    fields: [{ name: 'key', label: 'API key', kind: 'secret' }],
    keep: '1h',
    renewal: false,
  }
}

/** A sign-in: a visible user name and a hidden password, the agent proposing 30 days. */
export function signInRequest(): CredentialRequestView {
  return {
    kind: 'credential',
    id: 'cr-linear',
    name: 'Linear sign-in for alex',
    reason: 'To file the issues you listed in the Linear workspace',
    fields: [
      { name: 'user', label: 'User name', kind: 'visible' },
      { name: 'password', label: 'Password', kind: 'secret' },
    ],
    keep: '30d',
    renewal: false,
  }
}

/** A credential kept an hour that ran out: the agent asks for its fields again. */
export function renewalRequest(): CredentialRequestView {
  return { ...googleKeyRequest(), id: 'cr-renewal', reason: 'The saved key was rejected; it may have been rotated', renewal: true }
}

/** What the backend says of values that do not match the request's fields. */
export const INVALID_ANSWER = 'The value of “API key” can be at most 64 KiB.'

/** What the backend says of an answer that would renew a saved credential with other fields. */
export const FIELDS_DIFFER =
  '“Google API key” already has other fields; answer with the same fields or choose another name.'

/** A stored credential the conversation asks to use. */
export function allowanceRequest(): AllowanceRequestView {
  return { kind: 'allowance', id: 'al-github', name: 'GitHub token for deploys', removed: false }
}

/** The same request after the user removed the credential in Settings. */
export function removedAllowanceRequest(): AllowanceRequestView {
  return { ...allowanceRequest(), id: 'al-removed', removed: true }
}
