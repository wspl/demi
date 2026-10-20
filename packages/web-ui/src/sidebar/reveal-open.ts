import { watch } from 'vue'

/**
 * Keeps the open conversation's row in sight, as Finder reveals what it opens.
 *
 * Once the open conversation is listed in a folded project, that project
 * unfolds. That happens when a project's new conversation has a draft with a
 * character or a file, and when one is opened from elsewhere. It unfolds when the
 * open conversation or its project changes, so the user can fold the project
 * again while it stays open.
 */
export function useRevealOpen(options: {
  /** The project the open conversation is listed in; null for none or a plain one. */
  openProject: () => string | null
  isFolded: (projectId: string) => boolean
  unfold: (projectId: string) => void
}): void {
  watch(
    options.openProject,
    (projectId) => {
      if (projectId !== null && options.isFolded(projectId))
        options.unfold(projectId)
    },
    { immediate: true },
  )
}
