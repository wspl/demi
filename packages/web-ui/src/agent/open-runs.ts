import { reactive, watch } from 'vue'
import type { MessageListBlock } from './pending-steers'
import { stepHolds } from './work-groups'

/**
 * Where a block brought into view shows: `row`, the list's row that holds
 * it, and `step`, the block itself when it is one step of an open run's
 * stack, whose own row is then brought into view and marked; null when the
 * row is the block's own, as a run of one step is.
 */
export interface RevealTarget {
  row: string
  step: string | null
}

export interface OpenRuns {
  /** Whether the run whose row the list draws under `key` is open. */
  isOpen: (key: string) => boolean
  setOpen: (key: string, open: boolean) => void
  /**
   * Where the block `id` shows, opening the run that folds it, as a tree
   * opens the folders above a search result (`shell-tool.md` § Command
   * reports); the run stays open after, as the reader's own opening leaves
   * it. Null when the list does not hold the block.
   */
  reveal: (id: string) => RevealTarget | null
}

/**
 * The runs of a transcript list that are open, by the keys the list draws
 * their rows under (`keyOf`, `rendering.md` § Work groups): the row a lone
 * step ran in is the run's row once the step ended, so a step opened while
 * it runs is still open as the run it starts. The list owns them rather than each run's
 * row, so a reveal opens a run whatever the row's state, and a run stays as
 * the reader left it when its row scrolls out of the drawn range and back.
 * Another `scope`, the list showing another transcript, starts folded.
 */
export function useOpenRuns(
  rows: () => readonly MessageListBlock[],
  scope: () => string,
  keyOf: (row: MessageListBlock) => string,
): OpenRuns {
  const open = reactive(new Set<string>())
  watch(scope, () => open.clear())

  function setOpen(run: string, value: boolean): void {
    if (value) {
      open.add(run)
    } else {
      open.delete(run)
    }
  }

  function reveal(id: string): RevealTarget | null {
    for (const row of rows()) {
      if (row.type === 'work_group') {
        const step = row.steps.find((each) => stepHolds(each, id))
        if (!step) {
          continue
        }
        // A run of one step is that step's own row: it is marked as it is.
        if (row.steps.length === 1) {
          return { row: row.id, step: null }
        }
        open.add(keyOf(row))
        return { row: row.id, step: step.id }
      }
      if (row.id === id) {
        return { row: id, step: null }
      }
    }
    return null
  }

  return { isOpen: (run) => open.has(run), setOpen, reveal }
}
