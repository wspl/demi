/**
 * The composer's Workflow menu (`product.md` § A conversation's workflow):
 * what the conversation's choice shows as, and the rows the menu offers,
 * derived from the choice and the project's workflows as the page keeps them.
 */

/** The conversation's choice, as its record holds it. */
export type WorkflowChoice =
  | { type: 'default' }
  | { type: 'none' }
  | { type: 'workflow'; id: string }

/** A workflow file of the project, as the backend lists it. */
export interface WorkflowOption {
  /** The file's name without `.yaml`. */
  id: string
  /** What the menu shows; null for a file that does not read. */
  name: string | null
  /** Why the file does not read; null while it does. */
  error: string | null
}

/**
 * The project's workflows as the page holds them: being read; read; their
 * directory not usable, as a link, with why; or not readable now.
 */
export type WorkflowList =
  | { state: 'reading' }
  | { state: 'read'; workflows: readonly WorkflowOption[] }
  | { state: 'unavailable'; reason: string }
  | { state: 'failed'; reason: string }

/** One row of the menu. */
export interface WorkflowRow {
  key: string
  label: string
  /** The choice the row makes; null for a row that only says something. */
  choice: WorkflowChoice | null
  selected: boolean
  /** Why it cannot be chosen; null when it can. */
  unavailable: string | null
}

/** What the menu's trigger shows. */
export interface WorkflowShown {
  label: string
  /** Why the chosen workflow cannot be used now; null while it can. */
  unavailable: string | null
}

const GONE = 'This workflow is no longer in the project. Choose another.'

function sameChoice(a: WorkflowChoice, b: WorkflowChoice): boolean {
  return a.type === b.type && (a.type !== 'workflow' || (b.type === 'workflow' && a.id === b.id))
}

/** What the trigger shows for `choice`, its name from `list` while it is read. */
export function workflowShown(choice: WorkflowChoice, list: WorkflowList): WorkflowShown {
  if (choice.type === 'default')
    return { label: 'Default', unavailable: null }
  if (choice.type === 'none')
    return { label: 'No Subagents', unavailable: null }
  if (list.state === 'unavailable')
    return { label: choice.id, unavailable: list.reason }
  if (list.state !== 'read')
    return { label: choice.id, unavailable: null }
  const listed = list.workflows.find((workflow) => workflow.id === choice.id)
  if (!listed)
    return { label: choice.id, unavailable: GONE }
  return { label: listed.name ?? choice.id, unavailable: listed.error }
}

/**
 * The rows of the menu: Default and No Subagents, then each workflow by its
 * name, one that does not read with why, and the chosen one when it is gone.
 */
export function workflowRows(choice: WorkflowChoice, list: WorkflowList): WorkflowRow[] {
  const row = (key: string, label: string, made: WorkflowChoice | null, unavailable: string | null = null): WorkflowRow => ({
    key,
    label,
    choice: made,
    selected: made !== null && sameChoice(made, choice),
    unavailable,
  })
  const rows = [
    row('default', 'Default', { type: 'default' }),
    row('none', 'No Subagents', { type: 'none' }),
  ]
  if (list.state === 'reading')
    return [...rows, row('reading', 'Reading Workflows…', null)]
  if (list.state === 'failed')
    return [...rows, row('failed', 'Couldn’t Read Workflows', null, list.reason)]
  if (list.state === 'unavailable') {
    const shown = [...rows, row('unavailable', 'Workflows Unavailable', null, list.reason)]
    return choice.type === 'workflow' ? [...shown, row(`workflow:${choice.id}`, choice.id, choice, list.reason)] : shown
  }
  for (const workflow of list.workflows) {
    rows.push(row(`workflow:${workflow.id}`, workflow.name ?? workflow.id, { type: 'workflow', id: workflow.id }, workflow.error))
  }
  if (choice.type === 'workflow' && !list.workflows.some((workflow) => workflow.id === choice.id))
    rows.push(row(`workflow:${choice.id}`, choice.id, choice, GONE))
  return rows
}
