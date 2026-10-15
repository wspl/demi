import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'

/** A project's workflows as the backend lists them: two that read and one that does not. */
export const projectWorkflows: WorkflowList = {
  state: 'read',
  workflows: [
    { id: 'docs', name: 'Docs Review', path: '/Users/alex/app/.demi/workflows/docs.yaml', unavailable: null },
    {
      id: 'release',
      name: null,
      path: '/Users/alex/app/.demi/workflows/release.yaml',
      unavailable: '/Users/alex/app/.demi/workflows/release.yaml does not read: line 4 column 9: validation error: is not 1 to 40 lowercase letters, digits and hyphens starting with a letter for `profiles[0].id`',
    },
    { id: 'review', name: 'Review Team', path: '/Users/alex/app/.demi/workflows/review.yaml', unavailable: null },
  ],
}

/** A project without workflows: the menu offers Global Workflow and No Subagents only. */
export const noWorkflows: WorkflowList = { state: 'read', workflows: [] }
