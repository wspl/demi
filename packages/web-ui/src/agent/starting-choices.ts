import type { ModelSettings } from './model-selection'
import type { WorkflowChoice, WorkflowList } from './workflow-menu'

/**
 * What a new conversation starts with (`web-api.md` § User preferences): the
 * model settings and the workflow last chosen in its project, or outside
 * projects for a conversation outside one. A project keeps them on its
 * workspace, the conversations outside a project in the preferences.
 */
export interface RememberedChoices {
  lastModel?: ModelSettings
  lastWorkflow?: WorkflowChoice
}

/** The choices a new conversation starts with. */
export interface StartingChoices {
  /** None before the user's first choice, which the composer then asks for. */
  model: ModelSettings | undefined
  workflow: WorkflowChoice
}

const NO_SUBAGENTS: WorkflowChoice = { type: 'none' }

/**
 * The choices a new conversation starts with, each on its own: its
 * `project`'s, or, where the project remembers none, as a project never
 * used, one the page does not know or a conversation outside projects
 * (`project` null) does, that of the conversations outside a project. A
 * workflow remembered nowhere is No Subagents.
 */
export function startingChoices(project: RememberedChoices | null, outside: RememberedChoices): StartingChoices {
  return {
    model: project?.lastModel ?? outside.lastModel,
    workflow: project?.lastWorkflow ?? outside.lastWorkflow ?? NO_SUBAGENTS,
  }
}

/**
 * The workflow a conversation that has not started yet goes on with once
 * its directory's workflows are read: No Subagents in place of a workflow
 * the directory does not have, `choice` otherwise, and while nothing says
 * so: the list is being read, failed, or there is no directory to read.
 */
export function startingWorkflow(choice: WorkflowChoice, list: WorkflowList): WorkflowChoice {
  if (choice.type !== 'workflow' || list.state !== 'read')
    return choice
  return list.workflows.some((workflow) => workflow.id === choice.id) ? choice : NO_SUBAGENTS
}
