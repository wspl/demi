import { t } from './locales/text'

/**
 * The composer's Workflow menu: what the conversation's choice shows as, and the
 * rows the menu offers, derived from the choice and the project's workflows as
 * the page keeps them.
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
  /** What the menu shows; null for a file that cannot be used. */
  name: string | null
  /** The file on the Host. */
  path: string
  /** Why it cannot be used, a link or a file that does not read; null while it can. */
  unavailable: string | null
  /** The ids of the profiles it declares, their `--profile` values. */
  profiles: readonly string[]
}

/**
 * The project's workflows as the page holds them: none to read, for a
 * conversation with no directory yet, which offers Default and No
 * Subagents; being read; read; their directory not usable, as a link, with
 * why; or not readable now.
 */
export type WorkflowList =
  | { state: 'nowhere' }
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

function sameChoice(a: WorkflowChoice, b: WorkflowChoice): boolean {
  return a.type === b.type && (a.type !== 'workflow' || (b.type === 'workflow' && a.id === b.id))
}

/** The user's subagent settings as Global Workflow reads them: the Subagents switch and each profile's. */
export interface GlobalProfiles {
  enabled: boolean
  profiles: readonly { name: string, enabled: boolean }[]
}

/**
 * The profiles `choice` offers now, by their `--profile` values: a project
 * workflow's, from `list` while it is read; under Global Workflow the user's
 * enabled profiles while the Subagents switch is on, none while it is off;
 * and none under No Subagents (`packages/web-ui/src/agent/ContextUsageIndicator.vue`).
 */
export function workflowProfiles(choice: WorkflowChoice, list: WorkflowList, global: GlobalProfiles): readonly string[] {
  if (choice.type === 'default')
    return global.enabled ? global.profiles.filter((profile) => profile.enabled).map((profile) => profile.name) : []
  if (choice.type === 'none' || list.state !== 'read')
    return []
  return list.workflows.find((workflow) => workflow.id === choice.id)?.profiles ?? []
}

/** What the trigger shows for `choice`, its name from `list` while it is read. */
export function workflowShown(choice: WorkflowChoice, list: WorkflowList): WorkflowShown {
  if (choice.type === 'default')
    return { label: t('workflows.default'), unavailable: null }
  if (choice.type === 'none')
    return { label: t('workflows.none'), unavailable: null }
  if (list.state === 'unavailable')
    return { label: choice.id, unavailable: list.reason }
  if (list.state !== 'read')
    return { label: choice.id, unavailable: null }
  const listed = list.workflows.find((workflow) => workflow.id === choice.id)
  if (!listed)
    return { label: choice.id, unavailable: t('workflows.gone') }
  return { label: listed.name ?? choice.id, unavailable: listed.unavailable }
}

/**
 * The rows of the menu: Global Workflow and No Subagents, then each workflow by its
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
    row('default', t('workflows.default'), { type: 'default' }),
    row('none', t('workflows.none'), { type: 'none' }),
  ]
  if (list.state === 'nowhere')
    return choice.type === 'workflow' ? [...rows, row(`workflow:${choice.id}`, choice.id, choice)] : rows
  if (list.state === 'reading')
    return [...rows, row('reading', t('workflows.reading'), null)]
  if (list.state === 'failed')
    return [...rows, row('failed', t('workflows.failed'), null, list.reason)]
  if (list.state === 'unavailable') {
    const shown = [...rows, row('unavailable', t('workflows.unavailable'), null, list.reason)]
    return choice.type === 'workflow' ? [...shown, row(`workflow:${choice.id}`, choice.id, choice, list.reason)] : shown
  }
  for (const workflow of list.workflows) {
    rows.push(row(`workflow:${workflow.id}`, workflow.name ?? workflow.id, { type: 'workflow', id: workflow.id }, workflow.unavailable))
  }
  if (choice.type === 'workflow' && !list.workflows.some((workflow) => workflow.id === choice.id))
    rows.push(row(`workflow:${choice.id}`, choice.id, choice, t('workflows.gone')))
  return rows
}
