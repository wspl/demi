import { watch } from 'vue'
import { treeLayout } from '@demicodes/web-ui/files/tree-layout'
import { usePreferences } from '../state/preferences'

/**
 * Keeps the file tree's choice, shown or hidden and its width, in the user's
 * preferences, which every device reads: the views take it as the preferences
 * bring it, and the user's change goes back, once a drag of its divider
 * pauses.
 */
export function keepFileTree(): void {
  const preferences = usePreferences()
  watch(() => preferences.fileTree, (kept) => {
    if (kept) {
      treeLayout.open.value = kept.shown
      treeLayout.width.value = kept.width
    }
  }, { immediate: true })
  watch([treeLayout.open, treeLayout.width], ([shown, width]) => {
    const kept = preferences.fileTree
    const rounded = Math.round(width)
    if (kept?.shown !== shown || kept.width !== rounded) {
      preferences.update({ fileTree: { shown, width: rounded } }, { bursty: true })
    }
  })
}
