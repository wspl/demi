import type { PermissionCategoryView, PermissionRequestView } from '@demicodes/web-ui/permissions/types'

/** Manage skills, as `plugin-skills` declares it. */
export const manageSkills: PermissionCategoryView = {
  id: 'skills.manage',
  action: 'manage skills',
  description:
    'Manage skills lets the agents of this conversation add, update and remove skill sources and turn skills on or off. Your skills reach every conversation, and the skills that are on are installed on every Host your conversations use.',
}

/** A category the user's command set no longer declares: the page knows its id alone. */
export const retiredCategory: PermissionCategoryView = {
  id: 'conversations.read',
  action: null,
  description: null,
}

/** The root's request, as the design's example raises it. */
export function rootRequest(): PermissionRequestView {
  return {
    id: 'pr-root',
    category: manageSkills,
    command: 'demi skills add vercel-labs/agent-skills --skill web-design-guidelines',
    subagent: null,
  }
}

/** A subagent's request. */
export function subagentRequest(): PermissionRequestView {
  return {
    id: 'pr-subagent',
    category: manageSkills,
    command: 'demi skills enable vercel-labs/agent-skills --skill react-best-practices',
    subagent: { number: 2, description: 'Frontend review' },
  }
}

/** Three requests waiting at once: two of Manage skills, from the root and a subagent, and one of a category no longer declared. */
export function queuedRequests(): PermissionRequestView[] {
  return [
    rootRequest(),
    subagentRequest(),
    {
      id: 'pr-retired',
      category: retiredCategory,
      command: 'demi conversations show 12',
      subagent: { number: 3, description: 'History search' },
    },
  ]
}
