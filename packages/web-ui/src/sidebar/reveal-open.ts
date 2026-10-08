import { watch } from 'vue'

/**
 * Keeps the open conversation's row in sight: once it is listed in a folded
 * project, as a project's new conversation is once its draft holds a
 * character or a file, or one opened from elsewhere is, that project
 * unfolds, as Finder reveals what it opens. It unfolds when the open
 * conversation or its project changes, so the user can fold the project
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
