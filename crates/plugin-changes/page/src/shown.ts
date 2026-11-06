import { computed, type ComputedRef } from 'vue'
import type { ConversationFileService, Repositories } from '@demicodes/plugin-sdk'
import { followsPreferred, viewDirectory, type ChangeData } from './data'

/** The working tree the Change view shows, which its tab and its badge share. */
export interface ShownDirectory {
  /** The repositories Commits offers, null until the device answered. */
  repositories: ComputedRef<Repositories | null>
  /**
   * The directory shown, undefined for the conversation's; null while Commits
   * waits for the repositories to say which one it shows, and reads nothing.
   */
  directory: ComputedRef<string | undefined | null>
}

/**
 * The directory a link, a request or the user's pick names, else in Commits
 * the repository the agents changed a file in last, the conversation's
 * without one.
 */
export function useShownDirectory(files: ConversationFileService, data: () => ChangeData): ShownDirectory {
  files.showRepositories()
  const repositories = computed(() => files.repositories())
  const directory = computed(() => {
    const shown = data()
    if (shown.mode !== 'commits' || !followsPreferred(shown))
      return viewDirectory(shown)
    const preferred = repositories.value?.preferred
    if (preferred === undefined)
      return null
    return preferred === null || preferred === files.root ? undefined : preferred
  })
  return { repositories, directory }
}
