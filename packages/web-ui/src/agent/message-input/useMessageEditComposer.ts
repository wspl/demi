import { computed, nextTick, onScopeDispose, ref, shallowRef, watch } from 'vue'
import type { Ref } from 'vue'
import { useAutofocus } from '../../ui/autofocus'
import {
  changeMessageEditContent,
  type MessageEditState,
} from '../message-editing'
import { attachmentFileError, fileToUserContent } from './attachments'

/** Binds the existing composer to a detached edit without touching its normal draft. */
export function useMessageEditComposer(options: {
  state: () => MessageEditState | null | undefined
  update: (state: MessageEditState | null) => void
  root: Ref<HTMLElement | undefined>
}) {
  const reading = shallowRef<AbortController | null>(null)
  const autofocus = useAutofocus()
  const attachmentError = ref<string | null>(null)
  const parts = computed(() => options.state()?.request.content.map((part, index) => ({ part, index })) ?? [])
  const textParts = computed(() => parts.value.filter((item) => item.part.type === 'text'))
  const attachments = computed(() => parts.value.filter((item) => item.part.type !== 'text'))
  const editable = computed(() => options.state()?.phase === 'editing')

  function stopReading(): void {
    reading.value?.abort()
    reading.value = null
  }

  watch(() => options.state()?.request.operationId, async (operationId, previous) => {
    stopReading()
    attachmentError.value = null
    if (!operationId && !previous) {
      return
    }
    await nextTick()
    autofocus(options.root.value?.querySelector('textarea'))
  }, { immediate: true })
  onScopeDispose(stopReading)

  function changeText(index: number, value: string): void {
    const state = options.state()
    if (state) {
      options.update(changeMessageEditContent(state, (content) => {
        const part = content[index]
        if (part?.type === 'text') {
          part.text = value
        }
      }))
    }
  }

  function removeAttachment(index: number): void {
    const state = options.state()
    if (state) {
      options.update(changeMessageEditContent(state, (content) => {
        content.splice(index, 1)
      }))
    }
  }

  function cancel(): void {
    if (editable.value) {
      options.update(null)
    }
  }

  async function addFiles(files: File[]): Promise<void> {
    const state = options.state()
    if (!state || !editable.value || reading.value || files.length === 0) {
      return
    }
    attachmentError.value = null
    const controller = new AbortController()
    reading.value = controller
    try {
      for (const file of files) {
        const error = attachmentFileError(file, [])
        if (error) {
          throw new Error(error)
        }
      }
      const added = await Promise.all(files.map((file) => fileToUserContent(file, { signal: controller.signal })))
      controller.signal.throwIfAborted()
      // An edit resends content inline; a file the model cannot read natively belongs to a new message, where it reaches the host.
      const unreadable = added.find((block) => block.type === 'document' && !isPdf(block.source.mediaType))
      if (unreadable && unreadable.type === 'document') {
        throw new Error(`${unreadable.source.fileName}: an edit can add images, videos and PDFs; attach other files to a new message.`)
      }
      const current = options.state()
      if (current?.request.operationId === state.request.operationId) {
        options.update(changeMessageEditContent(current, (content) => {
          content.push(...added)
        }))
      }
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

  return { textParts, attachments, editable, reading, attachmentError, changeText, removeAttachment, cancel, addFiles }
}

function isPdf(mediaType: string): boolean {
  return mediaType.split(';')[0]?.trim() === 'application/pdf'
}
