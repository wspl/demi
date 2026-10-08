import type { Block, InstructionEntry } from '@demicodes/protocol'
import { baseName, parentPath, relativePath } from '../files/paths'

/**
 * What the model holds of the user's and the project's instructions: the
 * entries of the newest instructions block of `blocks`, in its order
 * (`instructions.md` § What the card lists); none before the first, and
 * none once the newest says nothing is left.
 */
export function loadedInstructions(blocks: readonly Block[]): InstructionEntry[] {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]
    if (block.type === 'context' && block.source === 'instructions') {
      return block.instructions ?? []
    }
  }
  return []
}

/** One row of the context card's Instructions. */
export interface InstructionRow {
  entry: InstructionEntry
  /** What the row says: `Personal instructions`, or the file's path from the outermost file's directory. */
  label: string
  /** The full path of a file; none for the personal instructions. */
  path: string | null
  /** Whether the file is too large to include, which the row marks. */
  tooLarge: boolean
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
      return { entry, label: 'Personal instructions', path: null, tooLarge: false }
    }
    const label = base === null ? baseName(entry.path) : relativePath(base, entry.path)
    return { entry, label, path: entry.path, tooLarge: entry.kind === 'too_large' }
  })
}
