import type { Block, EditCopies } from '@demicodes/protocol'
import { z } from 'zod'
import { storedShellView, toolCallTitle } from '../agent/block-helpers'
import type { ToolCallBlock } from '../agent/block-types'
import type { ReadCallChange } from './changes'

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
  /** Which of the call's segments of the file it is. */
  segment: number
  /** The segment created the file, so nothing came before it. */
  created: boolean
  /** The file's two sides; absent when they were not stored. */
  copies?: EditCopies
}

/** A file a request changed, with its edits in the order they were made. */
export interface RequestFile {
  /** As the Host names it. */
  path: string
  /** `added` when the request created it. */
  kind: 'added' | 'modified'
  /** The lines the request's calls added and removed, summed over its calls. */
  added: number
  removed: number
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
  let current: { request: TranscriptRequest; byPath: Map<string, RequestFile> } | null = null
  for (const block of blocks) {
    if (block.type === 'user') {
      current = { request: { id: block.id, files: [] }, byPath: new Map() }
      requests.push(current.request)
    }
    if (!current) {
      continue
    }
    requestOf.set(block.id, current.request)
    if (block.type === 'tool_call') {
      addCall(current.request, current.byPath, block)
    }
  }
  return { requests, requestOf }
}

function addCall(request: TranscriptRequest, byPath: Map<string, RequestFile>, block: ToolCallBlock): void {
  const view = storedShellView(block)
  if (!view?.files) {
    return
  }
  const title = toolCallTitle(block)
  for (const file of view.files) {
    let entry = byPath.get(file.path)
    if (!entry) {
      entry = { path: file.path, kind: file.kind, added: 0, removed: 0, edits: [] }
      byPath.set(file.path, entry)
      request.files.push(entry)
    }
    entry.added += file.added
    entry.removed += file.removed
    file.edits.forEach((edit, segment) => {
      entry.edits.push({
        call: view.commandId,
        title,
        segment,
        created: file.kind === 'added' && segment === 0,
        ...(edit.copies ? { copies: edit.copies } : {}),
      })
    })
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

/** One edit of a request's file, by its call and segment, which stay as later calls add edits. */
export const requestEditRefSchema = z.object({
  call: z.string(),
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

/** What a request's line opens: its first file's All Changes. */
export function requestLineSelection(node: string | null, request: TranscriptRequest): RequestEditSelection | null {
  const first = request.files[0]
  return first ? { node, request: request.id, file: first.path, edit: null } : null
}

/** What a file pill under a call opens: the file at that call's first edit, in the call's request. */
export function pillSelection(
  node: string | null,
  requests: TranscriptRequests,
  block: ToolCallBlock,
  path: string,
): RequestEditSelection | null {
  const request = requests.requestOf.get(block.id)
  const call = storedShellView(block)?.commandId
  const file = request?.files.find((entry) => entry.path === path)
  const first = file?.edits.find((edit) => edit.call === call)
  if (!request || !first) {
    return null
  }
  return { node, request: request.id, file: path, edit: { call: first.call, segment: first.segment } }
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
  const index = edit ? file.edits.findIndex((entry) => entry.call === edit.call && entry.segment === edit.segment) : -1
  if (index >= 0) {
    return index
  }
  if (offersAllChanges(file)) {
    return null
  }
  return Math.max(0, file.edits.findIndex((entry) => entry.copies !== undefined))
}

/**
 * The two sides a selection shows: one edit's own, or for All Changes the
 * first edit's original and the last edit's result; null when a side was
 * not stored.
 */
export function selectionCopies(file: RequestFile, index: number | null): EditCopies | null {
  if (index !== null) {
    return file.edits[index]?.copies ?? null
  }
  const original = file.edits[0]?.copies?.original
  const modified = file.edits.at(-1)?.copies?.modified
  return original && modified ? { original, modified } : null
}

/** A request's files for the Change view, and how it reads an edit's two sides. */
export interface RequestChangeSource {
  /** Empty once the transcript no longer holds the request. */
  files: readonly RequestFile[]
  read: ReadCallChange
}
