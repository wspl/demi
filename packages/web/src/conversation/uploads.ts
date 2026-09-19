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
    item: Extract<ProductAttachment, { kind: 'file' }>,
  ): Promise<void> {
    const ready = await uploads.start(
      item.id,
      async (signal, report) => {
        const result = await uploadBytes('/attachments', item.file, {
          signal,
          progress: (sent) => report(sent / item.file.size),
        })
        const attachment = z
          .object({ attachment: z.object({ id: z.string() }) })
          .parse(result).attachment
        item.upload = { id: attachment.id }
      },
      (update) => applyAttachmentUpdate(item, update),
    ).catch((error: unknown) => {
      // A failed upload keeps its capsule, which offers Retry; the caller toasts the reason.
      applyAttachmentUpdate(item, { phase: 'failed' })
      onChange()
      throw error
    })
    if (ready)
      onChange()
  }

  /** Uploads a file that failed again. */
  function retryFile(conversation: Conversation, id: string): void {
    const item = conversation.files.find((file) => file.id === id)
    if (item && isComposerFile(item) && item.phase === 'failed') {
      void uploadFile(item).catch(onError)
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
        void uploadFile(reactiveItem).catch(onError)
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

  /**
   * Keeps the draft's files to its capsules, in their order: a file whose
   * capsule left the text is removed. The files of a message still being
   * sent are not the draft's.
   */
  function arrangeFiles(conversation: Conversation, ids: readonly string[]): void {
    const sending = new Set(conversation.pendingSend?.fileIds ?? [])
    for (const file of conversation.files) {
      if (!sending.has(file.id) && !ids.includes(file.id))
        removeFile(conversation, file.id)
    }
    conversation.files = [
      ...conversation.files.filter((file) => sending.has(file.id)),
      ...ids.flatMap((id) => conversation.files.filter((file) => file.id === id)),
    ]
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
    retryFile,
    removeFile,
    arrangeFiles,
    dispose,
  }
}
