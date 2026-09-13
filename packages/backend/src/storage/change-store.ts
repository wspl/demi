import { shellEditsViewSchema } from '@demicodes/agent'
import type { Block } from '@demicodes/core'
import { decodeUtf8Strict, isRecord } from '@demicodes/utils'
import { EDIT_FILE_BYTES } from '@demicodes/command-protocol'
import type { JobExitMessage } from '@demicodes/runner-protocol'
import type { Host, ShellEditedFile } from '@demicodes/shell'
import type { ChangeObjects } from './change-objects'

/** Conversation-owned historical file contents, published before their block. */
export class ChangeStore {
  constructor(private readonly objects: ChangeObjects) {}

  async retain(
    conversationId: string,
    commandId: string,
    host: Host,
    files: JobExitMessage['files'],
  ): Promise<ShellEditedFile[]> {
    const retained: ShellEditedFile[] = []
    for (const [fileIndex, file] of files.entries()) {
      const edits: ShellEditedFile['edits'] = []
      for (const [editIndex, edit] of file.edits.entries()) {
        let kept = false
        if (edit.modified) {
          try {
            const original = edit.original ? await host.fs.readFile(edit.original) : new Uint8Array()
            const modified = await host.fs.readFile(edit.modified)
            text(modified)
            text(original)
            await this.objects.put(key(conversationId, commandId, fileIndex, editIndex, 'original'), original)
            await this.objects.put(key(conversationId, commandId, fileIndex, editIndex, 'modified'), modified)
            kept = true
          } catch (error) {
            // File metadata survives a failed snapshot transfer or publication.
            console.error('Could not retain file edit', { conversationId, commandId, path: file.path, error })
          }
        }
        edits.push({ kept })
      }
      retained.push({ path: file.path, kind: file.kind, added: file.added, removed: file.removed, edits })
    }
    return retained
  }

  /** Forked blocks keep command identities; their contents become independently owned. */
  async fork(sourceId: string, destinationId: string, blocks: readonly Block[]): Promise<void> {
    const copied = new Set<string>()
    for (const block of blocks) {
      if (block.type !== 'tool_call' || !isRecord(block.view) || block.view['kind'] !== 'shell' || block.view['files'] === undefined) {
        continue
      }
      const call = shellEditsViewSchema.parse(block.view)
      if (copied.has(call.commandId)) {
        continue
      }
      copied.add(call.commandId)
      for (const [fileIndex, file] of call.files.entries()) {
        for (const [editIndex, edit] of file.edits.entries()) {
          if (!edit.kept) {
            continue
          }
          for (const side of ['original', 'modified'] as const) {
            const bytes = await this.objects.get(key(sourceId, call.commandId, fileIndex, editIndex, side))
            // A missing historical object remains unavailable in both conversations.
            if (bytes) {
              await this.objects.put(key(destinationId, call.commandId, fileIndex, editIndex, side), bytes)
            }
          }
        }
      }
    }
  }

  async read(
    conversationId: string,
    commandId: string,
    files: readonly ShellEditedFile[],
    path: string,
    edit: number,
  ): Promise<{ original: string; modified: string } | null> {
    const file = files.findIndex(file => file.path === path)
    if (file < 0 || !files[file]?.edits[edit]?.kept) {
      return null
    }
    const original = await this.objects.get(key(conversationId, commandId, file, edit, 'original'))
    const modified = await this.objects.get(key(conversationId, commandId, file, edit, 'modified'))
    if (!original || !modified) {
      return null
    }
    return { original: text(original), modified: text(modified) }
  }
}

function key(conversation: string, command: string, file: number, edit: number, side: 'original' | 'modified'): string {
  if (![conversation, command].every(value => /^[A-Za-z0-9_-]+$/.test(value))) {
    throw new Error('Invalid edit history identity')
  }
  return `changes/${conversation}/${command}/${file}/${edit}.${side}`
}

function text(bytes: Uint8Array): string {
  if (bytes.byteLength > EDIT_FILE_BYTES || bytes.includes(0)) {
    throw new Error('Invalid text snapshot')
  }
  const decoded = decodeUtf8Strict(bytes)
  if (decoded === null) {
    throw new Error('Invalid UTF-8 snapshot')
  }
  return decoded
}
