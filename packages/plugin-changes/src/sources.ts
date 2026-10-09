import { computed, type ComputedRef } from 'vue'
import { sameRequestFiles, type ChangeSources, type ConversationFileService, type RequestFile } from '@demicodes/plugin-sdk'
import type { ChangeData } from './data'

const NO_FILES: readonly RequestFile[] = []

/**
 * What the Change view shows: the working tree's changes, and in
 * Conversation the request the tab names, as its agent's transcript holds it
 * now. Each frame of a turn derives the transcript anew: the sources stay
 * the same value while a frame changes nothing of them, so the view does not
 * render for it (`plugin-pages.md` § What the service keeps).
 */
export function useChangeSources(files: ConversationFileService, data: () => ChangeData): ComputedRef<ChangeSources> {
  const requestFiles = computed<readonly RequestFile[] | null>((previous) => {
    const shown = data().request
    if (!shown)
      return null
    const next = files.request(shown.node, shown.request)?.files ?? NO_FILES
    return previous && sameRequestFiles(previous, next) ? previous : next
  })
  return computed(() => ({
    uncommitted: files.changes,
    conversation: requestFiles.value ? { files: requestFiles.value, read: files.edit } : null,
  }))
}
