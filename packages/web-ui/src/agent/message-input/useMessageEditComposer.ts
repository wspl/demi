import { computed, onScopeDispose, ref, shallowRef, toRaw, watch } from 'vue'
import {
  changeMessageEditContent,
  type MessageEditContent,
  type MessageEditState,
} from '../message-editing'
import {
  contentCapsule,
  type MessageCapsule,
  type MessageTransfer,
  type TransferContext,
} from '../message-editor/capsules'
import {
  AttachmentUploadQueue,
  attachmentFileError,
  attachmentSendBlockReason,
  filePreviewUrl,
  type UploadFile,
} from './attachments'
import { joinMessageContent, splitMessageContent } from './message-content'

/** A file the edit added: its bytes, its capsule, and its upload while it is not done. */
interface AddedFile {
  file: File
  capsule: MessageCapsule
  transfer: MessageTransfer | null
}

/**
 * Binds the composer's editor to a detached edit without touching its normal
 * draft: the message opens as it was sent, and every change is the edit's new
 * content, its text and files in the order shown. A file added to the edit is
 * uploaded the way the main composer uploads one (`message-editing.md`
 * § The editor) and joins the edit's content once its upload is done; a
 * deleted capsule stops its upload, and an undo starts it over.
 */
export function useMessageEditComposer(options: {
  state: () => MessageEditState | null | undefined
  update: (state: MessageEditState | null) => void
  upload: UploadFile
}) {
  const uploads = new AttachmentUploadQueue()
  const attachmentError = ref<string | null>(null)
  /** The edit's files by capsule id: the message's records and media, or an added file's upload once done. */
  const files = shallowRef(new Map<string, MessageEditContent[]>())
  /** The files added to the edit, by capsule id. */
  const added = shallowRef(new Map<string, AddedFile>())
  /** The Markdown the editor shows, a mark where each file stands. */
  const markdown = ref('')
  /** The capsule ids in the order of their marks. */
  const order = ref<string[]>([])
  const editable = computed(() => options.state()?.phase === 'editing')
  const capsules = computed(() => order.value.map((id) => capsuleOf(id)))
  /** Why the edit cannot go yet: an upload of its own still running, or one that failed. */
  const sendBlockReason = computed(() => attachmentSendBlockReason(
    order.value.flatMap((id) => added.value.get(id)?.transfer?.phase ?? []),
  ))

  /** A file of the edit as its capsule shows it; an added file keeps the picture it was added with. */
  function capsuleOf(id: string): MessageCapsule {
    const blocks = files.value.get(id)
    const file = added.value.get(id)
    if (!blocks) {
      return file?.capsule ?? { id, name: 'file' }
    }
    const capsule = contentCapsule(id, blocks)
    return file?.capsule.image ? { ...capsule, image: file.capsule.image } : capsule
  }

  /** Ends what the edit had going: its uploads, and the pictures of its added files. */
  function release(): void {
    uploads.cancelAll()
    for (const file of added.value.values()) {
      if (typeof file.capsule.image === 'string') {
        URL.revokeObjectURL(file.capsule.image)
      }
    }
    added.value = new Map()
  }

  // A new edit opens its message; the content is copied, since the edit is persisted apart from the transcript.
  watch(() => options.state()?.request.operationId, () => {
    release()
    attachmentError.value = null
    const content = structuredClone(toRaw(options.state()?.request.content ?? []))
    const split = splitMessageContent(content)
    const ids = split.files.map(() => crypto.randomUUID())
    files.value = new Map(ids.map((id, index) => [id, split.files[index] ?? []]))
    markdown.value = split.markdown
    order.value = ids
  }, { immediate: true })
  onScopeDispose(release)

  /** Writes the edit's content as the editor shows it: its text, and each file that is ready to go. */
  function publish(): void {
    const state = options.state()
    if (!state) {
      return
    }
    const content = joinMessageContent(markdown.value, order.value.map((id) => files.value.get(id) ?? []))
    options.update(changeMessageEditContent(state, (current) => {
      current.splice(0, current.length, ...content)
    }))
  }

  function change(nextMarkdown: string, attachments: MessageCapsule[]): void {
    const ids = attachments.map((capsule) => capsule.id)
    const now = new Set(ids)
    for (const id of order.value) {
      if (!now.has(id)) {
        uploads.cancel(id)
      }
    }
    // A file whose upload a deleted capsule stopped starts over; one still uploading goes on.
    for (const id of ids) {
      if (added.value.get(id)?.transfer && !uploads.has(id)) {
        void uploadFile(id)
      }
    }
    markdown.value = nextMarkdown
    order.value = ids
    publish()
  }

  function cancel(): void {
    if (editable.value) {
      options.update(null)
    }
  }

  function setTransfer(id: string, transfer: MessageTransfer | null): void {
    const file = added.value.get(id)
    if (file) {
      added.value = new Map(added.value).set(id, { ...file, transfer })
    }
  }

  /** Uploads one added file; once the backend has it, the file is part of the edit. */
  async function uploadFile(id: string): Promise<void> {
    const file = added.value.get(id)?.file
    const operationId = options.state()?.request.operationId
    if (!file) {
      return
    }
    try {
      const ready = await uploads.start(
        id,
        async (signal, report) => {
          const uploaded = await options.upload(file, { signal, progress: report })
          signal.throwIfAborted()
          files.value = new Map(files.value).set(id, [{
            type: 'upload',
            ref: uploaded.id,
            fileName: file.name,
            mediaType: uploaded.mediaType,
            sha256: uploaded.sha256,
            ...(uploaded.snippet ? { snippet: uploaded.snippet } : {}),
          }])
        },
        (update) => setTransfer(id, update.phase === 'ready'
          ? null
          : update.phase === 'uploading'
            ? { phase: 'uploading', progress: update.progress ?? 0 }
            : { phase: 'failed' }),
      )
      if (ready && options.state()?.request.operationId === operationId) {
        publish()
      }
    } catch (error) {
      // A failed upload keeps its capsule, which offers Retry; the composer toasts the reason.
      setTransfer(id, { phase: 'failed' })
      attachmentError.value = `${file.name}: ${error instanceof Error ? error.message : String(error)}`
    }
  }

  /** Takes files into the edit and answers with their capsules, for the editor to put in the message. */
  function addFiles(picked: File[]): MessageCapsule[] {
    if (!options.state() || !editable.value) {
      return []
    }
    attachmentError.value = null
    const names = capsules.value.map((capsule) => capsule.name)
    const taken: MessageCapsule[] = []
    for (const file of picked) {
      const error = attachmentFileError(file, names)
      if (error) {
        attachmentError.value = error
        continue
      }
      names.push(file.name)
      const id = crypto.randomUUID()
      const image = filePreviewUrl(file)
      const capsule: MessageCapsule = { id, name: file.name, ...(image ? { image } : {}) }
      added.value = new Map(added.value).set(id, { file, capsule, transfer: { phase: 'uploading', progress: 0 } })
      taken.push(capsule)
      void uploadFile(id)
    }
    return taken
  }

  /** Whether the capsule is one of this edit's files. */
  function carries(id: string): boolean {
    return files.value.has(id) || added.value.has(id)
  }

  /** The uploads of the edit's added files, which their capsules read. */
  const transfers: TransferContext = {
    transfer: (id) => added.value.get(id)?.transfer ?? undefined,
    carries,
    retry: (id) => {
      if (added.value.get(id)?.transfer?.phase === 'failed') {
        void uploadFile(id)
      }
    },
    current: (id) => (carries(id) ? capsuleOf(id) : undefined),
  }

  return {
    markdown,
    capsules,
    editable,
    sendBlockReason,
    attachmentError,
    transfers,
    change,
    cancel,
    addFiles,
  }
}
