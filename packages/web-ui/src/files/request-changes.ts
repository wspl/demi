import type { Block, ChangedFile, EditCopies, EditedFile, EditKind, FileEdit, LineCounts } from '@demicodes/protocol'
import type { HeldTranscript } from '@demicodes/conversation-client'
import { z } from 'zod'
import { storedShellView, toolCallTitle } from '../agent/block-helpers'
import type { ToolCallBlock } from '../agent/block-types'
import type { ChangeFile, ReadCallChange } from './changes'

/**
 * A request's changes (`crates/agent-store/src/changes.rs`): what one message of
 * the user asked for, the blocks that name its `user` block as their request,
 * and the files its calls changed, as the backend stored them with the
 * request's latest call (`requestFiles`).
 *
 * The page reads the lists it is given and counts nothing: a subagent's changes
 * are in its own transcript, not its parent's.
 */

/** One edit of a request's file: one segment of one of its calls. */
export interface RequestEdit extends FileEdit {
  /** The call's title, as its row in the transcript names it. */
  title: string
  /** The lines the call changed in the file, when the edit is its only one there. */
  lines?: LineCounts
}

/**
 * A file a request changed, as it stands after the request's latest call:
 * its kind, All Changes' lines, and its edits in the order they were made.
 */
export interface RequestFile extends Omit<ChangedFile, 'edits'> {
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
 * The requests of a window of one agent's transcript, each with the files
 * its latest call stored. Every block names the request it belongs to, its
 * own for a `user` block, so a window that starts inside a request, as a
 * compacted agent's does, finds that request's edits without its `user`
 * block; a block before the first `user` block names none.
 */
export function storedRequests(blocks: readonly Block[]): TranscriptRequests {
  const requests: TranscriptRequest[] = []
  const requestOf = new Map<string, TranscriptRequest>()
  const held = new Map<string, { request: TranscriptRequest; calls: Map<string, ToolCallBlock>; latest: ChangedFile[] }>()
  for (const block of blocks) {
    const id = block.request
    if (id === undefined)
      continue
    let current = held.get(id)
    if (!current) {
      current = { request: { id, files: [] }, calls: new Map(), latest: [] }
      held.set(id, current)
      requests.push(current.request)
    }
    requestOf.set(block.id, current.request)
    const view = block.type === 'tool_call' ? storedShellView(block) : null
    if (block.type === 'tool_call' && view) {
      current.calls.set(view.commandId, block)
      if (view.requestFiles)
        current.latest = view.requestFiles
    }
  }
  for (const { request, calls, latest } of held.values())
    request.files = latest.map((file) => requestFile(file, calls))
  return { requests, requestOf }
}

/** A stored file of a request, its edits named by their calls' titles. */
function requestFile(file: ChangedFile, calls: ReadonlyMap<string, ToolCallBlock>): RequestFile {
  return {
    ...file,
    edits: file.edits.map((edit) => {
      const call = calls.get(edit.call)
      const own = call ? storedShellView(call)?.files?.find((entry) => entry.path === edit.path) : undefined
      return {
        ...edit,
        title: call ? toolCallTitle(call) : '',
        ...(own?.edits.length === 1 ? { lines: { added: own.added, removed: own.removed } } : {}),
      }
    }),
  }
}

/** A stored file as a pill or a list row shows it; a file whose ends were not kept is named alone. */
export function changeFile(file: ChangedFile | EditedFile): ChangeFile {
  const lines = 'lines' in file ? file.lines : 'added' in file ? { added: file.added, removed: file.removed } : undefined
  return {
    path: file.path,
    kind: file.kind,
    ...(file.from ? { from: file.from } : {}),
    added: lines?.added ?? 0,
    removed: lines?.removed ?? 0,
  }
}

/** The files a shell call's pills show: those it changed, but outside its agent's working directory. */
export function callPills(block: ToolCallBlock): ChangeFile[] {
  return (storedShellView(block)?.files ?? []).filter((file) => !file.outside).map(changeFile)
}

/**
 * The files a run's folded pills show: those its latest call stored,
 * counted from before the run to after that call.
 */
export function runFiles(calls: readonly ToolCallBlock[]): ChangedFile[] {
  for (const call of [...calls].reverse()) {
    const files = storedShellView(call)?.runFiles
    if (files)
      return files
  }
  return []
}

/** The transcripts of one conversation: its own agent's, and each subagent's by its id. */
export interface ConversationTranscripts {
  history: HeldTranscript
  subagents: readonly { id: string; history: HeldTranscript }[]
}

/** The request `request` of the agent `node` (null for the conversation's own), as its transcript holds it now. */
export function findRequest(
  transcripts: ConversationTranscripts,
  node: string | null,
  request: string,
): TranscriptRequest | null {
  const history = node === null
    ? transcripts.history
    : transcripts.subagents.find((agent) => agent.id === node)?.history
  // The window that holds its latest call holds what it stored of the
  // request's files; its `user` block may stand in a page before it.
  const window = history?.windows.findLast((each) => each.blocks.some((block) => block.request === request))
  if (!window) {
    return null
  }
  return storedRequests(window.blocks).requests.find((entry) => entry.id === request) ?? null
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
 * What the `edit` intent opens: a request of
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
  /**
   * The directory the agent worked in when it wrote the call, which the view
   * roots its tree at; absent for the conversation's own agent, which works in
   * the conversation's.
   */
  root: z.string().optional(),
})
export type RequestEditSelection = z.infer<typeof requestEditSelectionSchema>

/**
 * What a file pill opens: the file of the request whose edits hold the call
 * `call`'s edit of `path`, at that edit, under the name the file has there now.
 *
 * It is null once a later call removed the file, or for a file the call only
 * removed or renamed. `root` is the directory the call's agent worked in, null for
 * the conversation's own.
 */
export function pillSelection(
  node: string | null,
  requests: TranscriptRequests,
  block: ToolCallBlock,
  call: string,
  path: string,
  root: string | null,
): RequestEditSelection | null {
  const request = requests.requestOf.get(block.id)
  for (const file of request?.files ?? []) {
    if (file.kind === 'deleted')
      continue
    const first = file.edits.find((edit) => edit.call === call && edit.path === path)
    if (request && first)
      return { node, request: request.id, file: file.path, edit: editRef(first), ...(root === null ? {} : { root }) }
  }
  return null
}

/**
 * What the "+N Files" pill opens for the file `path` of the call or run
 * (`crates/agent-store/src/changes.rs`): the file of the request whose list holds
 * it, at All Changes.
 *
 * So it works for a file the request removed too, which a pill alone is no control
 * for. It is null for a file the request does not list.
 */
export function requestFileSelection(
  node: string | null,
  requests: TranscriptRequests,
  block: ToolCallBlock,
  path: string,
  root: string | null,
): RequestEditSelection | null {
  const request = requests.requestOf.get(block.id)
  return request?.files.some((file) => file.path === path)
    ? { node, request: request.id, file: path, edit: null, ...(root === null ? {} : { root }) }
    : null
}

/** How a selection names `edit`. */
export function editRef(edit: FileEdit): RequestEditRef {
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
  return file.kind === 'added'
    ? file.edits[0]
    : file.edits.find((edit) => !edit.created) ?? file.edits[0]
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
 * Whether a selection shows no sides because the file is binary, which edit
 * tracking never keeps: the edit's own mark, or for All Changes any edit's, as
 * the backend recorded why.
 */
export function selectionIsBinary(file: RequestFile, index: number | null): boolean {
  return index !== null ? file.edits[index]?.binary === true : file.edits.some((edit) => edit.binary === true)
}

/**
 * Whether two readings of a request's files hold the same files and edits.
 *
 * The request is read anew on each frame of a turn, so the same files come as new
 * objects. A view compares them by what they hold and keeps what it shows while a
 * frame changes none of it. They are plain data in the order the backend stored
 * them, so their JSON tells them apart, whatever fields they gain.
 */
export function sameRequestFiles(a: readonly RequestFile[], b: readonly RequestFile[]): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b)
}

/** A request's files for the Change view, and how it reads an edit's two sides. */
export interface RequestChangeSource {
  /** Empty once the transcript no longer holds the request. */
  files: readonly RequestFile[]
  read: ReadCallChange
}
