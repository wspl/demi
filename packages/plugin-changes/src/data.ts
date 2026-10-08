import { z } from 'zod'
import {
  requestEditSelectionSchema,
  type ChangeFile,
  type ChangeMode,
  type RequestEditRef,
  type RequestEditSelection,
} from '@demicodes/plugin-sdk'

/**
 * What the Change view shows (`edit-tracking.md` § What the conversation
 * shows): its mode, the working-tree file it holds in Uncommitted, the
 * request, file and edit it holds in Conversation, and what Back and Forward
 * return to. A request is named, never copied: the view reads it from the
 * transcript as it is now.
 */
const changeStepSchema = z.object({
  mode: z.enum(['conversation', 'uncommitted']),
  /** The working-tree path to show; null selects its first listed file. */
  uncommitted: z.string().nullable(),
  /** The request, its file and its edit, or All Changes; null before any was opened. */
  request: requestEditSelectionSchema.nullable(),
})
export type ChangeStep = z.infer<typeof changeStepSchema>

export const changeDataSchema = changeStepSchema.extend({
  back: z.array(changeStepSchema),
  forward: z.array(changeStepSchema),
})
export type ChangeData = z.infer<typeof changeDataSchema>

/** The view as a conversation's panel first shows it: Uncommitted, with nothing to go back to. */
export function firstChangeData(): ChangeData {
  return { mode: 'uncommitted', uncommitted: null, request: null, back: [], forward: [] }
}

/** Conversation paths come from the request's selection; only the working tree holds a list selection. */
export function changePath(data: ChangeStep, mode: ChangeMode = data.mode, files?: readonly ChangeFile[]): string | null {
  if (mode === 'conversation') {
    return data.request?.file ?? null
  }
  if (!files || files.some((file) => file.path === data.uncommitted)) {
    return data.uncommitted
  }
  return files[0]?.path ?? null
}

function step(data: ChangeData): ChangeStep {
  return { mode: data.mode, uncommitted: data.uncommitted, request: data.request }
}

function sameEdit(a: RequestEditRef | null, b: RequestEditRef | null): boolean {
  return a === b || (a !== null && b !== null && a.call === b.call && a.segment === b.segment)
}

function sameRequest(a: RequestEditSelection | null, b: RequestEditSelection | null): boolean {
  return a === b || (a !== null && b !== null && a.node === b.node && a.request === b.request
    && a.file === b.file && sameEdit(a.edit, b.edit))
}

/** `data` showing `next`, the step it showed kept for Back; the same step again changes nothing. */
function go(data: ChangeData, next: ChangeStep): ChangeData {
  if (next.mode === data.mode && next.uncommitted === data.uncommitted && sameRequest(next.request, data.request)) {
    return data
  }
  return { ...next, back: [...data.back, step(data)], forward: [] }
}

/**
 * The view showing `path` in `mode`, remembering what it showed: a mode
 * switch and a pick in the sidebar are both steps Back returns to. Another
 * file of the request shows its All Changes.
 */
export function showChange(data: ChangeData, mode: ChangeMode, path: string | null): ChangeData {
  if (mode === 'uncommitted') {
    return go(data, { mode, uncommitted: path, request: data.request })
  }
  const request = data.request && path !== null && path !== data.request.file
    ? { ...data.request, file: path, edit: null }
    : data.request
  return go(data, { mode, uncommitted: data.uncommitted, request })
}

/** The view showing another edit of the file it shows, or its All Changes. */
export function showEdit(data: ChangeData, edit: RequestEditRef | null): ChangeData {
  if (!data.request) {
    return data
  }
  return go(data, { mode: 'conversation', uncommitted: data.uncommitted, request: { ...data.request, edit } })
}

/** The `edit` intent: a request's line or a file pill, in Conversation mode. */
export function showRequestEdit(data: ChangeData, selection: RequestEditSelection): ChangeData {
  return go(data, { mode: 'conversation', uncommitted: data.uncommitted, request: selection })
}

/** The view showing what it showed before, the current step kept for Forward. */
export function goBack(data: ChangeData): ChangeData {
  const previous = data.back.at(-1)
  if (!previous) {
    return data
  }
  return { ...previous, back: data.back.slice(0, -1), forward: [...data.forward, step(data)] }
}

/** The view showing what Back left, the current step kept for Back. */
export function goForward(data: ChangeData): ChangeData {
  const next = data.forward.at(-1)
  if (!next) {
    return data
  }
  return { ...next, forward: data.forward.slice(0, -1), back: [...data.back, step(data)] }
}
