import { Chunk } from '@codemirror/merge'
import { Text } from '@codemirror/state'

/**
 * The lines a diff from `original` to `modified` adds and removes, as the
 * diff editor's own chunks cover them, so the counts a header shows are
 * those of the diff under it.
 */
export function diffLineCounts(original: string, modified: string): { added: number; removed: number } {
  const a = Text.of(original.split('\n'))
  const b = Text.of(modified.split('\n'))
  let added = 0
  let removed = 0
  for (const chunk of Chunk.build(a, b)) {
    if (chunk.toA > chunk.fromA)
      removed += a.lineAt(chunk.endA).number - a.lineAt(chunk.fromA).number + 1
    if (chunk.toB > chunk.fromB)
      added += b.lineAt(chunk.endB).number - b.lineAt(chunk.fromB).number + 1
  }
  return { added, removed }
}
