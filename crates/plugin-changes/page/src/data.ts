import { ref } from 'vue'
import { z } from 'zod'
import {
  definePlace,
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
 *
 * What it showed before is the web browser's history, through the page's
 * address (`changePlace`). A request is named, never copied: the view reads it
 * from the transcript as it is now.
 */
export const changeDataSchema = z.object({
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
  /** The branch the commit list shows, null for the checked-out one; choosing one is no step of the history. */
  branch: z.string().nullable(),
  /** A commit link opened the commit shown, and the list's branch is still to be placed for it. */
  place: z.boolean(),
  /**
   * Whether a Markdown or SVG file's change shows its diff or both sides
   * rendered, as the address records it for the change shown; absent, the
   * view the user last chose anywhere (`changeView`), which the address
   * records with the change.
   */
  view: z.enum(['diff', 'preview']).optional(),
})
export type ChangeData = z.infer<typeof changeDataSchema>

/** Where the view is: what a move changes, and what Back returns to. */
type ChangeStep = Pick<ChangeData, 'mode' | 'commit' | 'file' | 'root' | 'request'>

/**
 * Markdown and SVG: the view the user last chose, the text diff or both
 * sides rendered, for the page's lifetime, in which a change the address
 * names no view for shows.
 */
export const changeView = ref<'diff' | 'preview'>('diff')

/** The view as a conversation's panel first shows it: Commits, on Uncommitted Changes. */
export function firstChangeData(): ChangeData {
  return { mode: 'commits', commit: null, file: null, request: null, branch: null, place: false }
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
export function changePath(data: ChangeData, mode: ChangeMode = data.mode, files?: readonly ChangeFile[]): string | null {
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

function sameRequest(a: RequestEditSelection | null, b: RequestEditSelection | null): boolean {
  return a === b || (a !== null && b !== null && a.node === b.node && a.request === b.request
    && a.file === b.file && a.root === b.root && sameEditRef(a.edit, b.edit))
}

/** `data` showing `next`; the same step again changes nothing. */
function go(data: ChangeData, next: ChangeStep): ChangeData {
  if (next.mode === data.mode && next.commit === data.commit && next.file === data.file
    && next.root === data.root && sameRequest(next.request, data.request)) {
    return data
  }
  // Another change shows in the view last chosen, until the tab records it.
  const { root: _root, view: _view, ...rest } = data
  return { ...rest, ...next }
}

/** The step `data` shows, with what Commits mode shows unchanged. */
function commits(data: ChangeData): Omit<ChangeStep, 'mode' | 'request'> {
  const { commit, file, root } = data
  return { commit, file, ...(root === undefined ? {} : { root }) }
}

/**
 * The view showing `path` in `mode`: a mode switch and a pick in the sidebar
 * are both steps Back returns to. Another file of the request shows its All
 * Changes.
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

/** The view's choice of diff or preview for a Markdown file's change: how it is seen, not what. */
export function showPresentation(data: ChangeData, presentation: 'diff' | 'preview'): ChangeData {
  return data.view === presentation ? data : { ...data, view: presentation }
}

/**
 * The working tree the view shows: the directory a commit link named, else
 * the one its request was opened for, the conversation's when neither names
 * one.
 */
export function viewDirectory(data: ChangeData): string | undefined {
  if (data.root !== undefined)
    return data.root ?? undefined
  return data.request?.root
}

/**
 * The Change view's place in the page's address, as
 * `?panel=change&mode=conversation&request=u1&path=src/app.ts&call=c2&segment=0`:
 *
 * - `mode`, `commit`, `file`, `root` and `branch`: what Commits shows, a
 *   commit link's directory `root`, empty for the conversation's.
 * - `request`, `agent`, `path` and `directory`: the request held and its file,
 *   and `call`, `segment` and `edited` its edit, absent for All Changes.
 * - `view`: a Markdown or SVG change shown as its diff or its preview; a
 *   change opened without one is written with the one last chosen
 *   (`changeView`), in the same move.
 */
export const changePlace = definePlace({
  schema: z.object({
    mode: z.literal('conversation').optional(),
    commit: z.string().optional(),
    file: z.string().optional(),
    root: z.string().optional(),
    branch: z.string().optional(),
    request: z.string().optional(),
    agent: z.string().optional(),
    path: z.string().optional(),
    directory: z.string().optional(),
    call: z.string().optional(),
    segment: z.coerce.number<string>().int().min(0).optional(),
    edited: z.string().optional(),
    view: z.enum(['diff', 'preview']).optional(),
  }),
  write: (data: ChangeData) => {
    const request = data.request
    const edit = request?.edit ?? null
    return {
      mode: data.mode === 'conversation' ? 'conversation' as const : undefined,
      commit: data.commit ?? undefined,
      file: data.file ?? undefined,
      root: data.root === undefined ? undefined : data.root ?? '',
      branch: data.branch ?? undefined,
      request: request?.request,
      agent: request?.node ?? undefined,
      path: request?.file,
      directory: request?.root,
      call: edit?.call,
      segment: edit === null ? undefined : String(edit.segment),
      edited: edit === null || edit.path === request?.file ? undefined : edit.path,
      view: data.view ?? changeView.value,
    }
  },
  read: (place, current): ChangeData => {
    const request: RequestEditSelection | null = place.request === undefined || place.path === undefined
      ? null
      : {
          node: place.agent ?? null,
          request: place.request,
          file: place.path,
          edit: place.call === undefined || place.segment === undefined
            ? null
            : { call: place.call, path: place.edited ?? place.path, segment: place.segment },
          ...(place.directory === undefined ? {} : { root: place.directory }),
        }
    const commit = place.commit ?? null
    return {
      mode: place.mode ?? 'commits',
      commit,
      file: place.file ?? null,
      ...(place.root === undefined ? {} : { root: place.root === '' ? null : place.root }),
      request,
      branch: place.branch ?? null,
      // A commit link places the list's branch for the commit it opened.
      place: current.place && current.commit === commit,
      ...(place.view === undefined ? {} : { view: place.view }),
    }
  },
})
