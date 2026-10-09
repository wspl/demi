import type { Block, EditCopies, PathChange } from '@demicodes/protocol'
import { z } from 'zod'
import { storedShellView, toolCallTitle } from '../agent/block-helpers'
import type { ToolCallBlock } from '../agent/block-types'
import type { ReadCallChange } from './changes'
import { diffLineCounts } from './diff-counts'
import { isWithinPath } from './paths'

/**
 * A request's changes (`edit-tracking.md` § Delivery to the conversation):
 * what one message of the user asked for, from its `user` block to the same
 * agent's next one, and the files its calls changed. Derived from one
 * agent's transcript whenever it is shown, never stored: a subagent's
 * changes are in its own transcript, not its parent's.
 */

/** One edit of a request's file: one segment of one of its calls. */
export interface RequestEdit {
  /** The command whose job made the edit. */
  call: string
  /** The call's title, as its row in the transcript names it. */
  title: string
  /** The file's path in the call, which a later call may have renamed. */
  path: string
  /** Which of the call's segments of the file it is. */
  segment: number
  /** The segment created the file, so nothing came before it. */
  created: boolean
  /** The file's two sides; absent when they were not stored. */
  copies?: EditCopies
}

/**
 * A file a request changed, as it stands after the request's latest call,
 * with its edits in the order they were made: a file a later call renamed
 * is listed under its new name with its earlier edits, and one a later call
 * renamed away or removed is not listed (`edit-tracking.md` § A request).
 */
export interface RequestFile {
  /** As the Host names it. */
  path: string
  /** `added` when the request created it. */
  kind: 'added' | 'modified'
  edits: RequestEdit[]
}

export interface TranscriptRequest {
  /** The `user` block that starts it. */
  id: string
  /** In the order they were first changed. */
  files: RequestFile[]
}

export interface TranscriptRequests {
  requests: TranscriptRequest[]
  /** The request each block belongs to, by block id; a block before the first `user` block has none. */
  requestOf: ReadonlyMap<string, TranscriptRequest>
}

/**
 * One agent's requests. Only a `user` block starts one: a steer, a wakeup,
 * another agent's message and a resume continue the request they follow.
 */
export function transcriptRequests(blocks: readonly Block[]): TranscriptRequests {
  const requests: TranscriptRequest[] = []
  const requestOf = new Map<string, TranscriptRequest>()
  let current: { request: TranscriptRequest; calls: Map<string, number> } | null = null
  for (const block of blocks) {
    if (block.type === 'user') {
      current = { request: { id: block.id, files: [] }, calls: new Map() }
      requests.push(current.request)
    }
    if (!current) {
      continue
    }
    requestOf.set(block.id, current.request)
    if (block.type === 'tool_call') {
      addCall(current.request.files, current.calls, block)
    }
  }
  return { requests, requestOf }
}

/**
 * The files `calls` changed, in the order first changed, each with its edits
 * in order: a request's, or a run of its calls the transcript shows as one
 * row, whose files are counted from the same two ends.
 */
export function callFiles(calls: readonly ToolCallBlock[]): RequestFile[] {
  const files: RequestFile[] = []
  const order = new Map<string, number>()
  for (const call of calls) {
    addCall(files, order, call)
  }
  return files
}

/**
 * Adds a call to the files of the calls before it, `order` numbering the
 * calls as they come: first its renames and removals, in the order it made
 * them, move or drop the files the earlier calls changed; then its own
 * files, which its job already names as they stand after it, join them.
 */
function addCall(files: RequestFile[], order: Map<string, number>, block: ToolCallBlock): void {
  const view = storedShellView(block)
  if (!view) {
    return
  }
  order.set(view.commandId, order.size)
  // Files a rename of this call brought to a name the list did not hold:
  // whether a file was there before, only the call's own entry can tell.
  const arrived = new Set<string>()
  for (const change of view.pathChanges ?? []) {
    applyPathChange(files, order, change, arrived)
  }
  const title = toolCallTitle(block)
  for (const file of view.files ?? []) {
    let entry = files.find((existing) => existing.path === file.path)
    if (!entry) {
      entry = { path: file.path, kind: file.kind, edits: [] }
      files.push(entry)
    } else if (arrived.has(file.path)) {
      entry.kind = file.kind
    }
    file.edits.forEach((edit, segment) => {
      entry.edits.push({
        call: view.commandId,
        title,
        path: file.path,
        segment,
        created: file.kind === 'added' && segment === 0,
        ...(edit.copies ? { copies: edit.copies } : {}),
      })
    })
  }
}

/**
 * Follows one rename or removal: a removed path, or a folder with all in it,
 * leaves the list; a renamed one takes its new name with its edits, merged
 * into the entry the new name already has, if any, which keeps its kind and
 * its place, or the earlier of the two.
 */
function applyPathChange(
  files: RequestFile[],
  order: ReadonlyMap<string, number>,
  change: PathChange,
  arrived: Set<string>,
): void {
  if (change.kind === 'removed') {
    const kept = files.filter((file) => !isWithinPath(file.path, change.path))
    files.splice(0, files.length, ...kept)
    return
  }
  for (const moved of files.filter((file) => isWithinPath(file.path, change.from))) {
    const path = change.to + moved.path.slice(change.from.length)
    const target = files.find((file) => file.path === path)
    if (!target) {
      moved.path = path
      moved.kind = 'added'
      arrived.add(path)
      continue
    }
    // Each call's edits stay in their order, the replaced file's first.
    const callOrder = (edit: RequestEdit) => order.get(edit.call) ?? 0
    target.edits = [...target.edits, ...moved.edits].sort((a, b) => callOrder(a) - callOrder(b))
    const at = Math.min(files.indexOf(moved), files.indexOf(target))
    files.splice(files.indexOf(moved), 1)
    files.splice(files.indexOf(target), 1)
    files.splice(at, 0, target)
  }
}

/** The transcripts of one conversation: its own agent's, and each subagent's by its id. */
export interface ConversationTranscripts {
  blocks: readonly Block[]
  subagents: readonly { id: string; blocks: readonly Block[] }[]
}

/** The request `request` of the agent `node` (null for the conversation's own), as its transcript holds it now. */
export function findRequest(
  transcripts: ConversationTranscripts,
  node: string | null,
  request: string,
): TranscriptRequest | null {
  const blocks = node === null
    ? transcripts.blocks
    : transcripts.subagents.find((agent) => agent.id === node)?.blocks
  if (!blocks) {
    return null
  }
  return transcriptRequests(blocks).requests.find((entry) => entry.id === request) ?? null
}

/**
 * One edit of a request's file, by its call, the file's path in that call
 * and the segment, which stay as later calls add edits or rename the file.
 */
export const requestEditRefSchema = z.object({
  call: z.string(),
  path: z.string(),
  segment: z.int().min(0),
})
export type RequestEditRef = z.infer<typeof requestEditRefSchema>

/**
 * What the `edit` intent opens (`plugin-pages.md` § Intents): a request of
 * one agent, the file of it to show, and the edit of that file, or null for
 * All Changes.
 */
export const requestEditSelectionSchema = z.object({
  /** The agent whose transcript holds the request: null for the conversation's own, a subagent's id otherwise. */
  node: z.string().nullable(),
  /** The `user` block that starts the request. */
  request: z.string(),
  /** The file's path, as the Host names it. */
  file: z.string(),
  edit: requestEditRefSchema.nullable(),
})
export type RequestEditSelection = z.infer<typeof requestEditSelectionSchema>

/**
 * What a file pill under a call opens: the file `path` names in the call,
 * at the call's first edit of it, in the call's request, under the name it
 * has there now; null once a later call removed it.
 */
export function pillSelection(
  node: string | null,
  requests: TranscriptRequests,
  block: ToolCallBlock,
  path: string,
): RequestEditSelection | null {
  const request = requests.requestOf.get(block.id)
  const call = storedShellView(block)?.commandId
  for (const file of request?.files ?? []) {
    const first = file.edits.find((edit) => edit.call === call && edit.path === path)
    if (request && first) {
      return { node, request: request.id, file: file.path, edit: editRef(first) }
    }
  }
  return null
}

/** How a selection names `edit`. */
export function editRef(edit: RequestEdit): RequestEditRef {
  return { call: edit.call, path: edit.path, segment: edit.segment }
}

/** Whether two references name the same edit. */
export function sameEditRef(a: RequestEditRef | null, b: RequestEditRef | null): boolean {
  return a === b || (a !== null && b !== null && a.call === b.call && a.path === b.path && a.segment === b.segment)
}

/** Whether the file offers All Changes: its first edit's original and its last edit's result were both kept. */
export function offersAllChanges(file: RequestFile): boolean {
  return selectionCopies(file, null) !== null
}

/**
 * Where the edit the view shows stands among the file's edits; null for All
 * Changes. `edit` names one; null asks for All Changes, which a file that
 * does not offer it answers with its first edit with contents, or its first.
 */
export function editIndex(file: RequestFile, edit: RequestEditRef | null): number | null {
  const index = edit ? file.edits.findIndex((entry) => sameEditRef(editRef(entry), edit)) : -1
  if (index >= 0) {
    return index
  }
  if (offersAllChanges(file)) {
    return null
  }
  return Math.max(0, file.edits.findIndex((entry) => entry.copies !== undefined))
}

/**
 * The edit All Changes starts from: the first. A file that was there
 * before the request, and that a rename replaced with one an earlier call
 * created, starts from the first edit made to it, not from that creation.
 */
export function allChangesStart(file: RequestFile): RequestEdit | undefined {
  return file.kind === 'modified'
    ? file.edits.find((edit) => !edit.created) ?? file.edits[0]
    : file.edits[0]
}

/**
 * The two sides a selection shows: one edit's own, or for All Changes its
 * start's original and the last edit's result; null when a side was not
 * stored.
 */
export function selectionCopies(file: RequestFile, index: number | null): EditCopies | null {
  if (index !== null) {
    return file.edits[index]?.copies ?? null
  }
  const original = allChangesStart(file)?.copies?.original
  const modified = file.edits.at(-1)?.copies?.modified
  return original && modified ? { original, modified } : null
}

/**
 * The lines `file` added and removed across its edits, its All Changes as
 * the Change view's header counts it; null when its ends were not kept.
 */
export async function fileLineCounts(
  file: RequestFile,
  read: ReadCallChange,
  signal?: AbortSignal,
): Promise<{ added: number; removed: number } | null> {
  const copies = selectionCopies(file, null)
  if (!copies) {
    return null
  }
  const pair = await read(copies, signal)
  return pair ? diffLineCounts(pair.original, pair.modified) : null
}

/**
 * Whether two derivations of a request's files hold the same files and
 * edits. The transcript is derived anew on each frame of a turn, so the same
 * files come as new objects; a view compares them by what they hold and
 * keeps what it shows while a frame changes none of it (`plugin-pages.md`
 * § What the service keeps). They are plain data the derivation builds in
 * one order, so their JSON tells them apart, whatever fields they gain.
 */
export function sameRequestFiles(a: readonly RequestFile[], b: readonly RequestFile[]): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b)
}

/** Names files' ends, which their counts follow: they change only when a later call changes a file. */
export function filesEndsKey(files: readonly RequestFile[]): string {
  return files.map((file) => {
    const copies = selectionCopies(file, null)
    return copies ? `${copies.original}:${copies.modified}` : `${file.path}:-`
  }).join(',')
}

/** A request's files for the Change view, and how it reads an edit's two sides. */
export interface RequestChangeSource {
  /** Empty once the transcript no longer holds the request. */
  files: readonly RequestFile[]
  read: ReadCallChange
}
