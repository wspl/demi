import { t } from './locales/text'
import type { CloudState } from './types'

/** What each step of a Cloud reset says while it runs, and how it ends. */
export const resetPhaseLabels: Record<NonNullable<CloudState['phase']>, string> = {
  get stopping() {
    return t('phase.stopping')
  },
  get saving() {
    return t('phase.saving')
  },
  get rebuilding() {
    return t('phase.rebuilding')
  },
  get booting() {
    return t('phase.booting')
  },
  get ready() {
    return t('phase.ready')
  },
  get failed() {
    return t('phase.failed')
  },
}
