import { z } from 'zod'
import {
  attachmentFileError,
  attachTextSnippet,
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
    item.progress = 0
    try {
      const result = await uploadBytes(
        '/attachments',
        item.file,
        controller.signal,
        (fraction) => {
          item.progress = fraction
        },
        item.file.type || 'application/octet-stream',
      )
      controller.signal.throwIfAborted()
      const attachment = z
        .object({ attachment: z.object({ id: z.string() }) })
        .parse(result).attachment
      item.upload = { id: attachment.id }
      item.phase = 'ready'
      item.progress = undefined
      onChange()
    } catch (error) {
      // A failed upload leaves the composer; the caller toasts the reason.
      if (!controller.signal.aborted) {
        removeFile(conversation, item.id)
        throw error
      }
    } finally {
      if (uploads.get(item.id) === controller) {
        uploads.delete(item.id)
      }
    }
  }

  function addFiles(conversation: Conversation, files: File[]): void {
    for (const file of files) {
      const error = attachmentFileError(file, composerFileNames(conversation.files))
      if (error) {
        onError(new Error(error))
        continue
      }
      const item: Extract<ProductAttachment, { kind: 'file' }> = {
        ...composerAttachmentFromFile(file),
        file,
        upload: null,
      }
      conversation.files.push(item)
      const reactiveItem = conversation.files.find(
        (attachment) => attachment.id === item.id,
      )!
      if (isComposerFile(reactiveItem)) {
        void attachTextSnippet(reactiveItem, file)
        void uploadFile(conversation, reactiveItem).catch(onError)
      }
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
    removeFile,
    dispose,
  }
}
