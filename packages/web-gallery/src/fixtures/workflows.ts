import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'

/** A project's workflows as the backend lists them: two that read and one that does not. */
export const projectWorkflows: WorkflowList = {
  state: 'read',
  workflows: [
    { id: 'docs', name: 'Docs Review', path: '/Users/alex/app/.demi/workflows/docs.yaml', unavailable: null, profiles: ['scout'], userProfiles: false },
    {
      id: 'release',
      name: null,
      path: '/Users/alex/app/.demi/workflows/release.yaml',
      unavailable: '/Users/alex/app/.demi/workflows/release.yaml does not read: line 4 column 9: validation error: is not 1 to 40 lowercase letters, digits and hyphens starting with a letter for `profiles[0].id`',
      profiles: [],
      userProfiles: false,
    },
    { id: 'review', name: 'Review Team', path: '/Users/alex/app/.demi/workflows/review.yaml', unavailable: null, profiles: ['explore', 'reviewer'], userProfiles: true },
  ],
}

/** A project without workflows: the menu offers Global Workflow and No Subagents only. */
export const noWorkflows: WorkflowList = { state: 'read', workflows: [] }

/**
 * A conversation in the linked worktree 2 of the project app, whose
 * worktree's workflows differ from the project's: the menu lists both, each
 * under its name.
 */
export const worktreeWorkflows: WorkflowList = {
  state: 'read',
  workflows: [
    { id: 'parallel', name: 'Parallel Development', path: '/Users/alex/app/.demi/workflows/parallel.yaml', unavailable: null, profiles: ['builder', 'reviewer'], userProfiles: true },
    { id: 'review', name: 'Review Team', path: '/Users/alex/app/.demi/workflows/review.yaml', unavailable: null, profiles: ['explore', 'reviewer'], userProfiles: true },
  ],
  worktree: {
    project: 'app',
    name: '2',
    list: {
      state: 'read',
      workflows: [
        { id: 'parallel', name: 'Parallel Development', path: '/Users/alex/app-worktrees/2/.demi/workflows/parallel.yaml', unavailable: null, profiles: ['builder'], userProfiles: true },
      ],
    },
  },
}
