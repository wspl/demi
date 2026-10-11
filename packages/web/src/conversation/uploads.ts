import {
  applyAttachmentUpdate,
  arrangeCapsuleFiles,
  attachmentFileError,
  AttachmentUploadQueue,
  composerAttachmentFromFile,
  composerFileNames,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { CopiedStoredFile } from '@demicodes/web-ui/agent/message-input/copied-message'
import { asError } from '@demicodes/utils'
import { blobUrl, claimStoredFile, uploadAttachment } from '../api/uploads'
import type { Conversation, ProductAttachment } from '../state/types'

/** The files of the messages sent and not yet confirmed, which the composer carries until they are. */
export function sendingFileIds(conversation: Pick<Conversation, 'pendingSends'>): Set<string> {
  return new Set(conversation.pendingSends.flatMap((pending) => pending.fileIds))
}

type ComposerFile = Extract<ProductAttachment, { kind: 'file' }>

/** Where an uploaded picture loads from, for a file whose bytes this web browser does not have. */
export function blobPicture(upload: { mediaType: string; sha256: string }): string | undefined {
  return upload.mediaType.startsWith('image/') ? blobUrl(upload.sha256, upload.mediaType) : undefined
}

/** Owns file transfers and preview resources for one product account. */
export function createConversationUploads(
  onChange: () => void,
  onError: (error: unknown) => void,
) {
  const uploads = new AttachmentUploadQueue()
  /** Each file's upload under way, by its id, which a message sent meanwhile waits for. */
  const running = new Map<string, Promise<void>>()

  /** Uploads a file's bytes; a file another page uploaded has no bytes here, and needs none. */
  function uploadFile(item: ComposerFile): Promise<void> {
    const bytes = item.file
    if (!bytes) {
      return Promise.resolve()
    }
    const upload = (async () => {
      const ready = await uploads.start(
        item.id,
        async (signal, report) => {
          const uploaded = await uploadAttachment(bytes, { signal, progress: report })
          item.upload = { id: uploaded.id, mediaType: uploaded.mediaType, sha256: uploaded.sha256 }
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
    })()
    running.set(item.id, upload)
    const done = () => {
      if (running.get(item.id) === upload) {
        running.delete(item.id)
      }
    }
    upload.then(done, done)
    return upload
  }

  /**
   * Waits until a file of a sent message is uploaded (`product.md` § Writing
   * a message): the upload under way, or a new one of a file whose upload
   * failed, as Retry asks. Rejects when the upload fails, or when the file
   * has no upload and no bytes to make one, so the message is not delivered
   * without it.
   */
  async function uploaded(item: ComposerFile): Promise<NonNullable<ComposerFile['upload']>> {
    if (!item.upload) {
      try {
        await (running.get(item.id) ?? uploadFile(item))
      } catch (error) {
        throw new Error(`${item.name} did not upload. ${asError(error).message}`)
      }
    }
    if (!item.upload) {
      throw new Error(`${item.name} is not uploaded. Attach it again.`)
    }
    return item.upload
  }

  /**
   * Takes a file a pasted message holds as a blob of the user's: the backend
   * makes an upload of it without its bytes. Answers the file the composer
   * then carries, or null when the user's blobs do not hold it.
   */
  async function claimFile(
    conversation: Conversation,
    file: CopiedStoredFile,
    signal: AbortSignal,
  ): Promise<ComposerFile | null> {
    const claimed = await claimStoredFile(file, signal)
    if (!claimed) {
      return null
    }
    const upload = { id: claimed.id, mediaType: claimed.mediaType, sha256: claimed.sha256 }
    const id = crypto.randomUUID()
    conversation.files.push({
      kind: 'file',
      id,
      name: file.name,
      src: blobPicture(upload),
      phase: 'ready',
      ...(claimed.snippet ? { snippet: claimed.snippet } : {}),
      file: null,
      upload,
    })
    onChange()
    return conversation.files.find((each): each is ComposerFile => each.id === id && isComposerFile(each)) ?? null
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
        void uploadFile(reactiveItem).catch((error: unknown) => {
          // A file of a message already sent tells its failure on the
          // message's bubble, with Retry; only the composer's own needs a toast.
          if (!sendingFileIds(conversation).has(reactiveItem.id)) {
            onError(error)
          }
        })
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
    const sending = sendingFileIds(conversation)
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

  /**
   * Lets go of the files no capsule stands for, which nothing can bring back
   * now; those of the messages still on their way stay with them.
   */
  function releaseSpare(conversation: Conversation): void {
    const has = new Set([...conversation.attachmentIds, ...sendingFileIds(conversation)])
    for (const file of conversation.files) {
      if (!has.has(file.id)) {
        releaseFile(file)
      }
    }
    conversation.files = conversation.files.filter((file) => has.has(file.id))
    onChange()
  }

  /**
   * Makes `files` the message's, in their order, beside the files of a send
   * not yet accepted, and lets every other file go: a draft shown from
   * outside takes the place of what the composer held, its undo included.
   */
  function showFiles(conversation: Conversation, files: ProductAttachment[]): void {
    const sending = sendingFileIds(conversation)
    const shown = new Set(files.map((file) => file.id))
    for (const file of conversation.files) {
      if (!shown.has(file.id) && !sending.has(file.id)) {
        releaseFile(file)
      }
    }
    conversation.files = [
      ...files,
      ...conversation.files.filter((file) => sending.has(file.id) && !shown.has(file.id)),
    ]
    conversation.attachmentIds = files.map((file) => file.id)
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
    uploaded,
    claimFile,
    addFiles,
    retryFile,
    removeFile,
    releaseSpare,
    arrangeFiles,
    showFiles,
    dispose,
  }
}
