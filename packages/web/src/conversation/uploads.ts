import { z } from 'zod'
import {
  applyAttachmentUpdate,
  attachmentFileError,
  AttachmentUploadQueue,
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
  const uploads = new AttachmentUploadQueue()
  async function uploadFile(
    conversation: Conversation,
    item: Extract<ProductAttachment, { kind: 'file' }>,
  ): Promise<void> {
    const ready = await uploads.start(
      item.id,
      async (signal, report) => {
        const result = await uploadBytes(
          '/attachments',
          item.file,
          signal,
          report,
          item.file.type || 'application/octet-stream',
        )
        const attachment = z
          .object({ attachment: z.object({ id: z.string() }) })
          .parse(result).attachment
        item.upload = { id: attachment.id }
      },
      (update) => applyAttachmentUpdate(item, update),
    ).catch((error: unknown) => {
      // A failed upload leaves the composer; the caller toasts the reason.
      removeFile(conversation, item.id)
      throw error
    })
    if (ready)
      onChange()
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
    uploads.cancel(id)
    const item = conversation.files.find((item) => item.id === id)
    if (item && isComposerFile(item) && item.src) {
      URL.revokeObjectURL(item.src)
    }
    conversation.files = conversation.files.filter((item) => item.id !== id)
    onChange()
  }

  function dispose(conversations: Conversation[]): void {
    uploads.cancelAll()
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
