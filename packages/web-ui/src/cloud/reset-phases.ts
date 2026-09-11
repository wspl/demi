import type { CloudState } from './types'

/** What each step of a Cloud reset says while it runs, and how it ends. */
export const resetPhaseLabels: Record<NonNullable<CloudState['phase']>, string> = {
  stopping: 'Stopping Cloud tasks…',
  saving: 'Saving home files…',
  rebuilding: 'Rebuilding the system…',
  booting: 'Starting Cloud…',
  ready: 'Cloud is ready.',
  failed: 'Reset failed.',
}
