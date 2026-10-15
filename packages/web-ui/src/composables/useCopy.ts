import { computed, ref } from 'vue'
import { useClipboardItems } from '@vueuse/core'
import { reportCopyFailure } from '../infra/clipboard'
import { t } from '../infra/locales/text'

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
        throw new Error(t('clipboardUnavailable'))
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

  /**
   * Puts the text `made` resolves to on the clipboard, written now, inside
   * the click that asks for it, as an item whose content is the promise:
   * Safari refuses a write that comes after the click's handler awaited
   * something, and every browser takes such an item. `copied` turns true
   * once the text is there. When `made` rejects, nothing is copied and
   * nothing is told, since the caller tells why it has no text.
   */
  async function copyWhenMade(made: Promise<string>): Promise<void> {
    try {
      if (!clipboard.isSupported.value)
        throw new Error(t('clipboardUnavailable'))
      const blob = made.then((text) => {
        copiedText.value = text
        return new Blob([text], { type: 'text/plain' })
      })
      // Its rejection is `made`'s, which the caller handles; the write
      // reports its own failure below.
      blob.catch(() => undefined)
      await clipboard.copy([new ClipboardItem({ 'text/plain': blob })])
    } catch (error) {
      copiedText.value = null
      // The text was never made: its maker tells why.
      if (await made.then(() => true, () => false))
        reportCopyFailure(error)
    }
  }

  return { copy, copyWhenMade, copied }
}
