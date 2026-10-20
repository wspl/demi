import type { Block, InstructionEntry } from '@demicodes/protocol'
import { baseName, parentPath, relativePath } from '../files/paths'
import { t } from './locales/text'

/**
 * What the model holds of the user's and the project's instructions: the
 * entries of the newest instructions block of `blocks`, in its order, and
 * none once the newest says nothing is left. Without one, `before`: the
 * entries a page of the transcript named for a block the page does not
 * hold, none before the first.
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
  /**
   * What the row says: `Personal instructions`; a project file's path from
   * the outermost file's directory; or `Workflow: Review Team` for the
   * conversation's workflow.
   */
  label: string
  /** The full path of a file, the workflow's included; none for the personal instructions. */
  path: string | null
}

/**
 * The rows for `entries`, each project file named from the directory of
 * the outermost one, as `AGENTS.md` and `web/CLAUDE.md`, and the workflow
 * by its name.
 */
export function instructionRows(entries: readonly InstructionEntry[]): InstructionRow[] {
  let base: string | null = null
  for (const entry of entries) {
    if (entry.kind === 'file') {
      base = parentPath(entry.path)
      break
    }
  }
  return entries.map((entry) => {
    switch (entry.kind) {
      case 'personal':
        return { entry, label: t('context.personalInstructions'), path: null }
      case 'workflow':
        return { entry, label: t('context.workflowInstructions', { name: entry.name }), path: entry.path }
      case 'file': {
        const label = base === null ? baseName(entry.path) : relativePath(base, entry.path)
        return { entry, label, path: entry.path }
      }
    }
  })
}
