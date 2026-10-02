import { z } from 'zod'
import { callEditSelectionSchema, type CallEditSelection, type ChangeFile, type ChangeMode } from '@demicodes/plugin-sdk'

/**
 * What the Change view shows (`edit-tracking.md` § Delivery to the
 * conversation): its mode, the working-tree file it holds in Uncommitted,
 * the call's edit it shows in Conversation, and what Back and Forward return
 * to.
 */
const changeStepSchema = z.object({
  mode: z.enum(['conversation', 'uncommitted']),
  /** The working-tree path to show; null selects its first listed file. */
  uncommitted: z.string().nullable(),
  call: callEditSelectionSchema.nullable(),
  /** The edit segment of the call's file. */
  edit: z.int().min(0),
})
export type ChangeStep = z.infer<typeof changeStepSchema>

export const changeDataSchema = changeStepSchema.extend({
  back: z.array(changeStepSchema),
  forward: z.array(changeStepSchema),
})
export type ChangeData = z.infer<typeof changeDataSchema>

/** The view as a conversation's panel first shows it: Uncommitted, with nothing to go back to. */
export function firstChangeData(): ChangeData {
  return { mode: 'uncommitted', uncommitted: null, call: null, edit: 0, back: [], forward: [] }
}

/** Conversation paths come from the picked file; only the working tree holds a list selection. */
export function changePath(data: ChangeStep, mode: ChangeMode = data.mode, files?: readonly ChangeFile[]): string | null {
  if (mode === 'conversation') {
    return data.call?.file.path ?? null
  }
  if (!files || files.some((file) => file.path === data.uncommitted)) {
    return data.uncommitted
  }
  return files[0]?.path ?? null
}

function step(data: ChangeData): ChangeStep {
  return { mode: data.mode, uncommitted: data.uncommitted, call: data.call, edit: data.edit }
}

/**
 * The view showing `path` in `mode`, remembering what it showed: a mode
 * switch and a pick in the tree are both steps Back returns to.
 */
export function showChange(
  data: ChangeData,
  mode: ChangeMode,
  path: string | null,
  selection?: { call: CallEditSelection | null; edit: number },
): ChangeData {
  const call = selection ? selection.call : data.call
  const nextPath = mode === 'conversation' ? call?.file.path ?? null : path
  const sameFile = data.mode === mode && changePath(data, mode) === nextPath
    && data.call?.commandId === call?.commandId
  const edit = selection?.edit ?? (sameFile ? data.edit : 0)
  if (sameFile && data.edit === edit) {
    return data
  }
  return {
    mode,
    uncommitted: mode === 'uncommitted' ? path : data.uncommitted,
    call,
    edit,
    back: [...data.back, step(data)],
    forward: [],
  }
}

/** A file pill: the call's edit of that file, in Conversation mode. */
export function showCallEdit(data: ChangeData, selection: CallEditSelection): ChangeData {
  return showChange(data, 'conversation', null, { call: selection, edit: 0 })
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
