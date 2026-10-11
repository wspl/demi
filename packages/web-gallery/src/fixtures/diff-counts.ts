import { Text } from '@codemirror/state'
import { diffChunks } from '@demicodes/web-ui/editor/unifiedDiff'

/**
 * The lines a diff from `original` to `modified` adds and removes, for the
 * gallery's working tree, which plays the Host's git status: the page
 * counts no line itself, and shows the counts its sources give.
 */
export function diffLineCounts(original: string, modified: string): { added: number; removed: number } {
  const a = Text.of(original.split('\n'))
  const b = Text.of(modified.split('\n'))
  let added = 0
  let removed = 0
  for (const chunk of diffChunks(a, b)) {
    if (chunk.toA > chunk.fromA)
      removed += a.lineAt(chunk.endA).number - a.lineAt(chunk.fromA).number + 1
    if (chunk.toB > chunk.fromB)
      added += b.lineAt(chunk.endB).number - b.lineAt(chunk.fromB).number + 1
  }
  return { added, removed }
}
