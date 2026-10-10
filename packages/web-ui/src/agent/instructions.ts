import type { Block, InstructionEntry } from '@demicodes/protocol'
import { baseName, parentPath, relativePath } from '../files/paths'

/**
 * What the model holds of the user's and the project's instructions: the
 * entries of the newest instructions block of `blocks`, in its order
 * (`instructions.md` § What the card lists), and none once the newest says
 * nothing is left. Without one, `before`: the entries a page of the
 * transcript named for a block the page does not hold, none before the
 * first.
 */
export function loadedInstructions(
  blocks: readonly Block[],
  before: readonly InstructionEntry[] = [],
): readonly InstructionEntry[] {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]
    if (block.type === 'context' && block.source === 'instructions') {
      return block.instructions ?? []
    }
  }
  return before
}

/** One row of the context card's Instructions. */
export interface InstructionRow {
  entry: InstructionEntry
  /** What the row says: `Personal instructions`, or the file's path from the outermost file's directory. */
  label: string
  /** The full path of a file; none for the personal instructions. */
  path: string | null
}

/** The rows for `entries`, each file named from the directory of the outermost one, as `AGENTS.md` and `web/CLAUDE.md`. */
export function instructionRows(entries: readonly InstructionEntry[]): InstructionRow[] {
  let base: string | null = null
  for (const entry of entries) {
    if (entry.kind !== 'personal') {
      base = parentPath(entry.path)
      break
    }
  }
  return entries.map((entry) => {
    if (entry.kind === 'personal') {
      return { entry, label: 'Personal instructions', path: null }
    }
    const label = base === null ? baseName(entry.path) : relativePath(base, entry.path)
    return { entry, label, path: entry.path }
  })
}
