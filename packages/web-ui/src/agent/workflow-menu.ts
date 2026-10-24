import { t } from './locales/text'

/**
 * The composer's Workflow menu: what the conversation's choice shows as, and
 * the rows the menu offers, derived from the choice and the project's workflows
 * as the page keeps them, with those of the linked worktree the conversation
 * works in while they differ. /
 */

/**
 * The conversation's choice, as its record holds it: a workflow is the
 * project's, or with `worktree` the worktree's.
 */
export type WorkflowChoice =
  | { type: 'default' }
  | { type: 'none' }
  | { type: 'workflow'; id: string; worktree?: boolean }

/** A workflow file of the project, as the backend lists it. */
export interface WorkflowOption {
  /** The file's name without `.yaml`. */
  id: string
  /** What the menu shows; null for a file that cannot be used. */
  name: string | null
  /** The file on the Host. */
  path: string
  /**
   * Why it cannot be used, a link or a file that does not read; null while it
   * can.
   */
  unavailable: string | null
  /** The ids of the profiles it declares, their `--profile` values. */
  profiles: readonly string[]
}

/**
 * The project's workflows as the page holds them:
 *
 * - none to read, for a conversation with no directory yet, which offers
 *   Default and No Subagents
 * - being read
 * - read
 * - their directory not usable, as a link, with why
 * - not readable now
 */
export type WorkflowList =
  | { state: 'nowhere' }
  | { state: 'reading' }
  | { state: 'read'; workflows: readonly WorkflowOption[]; worktree?: WorktreeWorkflows }
  | { state: 'unavailable'; reason: string }
  | { state: 'failed'; reason: string }

/**
 * The workflows of the linked worktree the conversation works in, while they
 * differ from its project's: the menu lists each under its name.
 */
export interface WorktreeWorkflows {
  /** The project's name, which heads its own workflows. */
  project: string
  /** The worktree's name, as git names it. */
  name: string
  list: WorkflowList
}

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

/** The rows of one directory's workflows, headed by its name when the menu lists two. */
export interface WorkflowGroup {
  key: string
  label: string | null
  rows: WorkflowRow[]
}

/** What the menu's trigger shows. */
export interface WorkflowShown {
  label: string
  /** Why the chosen workflow cannot be used now; null while it can. */
  unavailable: string | null
}

function sameChoice(a: WorkflowChoice, b: WorkflowChoice): boolean {
  if (a.type !== 'workflow' || b.type !== 'workflow')
    return a.type === b.type
  return a.id === b.id && (a.worktree ?? false) === (b.worktree ?? false)
}

/**
 * The list a workflow `choice` is read from: the worktree's for a worktree
 * workflow while the menu lists it, else the project's `list`.
 */
function chosenList(choice: WorkflowChoice, list: WorkflowList): WorkflowList {
  if (choice.type === 'workflow' && choice.worktree && list.state === 'read')
    return list.worktree?.list ?? list
  return list
}

/**
 * The user's subagent settings as Global Workflow reads them: the Subagents
 * switch and each profile's.
 */
export interface GlobalProfiles {
  enabled: boolean
  profiles: readonly { name: string, enabled: boolean }[]
}

/**
 * The profiles `choice` offers now, by their `--profile` values:
 *
 * - for a project workflow, its profiles from `list` while it is read
 * - under Global Workflow, the user's enabled profiles while the Subagents
 *   switch is on, none while it is off
 * - none under No Subagents
 *   (`packages/web-ui/src/agent/ContextUsageIndicator.vue`)
 */
export function workflowProfiles(choice: WorkflowChoice, list: WorkflowList, global: GlobalProfiles): readonly string[] {
  if (choice.type === 'default')
    return global.enabled ? global.profiles.filter((profile) => profile.enabled).map((profile) => profile.name) : []
  const from = chosenList(choice, list)
  if (choice.type === 'none' || from.state !== 'read')
    return []
  return from.workflows.find((workflow) => workflow.id === choice.id)?.profiles ?? []
}

/**
 * What the trigger shows for `choice`, its name from `list` while it is read.
 */
export function workflowShown(choice: WorkflowChoice, shownList: WorkflowList): WorkflowShown {
  if (choice.type === 'default')
    return { label: t('workflows.default'), unavailable: null }
  if (choice.type === 'none')
    return { label: t('workflows.none'), unavailable: null }
  const list = chosenList(choice, shownList)
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
 * The rows of the menu: Global Workflow and No Subagents, then the project's
 * workflows, and the worktree's under each one's name while they differ.
 */
export function workflowRows(choice: WorkflowChoice, list: WorkflowList): { fixed: WorkflowRow[]; groups: WorkflowGroup[] } {
  const fixed = [
    row('default', t('workflows.default'), { type: 'default' }, choice),
    row('none', t('workflows.none'), { type: 'none' }, choice),
  ]
  const worktree = list.state === 'read' ? list.worktree : undefined
  if (!worktree) {
    // A worktree's workflow, once the worktree's are the project's again,
    // is the project's row.
    const shown: WorkflowChoice = choice.type === 'workflow' && choice.worktree ? { type: 'workflow', id: choice.id } : choice
    const rows = listRows(shown, list, false)
    return { fixed, groups: rows.length ? [{ key: 'project', label: null, rows }] : [] }
  }
  return {
    fixed,
    groups: [
      { key: 'project', label: worktree.project, rows: listRows(choice, list, false) },
      { key: 'worktree', label: t('workflows.worktree', { name: worktree.name }), rows: listRows(choice, worktree.list, true) },
    ],
  }
}

function row(key: string, label: string, made: WorkflowChoice | null, choice: WorkflowChoice, unavailable: string | null = null): WorkflowRow {
  return { key, label, choice: made, selected: made !== null && sameChoice(made, choice), unavailable }
}

/**
 * The rows of one directory's `list`, the worktree's with `worktree`: each
 * workflow by its name, one that does not read with why, and the chosen one
 * when it is gone.
 */
function listRows(choice: WorkflowChoice, list: WorkflowList, worktree: boolean): WorkflowRow[] {
  const prefix = worktree ? 'worktree:' : ''
  const chosen = choice.type === 'workflow' && (choice.worktree ?? false) === worktree ? choice : null
  const chosenRow = (unavailable: string | null) => chosen ? [row(`${prefix}workflow:${chosen.id}`, chosen.id, chosen, choice, unavailable)] : []
  if (list.state === 'nowhere')
    return chosenRow(null)
  if (list.state === 'reading')
    return [row(`${prefix}reading`, t('workflows.reading'), null, choice)]
  if (list.state === 'failed')
    return [row(`${prefix}failed`, t('workflows.failed'), null, choice, list.reason)]
  if (list.state === 'unavailable')
    return [row(`${prefix}unavailable`, t('workflows.unavailable'), null, choice, list.reason), ...chosenRow(list.reason)]
  const rows = list.workflows.map((workflow) => row(
    `${prefix}workflow:${workflow.id}`,
    workflow.name ?? workflow.id,
    worktree ? { type: 'workflow', id: workflow.id, worktree: true } : { type: 'workflow', id: workflow.id },
    choice,
    workflow.unavailable,
  ))
  if (chosen && !list.workflows.some((workflow) => workflow.id === chosen.id))
    rows.push(...chosenRow(t('workflows.gone')))
  return rows
}
