import { computed, onScopeDispose, shallowRef, watch, type ComputedRef } from 'vue'
import { fileLineCounts, selectionCopies, type RequestFile } from '../files/request-changes'
import { useEditReads } from './edit-selection'

export interface LineCount {
  added: number
  removed: number
}

/** A file's two ends, which its count follows: they change only when a later call changes it. */
function endsOf(file: RequestFile): string | null {
  const copies = selectionCopies(file, null)
  return copies ? `${copies.original}:${copies.modified}` : null
}

/**
 * Each of `files`' lines added and removed, its All Changes, read once
 * `shown` says the place that tells them is in view (`edit-tracking.md`
 * § What the conversation shows). By path: undefined until the file's
 * current ends are counted, null when they were not kept or could not be
 * read. A count follows its file's ends, so a file a later call changed is
 * counted anew while the others keep theirs, and counts of replaced ends
 * never show.
 */
export function useFileLineCounts(
  files: () => readonly RequestFile[],
  shown: () => boolean,
): ComputedRef<ReadonlyMap<string, LineCount | null | undefined>> {
  const read = useEditReads()
  /** Counts by ends, which never change once read. */
  const counted = shallowRef(new Map<string, LineCount | null>())
  const controller = new AbortController()

  watch([shown, () => files().map(endsOf).join(',')], ([visible]) => {
    const reader = read()
    if (!visible || !reader)
      return
    for (const file of files()) {
      const ends = endsOf(file)
      if (ends === null || counted.value.has(ends))
        continue
      // Reserve the ends, so a second change before the read lands reads them once.
      counted.value = new Map(counted.value).set(ends, null)
      fileLineCounts(file, reader, controller.signal).then(
        (count) => {
          counted.value = new Map(counted.value).set(ends, count)
        },
        () => {
          // Safe to ignore: an abort is the page leaving, and a read that
          // failed leaves the file named alone, as the design has it for ends
          // it cannot count; the Change view reports a failed read itself.
        },
      )
    }
  }, { immediate: true })

  onScopeDispose(() => controller.abort())

  return computed(() => new Map(files().map((file) => {
    const ends = endsOf(file)
    return [file.path, ends === null ? null : counted.value.get(ends)]
  })))
}
