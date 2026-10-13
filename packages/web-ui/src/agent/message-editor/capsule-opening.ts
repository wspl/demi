import { computed, type ComputedRef } from 'vue'
import { useMessageFiles } from '../../markdown/message-files'

/**
 * What a click on a capsule in the conversation does: a file on the
 * conversation's Host opens through the `file` intent, where a plugin opens
 * it. `path` is where the file is, none for a file the Host does not hold;
 * a composer opens nothing. One owner for the capsules of attached files and
 * of the images a message names.
 */
export function useCapsuleOpening(editable: () => boolean, path: () => string | null | undefined): {
  opens: ComputedRef<string | null>
  open(): void
} {
  const files = useMessageFiles()
  const opens = computed(() => {
    const at = path()
    return !editable() && at && files()?.open ? at : null
  })
  return {
    opens,
    open() {
      if (opens.value) {
        files()?.open?.({ path: opens.value, line: null })
      }
    },
  }
}
