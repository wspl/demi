import { z } from 'zod'
import {
  applyAttachmentUpdate,
  attachmentFileError,
  AttachmentUploadQueue,
  arrangeCapsuleFiles,
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
  /** Files whose capsule was deleted, by conversation: they wait there for an undo. */
  const aside = new Map<string, ProductAttachment[]>()
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

  /** Lets a file go for good: its transfer stops and its picture is freed. */
  function releaseFile(item: ProductAttachment): void {
    uploads.cancel(item.id)
    if (isComposerFile(item) && item.src) {
      URL.revokeObjectURL(item.src)
    }
  }

  function removeFile(conversation: Conversation, id: string): void {
    const item = conversation.files.find((item) => item.id === id)
    if (item) {
      releaseFile(item)
    }
    conversation.files = conversation.files.filter((item) => item.id !== id)
    onChange()
  }

  /** Drops what a conversation set aside; from here nothing can bring those files back. */
  function releaseAside(conversation: Conversation): void {
    for (const item of aside.get(conversation.id) ?? []) {
      releaseFile(item)
    }
    aside.delete(conversation.id)
  }

  /**
   * Keeps the draft's files to its capsules, in their order. A file whose
   * capsule left the text is set aside, so undoing the deletion brings both
   * back, and a capsule that comes back takes its file from there. The files
   * of a message still being sent are not the draft's.
   */
  function arrangeFiles(conversation: Conversation, ids: readonly string[]): void {
    const sending = new Set(conversation.pendingSend?.fileIds ?? [])
    const next = arrangeCapsuleFiles(
      conversation.files,
      aside.get(conversation.id) ?? [],
      ids,
      (item) => sending.has(item.id),
    )
    // A file set aside stops uploading; coming back, it uploads again from the start.
    for (const item of next.detached) {
      uploads.cancel(item.id)
    }
    conversation.files = next.files
    if (next.aside.length) {
      aside.set(conversation.id, next.aside)
    } else {
      aside.delete(conversation.id)
    }
    for (const item of next.restored) {
      if (isComposerFile(item) && item.phase !== 'ready') {
        void uploadFile(item).catch(onError)
      }
    }
    onChange()
  }

  function dispose(conversations: Conversation[]): void {
    uploads.cancelAll()
    for (const conversation of conversations) {
      for (const item of [...conversation.files, ...aside.get(conversation.id) ?? []]) {
        if (isComposerFile(item) && item.src) {
          URL.revokeObjectURL(item.src)
        }
      }
    }
    aside.clear()
  }
  return {
    uploadFile,
    addFiles,
    retryFile,
    removeFile,
    releaseAside,
    arrangeFiles,
    dispose,
  }
}
