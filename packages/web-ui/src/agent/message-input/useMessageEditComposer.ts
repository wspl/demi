import { computed, nextTick, onScopeDispose, ref, shallowRef, watch } from 'vue'
import type { Ref } from 'vue'
import {
  changeMessageEditContent,
  type MessageEditState,
} from '../message-editing'
import { attachmentFileError, fileMatchesAcceptedExtensions, fileToUserContent } from './attachments'

/** Binds the existing composer to a detached edit without touching its normal draft. */
export function useMessageEditComposer(options: {
  state: () => MessageEditState | null | undefined
  update: (state: MessageEditState | null) => void
  root: Ref<HTMLElement | undefined>
  acceptedExtensions: () => readonly string[] | null
}) {
  const reading = shallowRef<AbortController | null>(null)
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
    options.root.value?.querySelector('textarea')?.focus()
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
        if (!fileMatchesAcceptedExtensions(file, options.acceptedExtensions())) {
          throw new Error(`${file.name}: this model does not accept this file type.`)
        }
      }
      const added = await Promise.all(files.map((file) => fileToUserContent(file, { signal: controller.signal })))
      controller.signal.throwIfAborted()
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
