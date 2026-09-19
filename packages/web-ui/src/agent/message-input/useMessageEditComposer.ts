import { computed, onScopeDispose, ref, shallowRef, toRaw, watch } from 'vue'
import {
  changeMessageEditContent,
  type MessageEditContent,
  type MessageEditState,
} from '../message-editing'
import { contentCapsule } from '../message-editor/capsules'
import { attachmentFileError, fileToUserContent } from './attachments'
import { joinMessageContent, splitMessageContent } from './message-content'

/**
 * Binds the composer's editor to a detached edit without touching its normal
 * draft: the message opens as it was sent, and every change is the edit's new
 * content, its text and files in the order shown.
 */
export function useMessageEditComposer(options: {
  state: () => MessageEditState | null | undefined
  update: (state: MessageEditState | null) => void
}) {
  const reading = shallowRef<AbortController | null>(null)
  const attachmentError = ref<string | null>(null)
  /** The edit's files by capsule id: each a media block, its attachment record, or both. */
  const files = shallowRef(new Map<string, MessageEditContent[]>())
  /** The Markdown the editor shows, a mark where each file stands. */
  const markdown = ref('')
  /** The capsule ids in the order of their marks. */
  const order = ref<string[]>([])
  /** The names of files added to the edit: their media blocks carry none. */
  const addedNames = shallowRef(new Map<string, string>())
  const editable = computed(() => options.state()?.phase === 'editing')
  const capsules = computed(() => order.value.map((id) => {
    const capsule = contentCapsule(id, files.value.get(id) ?? [])
    const name = addedNames.value.get(id)
    return name ? { ...capsule, name } : capsule
  }))

  function stopReading(): void {
    reading.value?.abort()
    reading.value = null
  }

  // A new edit opens its message; the content is copied, since the edit is persisted apart from the transcript.
  watch(() => options.state()?.request.operationId, () => {
    stopReading()
    attachmentError.value = null
    const content = structuredClone(toRaw(options.state()?.request.content ?? []))
    const split = splitMessageContent(content)
    const ids = split.files.map(() => crypto.randomUUID())
    files.value = new Map(ids.map((id, index) => [id, split.files[index] ?? []]))
    addedNames.value = new Map()
    markdown.value = split.markdown
    order.value = ids
  }, { immediate: true })
  onScopeDispose(stopReading)

  function change(nextMarkdown: string, ids: string[]): void {
    markdown.value = nextMarkdown
    order.value = ids
    const state = options.state()
    if (state) {
      const content = joinMessageContent(nextMarkdown, ids.map((id) => files.value.get(id) ?? []))
      options.update(changeMessageEditContent(state, (current) => {
        current.splice(0, current.length, ...content)
      }))
    }
  }

  function cancel(): void {
    if (editable.value) {
      options.update(null)
    }
  }

  /** Reads files into the edit; their capsules land where the editor was told they would. */
  async function addFiles(added: File[]): Promise<void> {
    const state = options.state()
    if (!state || !editable.value || reading.value || added.length === 0) {
      return
    }
    attachmentError.value = null
    const controller = new AbortController()
    reading.value = controller
    try {
      for (const file of added) {
        const error = attachmentFileError(file, [])
        if (error) {
          throw new Error(error)
        }
      }
      const blocks = await Promise.all(added.map((file) => fileToUserContent(file, { signal: controller.signal })))
      controller.signal.throwIfAborted()
      // An edit resends content inline; a file the model cannot read natively belongs to a new message, where it reaches the host.
      const unreadable = blocks.find((block) => block.type === 'document' && !isPdf(block.source.mediaType))
      if (unreadable && unreadable.type === 'document') {
        throw new Error(`${unreadable.source.fileName}: an edit can add images, videos and PDFs; attach other files to a new message.`)
      }
      if (options.state()?.request.operationId !== state.request.operationId) {
        return
      }
      const nextFiles = new Map(files.value)
      const nextNames = new Map(addedNames.value)
      const ids = blocks.map((block, index) => {
        const id = crypto.randomUUID()
        nextFiles.set(id, [block])
        nextNames.set(id, added[index]!.name)
        return id
      })
      files.value = nextFiles
      addedNames.value = nextNames
      order.value = [...order.value, ...ids]
    } catch (error) {
      if (!controller.signal.aborted) {
        attachmentError.value = error instanceof Error ? error.message : String(error)
      }
    } finally {
      controller.abort()
      if (reading.value === controller) {
        reading.value = null
      }
    }
  }

  return { markdown, capsules, editable, reading, attachmentError, change, cancel, addFiles }
}

function isPdf(mediaType: string): boolean {
  return mediaType.split(';')[0]?.trim() === 'application/pdf'
}
