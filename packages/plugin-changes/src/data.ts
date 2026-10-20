import { z } from 'zod'
import {
  requestEditSelectionSchema,
  sameEditRef,
  type ChangeFile,
  type ChangeMode,
  type IntentPayloads,
  type RequestEditRef,
  type RequestEditSelection,
} from '@demicodes/plugin-sdk'

/**
 * What the Change view shows (`crates/agent-store/src/changes.rs`,
 * `packages/web-ui/src/files/history.ts`):
 *
 * - Its mode.
 * - In Commits, the item selected (the repository's uncommitted changes or one
 *   of its commits) and the file of it shown.
 * - In Conversation, the request, file and edit it holds.
 * - What Back and Forward return to.
 *
 * A request is named, never copied: the view reads it from the transcript as it
 * is now.
 */
const changeStepSchema = z.object({
  mode: z.enum(['commits', 'conversation']),
  /** In Commits, the item selected: a commit's id, null for Uncommitted Changes. */
  commit: z.string().nullable(),
  /** In Commits, the file of the item to show; null selects its first file. */
  file: z.string().nullable(),
  /**
   * The directory Commits mode reads when a commit link named one: the
   * working directory of the agent that wrote it, null for the
   * conversation's; absent, the request's.
   */
  root: z.string().nullable().optional(),
  /** The request, its file and its edit, or All Changes; null before any was opened. */
  request: requestEditSelectionSchema.nullable(),
})
export type ChangeStep = z.infer<typeof changeStepSchema>

export const changeDataSchema = changeStepSchema.extend({
  /** The branch the commit list shows, null for the checked-out one; choosing one is no step for Back. */
  branch: z.string().nullable(),
  /** A commit link opened the commit shown, and the list's branch is still to be placed for it. */
  place: z.boolean(),
  back: z.array(changeStepSchema),
  forward: z.array(changeStepSchema),
})
export type ChangeData = z.infer<typeof changeDataSchema>

/** The view as a conversation's panel first shows it: Commits, on Uncommitted Changes, with nothing to go back to. */
export function firstChangeData(): ChangeData {
  return { mode: 'commits', commit: null, file: null, request: null, branch: null, place: false, back: [], forward: [] }
}

/**
 * The file the view shows:
 *
 * - Conversation paths come from the request's selection.
 * - In Commits, a commit's file is the file held, or its first while it holds
 *   none, which the view picks.
 * - The uncommitted changes' file is the file held, or the first listed file
 *   while it holds none or one no longer listed.
 */
export function changePath(data: ChangeStep, mode: ChangeMode = data.mode, files?: readonly ChangeFile[]): string | null {
  if (mode === 'conversation') {
    return data.request?.file ?? null
  }
  if (data.commit !== null || !files || files.some((file) => file.path === data.file)) {
    return data.file
  }
  return files[0]?.path ?? null
}

/**
 * `data` holding the working-tree file it shows while it held none: the
 * first listed file then stays shown as the list changes, as a file the
 * user picked does, since only the user moves the view. No step for Back:
 * the view shows what it showed.
 */
export function holdShownChange(data: ChangeData, files: readonly ChangeFile[]): ChangeData {
  if (data.mode !== 'commits' || data.commit !== null || data.file !== null) {
    return data
  }
  const shown = changePath(data, 'commits', files)
  return shown === null ? data : { ...data, file: shown }
}

function step(data: ChangeData): ChangeStep {
  const { mode, commit, file, root, request } = data
  return { mode, commit, file, ...(root === undefined ? {} : { root }), request }
}

function sameRequest(a: RequestEditSelection | null, b: RequestEditSelection | null): boolean {
  return a === b || (a !== null && b !== null && a.node === b.node && a.request === b.request
    && a.file === b.file && a.root === b.root && sameEditRef(a.edit, b.edit))
}

/** `data` showing `next`, the step it showed kept for Back; the same step again changes nothing. */
function go(data: ChangeData, next: ChangeStep): ChangeData {
  if (next.mode === data.mode && next.commit === data.commit && next.file === data.file
    && next.root === data.root && sameRequest(next.request, data.request)) {
    return data
  }
  const { root: _root, ...rest } = data
  return { ...rest, ...next, back: [...data.back, step(data)], forward: [] }
}

/** The step `data` shows, with what Commits mode shows unchanged. */
function commits(data: ChangeData): Omit<ChangeStep, 'mode' | 'request'> {
  const { commit, file, root } = data
  return { commit, file, ...(root === undefined ? {} : { root }) }
}

/**
 * The view showing `path` in `mode`, remembering what it showed: a mode
 * switch and a pick in the sidebar are both steps Back returns to. Another
 * file of the request shows its All Changes.
 */
export function showChange(data: ChangeData, mode: ChangeMode, path: string | null): ChangeData {
  if (mode === 'commits') {
    return go(data, { ...commits(data), mode, file: path, request: data.request })
  }
  const request = data.request && path !== null && path !== data.request.file
    ? { ...data.request, file: path, edit: null }
    : data.request
  return go(data, { ...commits(data), mode, request })
}

/** The view showing an item of the commit list, a commit or Uncommitted Changes for null, at its first file. */
export function showItem(data: ChangeData, commit: string | null): ChangeData {
  return go(data, { ...commits(data), mode: 'commits', commit, file: null, request: data.request })
}

/**
 * The `commit` intent: a commit link, in Commits mode, in the directory its
 * agent worked in, its list's branch placed once the commit is read
 * (`placed`).
 */
export function showCommit(data: ChangeData, payload: IntentPayloads['commit']): ChangeData {
  const shown = go(data, { mode: 'commits', commit: payload.commit, file: null, root: payload.root ?? null, request: data.request })
  return { ...shown, place: true }
}

/** The list placed on `branch` for the commit a link opened. */
export function placeBranch(data: ChangeData, branch: string | null): ChangeData {
  return { ...data, branch, place: false }
}

/** The commit list showing `branch`, null for the checked-out one. */
export function showBranch(data: ChangeData, branch: string | null): ChangeData {
  return branch === data.branch ? data : { ...data, branch }
}

/** The view showing another edit of the file it shows, or its All Changes. */
export function showEdit(data: ChangeData, edit: RequestEditRef | null): ChangeData {
  if (!data.request) {
    return data
  }
  return go(data, { ...commits(data), mode: 'conversation', request: { ...data.request, edit } })
}

/** The `edit` intent: a file pill, in Conversation mode; Commits then reads the request's directory again. */
export function showRequestEdit(data: ChangeData, selection: RequestEditSelection): ChangeData {
  return go(data, { mode: 'conversation', commit: data.commit, file: data.file, request: selection })
}

/** The view showing what it showed before, the current step kept for Forward. */
export function goBack(data: ChangeData): ChangeData {
  const previous = data.back.at(-1)
  if (!previous) {
    return data
  }
  const { root: _root, ...rest } = data
  return { ...rest, ...previous, back: data.back.slice(0, -1), forward: [...data.forward, step(data)] }
}

/** The view showing what Back left, the current step kept for Back. */
export function goForward(data: ChangeData): ChangeData {
  const next = data.forward.at(-1)
  if (!next) {
    return data
  }
  const { root: _root, ...rest } = data
  return { ...rest, ...next, forward: data.forward.slice(0, -1), back: [...data.back, step(data)] }
}

/**
 * The working tree the view shows: the directory a commit link named, else
 * the one its request was opened for, the conversation's when neither names
 * one.
 */
export function viewDirectory(data: ChangeStep): string | undefined {
  if (data.root !== undefined)
    return data.root ?? undefined
  return data.request?.root
}
