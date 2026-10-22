import { reactive, watch } from 'vue'
import type { MessageListBlock } from './pending-steers'
import { stepHolds, workRowId } from './work-groups'

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
  /** Whether the run `row` shows, or the step it shows alone, is open. */
  isOpen: (row: MessageListBlock) => boolean
  setOpen: (row: MessageListBlock, open: boolean) => void
  /**
   * Where the block `id` shows, opening the run that folds it, as a tree opens
   * the folders above a search result (`crates/agent-session/src/reports.rs`);
   * the run stays open after, as the reader's own opening leaves it. Null when
   * the list does not hold the block.
   */
  reveal: (id: string) => RevealTarget | null
}

/**
 * The runs of a transcript list that are open, by the names of the blocks their
 * rows show (`rowKey`), never by the keys the list draws the rows under: a row
 * that takes the place of another takes its key (`succeedKeys`), and must not
 * take its open state with it. A step that runs alone is named as a step of its
 * own (`workRowId`) and, once it ended, as the run it started, so a run is open
 * when its first step was opened while it ran: it stays open as the run it
 * starts.
 *
 * The list owns them rather than each run's row, so a reveal opens a run
 * whatever the row's state, and a run stays as the reader left it when its row
 * scrolls out of the drawn range and back. Another `scope`, the list showing
 * another transcript, starts folded.
 */
export function useOpenRuns(
  rows: () => readonly MessageListBlock[],
  scope: () => string,
  keyOf: (row: MessageListBlock) => string,
): OpenRuns {
  const open = reactive(new Set<string>())
  watch(scope, () => open.clear())

  /** The names a row's open state is kept under: its own, and its first step's own row's. */
  function names(row: MessageListBlock): string[] {
    return row.type === 'work_group' ? [keyOf(row), workRowId(row.steps[0]!)] : [keyOf(row)]
  }

  function isOpen(row: MessageListBlock): boolean {
    return names(row).some((name) => open.has(name))
  }

  function setOpen(row: MessageListBlock, value: boolean): void {
    if (value) {
      open.add(keyOf(row))
    } else {
      for (const name of names(row)) {
        open.delete(name)
      }
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

  return { isOpen, setOpen, reveal }
}
