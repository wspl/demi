import type {
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

/** Use Your Credentials, as `demi credentials` declares it. */
export const useCredentials: PermissionCategoryView = {
  id: 'credentials.use',
  action: 'use your credentials',
  description:
    'Use Your Credentials lets the agents of this conversation list, read, save, replace and remove your saved credentials, and ask you for new ones.',
}

/** A command on a device the conversation does not reach yet, which asks for that device. */
export function deviceRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-device',
    categories: [],
    command: 'demi host shell --host build-box "df -h /srv"',
    subagent: null,
    device: { id: 'device-build-box', name: 'build-box' },
  }
}

/** A read of a credential, which asks for all of the user's credentials. */
export function credentialsRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-credentials',
    categories: [useCredentials],
    command: 'demi credentials read "GitHub token for deploys" token | gh auth login --with-token',
    subagent: null,
    device: null,
  }
}

/** The root's request, as the design's example raises it. */
export function rootRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-root',
    categories: [manageSkills],
    command: 'demi skills add vercel-labs/agent-skills --skill web-design-guidelines',
    subagent: null,
    device: null,
  }
}

/** A subagent's request. */
export function subagentRequest(): PermissionRequestView {
  return {
    kind: 'permission',
    id: 'pr-subagent',
    categories: [manageSkills],
    command: 'demi skills enable vercel-labs/agent-skills --skill react-best-practices',
    subagent: { number: 2, description: 'Frontend review' },
    device: null,
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
      command: 'demi conversations show 12',
      subagent: { number: 3, description: 'History search' },
      device: null,
    },
  ]
}

/** The design's example: an API key to add Google as a provider, one hidden field, kept for an hour. */
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

/** A sign-in: a visible user name and a hidden password, kept for 30 days. */
export function signInRequest(): CredentialRequestView {
  return {
    kind: 'credential',
    id: 'cr-linear',
    name: 'Linear sign-in for alex',
    reason: 'To file the issues you listed in the Linear project',
    fields: [
      { name: 'user', label: 'User name', kind: 'visible' },
      { name: 'password', label: 'Password', kind: 'secret' },
    ],
    keep: '30d',
    renewal: false,
  }
}

/** A deploy token the user asked the agent to keep using: kept until the user removes it. */
export function deployTokenRequest(): CredentialRequestView {
  return {
    kind: 'credential',
    id: 'cr-deploy',
    name: 'GitHub token for deploys',
    reason: 'To deploy the site whenever you ask',
    fields: [{ name: 'token', label: 'Personal access token', kind: 'secret' }],
    keep: 'always',
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
