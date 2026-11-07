import { watch } from 'vue'
import { clamp } from '@demicodes/utils'
import { TREE_WIDTH } from '@demicodes/web-ui/files/file-view'
import { treeLayout } from '@demicodes/web-ui/files/tree-layout'

const STORAGE_KEY = 'demi-gallery.file-tree'

/**
 * Keeps whether the file tree shows, and its width, across visits: the
 * product keeps them in the user's preferences, which every device reads; the
 * gallery keeps them in this browser.
 */
export function keepTreeLayout(): void {
  try {
    const kept = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null') as { open?: unknown; width?: unknown } | null
    if (typeof kept?.open === 'boolean') {
      treeLayout.open.value = kept.open
    }
    if (typeof kept?.width === 'number' && Number.isFinite(kept.width)) {
      treeLayout.width.value = clamp(kept.width, TREE_WIDTH.min, TREE_WIDTH.max)
    }
  } catch {
    // A value this gallery did not write starts from the defaults.
  }
  watch([treeLayout.open, treeLayout.width], ([open, width]) => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ open, width: Math.round(width) }))
  })
}
