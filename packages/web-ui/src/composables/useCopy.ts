import { computed, ref } from 'vue'
import { useClipboardItems } from '@vueuse/core'
import { reportCopyFailure } from '../infra/clipboard'

/** How long a control shows Copied after a copy. */
export const COPIED_MS = 1500

/**
 * What a copy puts on the clipboard: text, or an item that carries more than
 * text (a message with its files). Nothing, when there is nothing to copy.
 */
export type CopySource = string | ClipboardItem | undefined

/**
 * A copy for a control: `copy()` puts what `source` gives at that moment on
 * the clipboard and `copied` stays true for `COPIED_MS` after it, while the
 * copied text is still what the control would copy again. A copy that fails
 * is told to the user (`reportCopyFailure`); `copy()` never rejects.
 *
 * Text goes as a clipboard item too, since `useClipboard` (the same library)
 * falls back to a command that cannot tell whether it worked and shows
 * Copied either way; `useClipboardItems` rejects when the write fails.
 */
export function useCopy(source: () => CopySource) {
  const clipboard = useClipboardItems({ copiedDuring: COPIED_MS })
  const copiedText = ref<string | null>(null)

  const copied = computed(() => clipboard.copied.value && (copiedText.value === null || copiedText.value === source()))

  async function copy(): Promise<void> {
    try {
      const value = source()
      if (value === undefined)
        return
      if (!clipboard.isSupported.value)
        throw new Error('The clipboard is not available here.')
      copiedText.value = typeof value === 'string' ? value : null
      const item = typeof value === 'string'
        ? new ClipboardItem({ 'text/plain': new Blob([value], { type: 'text/plain' }) })
        : value
      await clipboard.copy([item])
    } catch (error) {
      copiedText.value = null
      reportCopyFailure(error)
    }
  }

  return { copy, copied }
}
