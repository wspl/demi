import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  attachmentFileError,
  AttachmentUploadQueue,
  composerAttachmentFromFile,
  composerFileNames,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { uploadAttachment } from '../api/uploads'
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
        const uploaded = await uploadAttachment(item.file, { signal, progress: report })
        item.upload = { id: uploaded.id }
        // The capsule shows the opening the backend read, as the message will carry it.
        item.snippet = uploaded.snippet
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

  /**
   * Takes files for a conversation's message and answers with what it took,
   * for the composer to put their capsules where they belong in the text.
   * Each starts uploading at once; its capsule shows what the answer says.
   */
  function addFiles(conversation: Conversation, files: File[]): ProductAttachment[] {
    const taken: ProductAttachment[] = []
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
        void uploadFile(reactiveItem).catch(onError)
      }
      taken.push(reactiveItem)
    }
    onChange()
    return taken
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
    conversation.attachmentIds = conversation.attachmentIds.filter((each) => each !== id)
    onChange()
  }

  /**
   * The files the message now has, in the order of its capsules; the rest the
   * composer goes on carrying, in case an undo brings their capsules back
   * (`arrangeCapsuleFiles`). The files of a message being sent are not its.
   */
  function arrangeFiles(conversation: Conversation, ids: readonly string[]): void {
    const sending = new Set(conversation.pendingSend?.fileIds ?? [])
    const next = arrangeCapsuleFiles(
      conversation.files,
      conversation.attachmentIds,
      ids,
      (item) => sending.has(item.id),
    )
    conversation.attachmentIds = [...ids]
    conversation.files = next.carried
    for (const item of next.stopped) {
      uploads.cancel(item.id)
    }
    // A file whose upload a deleted capsule stopped starts over; one still uploading goes on.
    for (const item of next.resumed) {
      if (item.kind === 'file' && item.phase !== 'ready' && !uploads.has(item.id)) {
        void uploadFile(item).catch(onError)
      }
    }
    onChange()
  }

  /** Lets go of the files no capsule stands for: nothing can bring them back now. */
  function releaseSpare(conversation: Conversation): void {
    const has = new Set(conversation.attachmentIds)
    for (const file of conversation.files) {
      if (!has.has(file.id)) {
        releaseFile(file)
      }
    }
    conversation.files = conversation.files.filter((file) => has.has(file.id))
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
    releaseSpare,
    arrangeFiles,
    dispose,
  }
}
