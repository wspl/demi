import { z } from 'zod'
import { sniffModelMediaType } from '@demicodes/core'
import {
  attachmentFileError,
  composerAttachmentFromFile,
  composerFileNames,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { uploadBytes } from '../api/uploads'
import type { Conversation, ProductAttachment } from '../state/types'

/** Owns file transfers and preview resources for one product account. */
export function createConversationUploads(
  onChange: () => void,
  onError: (error: unknown) => void,
) {
  const uploads = new Map<string, AbortController>()
  async function uploadFile(
    conversation: Conversation,
    item: Extract<ProductAttachment, { kind: 'file' }>,
  ): Promise<void> {
    const controller = new AbortController()
    uploads.get(item.id)?.abort()
    uploads.set(item.id, controller)
    item.phase = 'uploading'
    item.error = undefined
    item.progress = 0
    const contextVersion = conversation.contextVersion
    try {
      const bytes = new Uint8Array(await item.file.slice(0, 32).arrayBuffer())
      const media = sniffModelMediaType(bytes)
      const pdf = new TextDecoder().decode(bytes.slice(0, 5)) === '%PDF-'
      if (item.destination === 'message' && !media && !pdf) {
        throw new Error(`${item.name} is not a supported image, video or PDF.`)
      }
      const path =
        item.destination === 'message'
          ? '/attachments'
          : `/conversations/${encodeURIComponent(conversation.id)}/workspace-files?name=${encodeURIComponent(item.name)}`
      const result = await uploadBytes(
        path,
        item.file,
        controller.signal,
        (fraction) => {
          item.progress = fraction
        },
        media?.mediaType ??
          (pdf ? 'application/pdf' : item.file.type || 'application/octet-stream'),
      )
      controller.signal.throwIfAborted()
      if (item.destination === 'message') {
        const attachment = z
          .object({ attachment: z.object({ id: z.string() }) })
          .parse(result).attachment
        item.upload = {
          kind: 'message',
          id: attachment.id,
          media: media?.kind ?? 'document',
        }
      } else {
        item.upload = {
          kind: 'workspace',
          path: z.object({ path: z.string() }).parse(result).path,
          contextVersion,
        }
      }
      controller.signal.throwIfAborted()
      item.phase = 'ready'
      item.progress = undefined
      onChange()
    } catch (error) {
      if (!controller.signal.aborted) {
        item.phase = 'failed'
        item.progress = undefined
        item.error = error instanceof Error ? error.message : String(error)
        throw error
      }
    } finally {
      if (uploads.get(item.id) === controller) {
        uploads.delete(item.id)
      }
    }
  }

  function addFiles(
    conversation: Conversation,
    files: File[],
    acceptedExtensions: readonly string[] | null,
  ): void {
    for (const file of files) {
      const error = attachmentFileError(file, composerFileNames(conversation.files))
      if (error) {
        onError(new Error(error))
        continue
      }
      const item: Extract<ProductAttachment, { kind: 'file' }> = {
        ...composerAttachmentFromFile(file, acceptedExtensions),
        file,
        upload: null,
      }
      conversation.files.push(item)
      const reactiveItem = conversation.files.find(
        (attachment) => attachment.id === item.id,
      )!
      if (isComposerFile(reactiveItem)) {
        void uploadFile(conversation, reactiveItem).catch(onError)
      }
    }
  }

  function retryFile(conversation: Conversation, id: string): void {
    const item = conversation.files.find((file) => file.id === id)
    if (item && isComposerFile(item) && item.phase === 'failed') {
      void uploadFile(conversation, item).catch(onError)
    }
  }

  function removeFile(conversation: Conversation, id: string): void {
    uploads.get(id)?.abort()
    uploads.delete(id)
    const item = conversation.files.find((item) => item.id === id)
    if (item && isComposerFile(item) && item.src) {
      URL.revokeObjectURL(item.src)
    }
    conversation.files = conversation.files.filter((item) => item.id !== id)
    onChange()
  }

  function dispose(conversations: Conversation[]): void {
    for (const controller of uploads.values()) {
      controller.abort()
    }
    uploads.clear()
    for (const conversation of conversations) {
      for (const item of conversation.files) {
        if (isComposerFile(item) && item.src) {
          URL.revokeObjectURL(item.src)
        }
      }
    }
  }
  return {
    uploadFile,
    addFiles,
    retryFile,
    removeFile,
    dispose,
  }
}
