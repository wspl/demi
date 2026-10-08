import type { PermissionCategoryView, PermissionRequestView } from '@demicodes/web-ui/permissions/types'

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

/** Manage Devices, as the product's `demi host` declares it. */
export const manageDevices: PermissionCategoryView = {
  id: 'host.devices',
  action: 'manage devices',
  description:
    'Manage Devices lets the agents of this conversation list your devices, and attach them to this conversation or detach them. The agents run commands as you on an attached device.',
}

/** A move into a project on a device the conversation lacks, without either grant: one request of both categories. */
export function moveRequest(): PermissionRequestView {
  return {
    id: 'pr-move',
    categories: [organizeConversations, manageDevices],
    command: 'demi conversation move ledable-app',
    subagent: null,
  }
}

/** The root's request, as the design's example raises it. */
export function rootRequest(): PermissionRequestView {
  return {
    id: 'pr-root',
    categories: [manageSkills],
    command: 'demi skills add vercel-labs/agent-skills --skill web-design-guidelines',
    subagent: null,
  }
}

/** A subagent's request. */
export function subagentRequest(): PermissionRequestView {
  return {
    id: 'pr-subagent',
    categories: [manageSkills],
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
      id: 'pr-retired',
      categories: [retiredCategory],
      command: 'demi conversations show 12',
      subagent: { number: 3, description: 'History search' },
    },
  ]
}
