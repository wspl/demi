import type { StyleValue } from 'vue'

/** A 28px chrome row arriving in a transcript: it slides in from the left while fading in. */
export const CHROME_ENTER_MS = 200

/** The class and duration a row's wrapper binds while it enters; nothing otherwise. */
export function chromeEntrance(entering: boolean): { class: string; style: StyleValue } | undefined {
  if (!entering) {
    return undefined
  }
  return {
    class: 'chrome-enter',
    style: { '--chrome-enter-ms': `${CHROME_ENTER_MS}ms` },
  }
}
