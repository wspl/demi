import type { EditorState } from '@codemirror/state'
import { syntaxTree } from '@codemirror/language'

export interface StickyStructureItem {
  from: number
  to: number
  headerLineNumber: number
}

export function getStickyStructureStack(state: EditorState, topPos: number): StickyStructureItem[] {
  const tree = syntaxTree(state)
  const seenHeaderLines = new Set<number>()
  const items: StickyStructureItem[] = []
  const topLine = state.doc.lineAt(topPos)
  const firstContentOffset = topLine.text.search(/\S/)
  const probePos = Math.min(
    topLine.to,
    topLine.from + Math.max(0, firstContentOffset >= 0 ? firstContentOffset : 0),
  )
  let node: ReturnType<typeof tree.resolveInner> | null = tree.resolveInner(probePos, 1)

  while (node) {
    const fromLine = state.doc.lineAt(node.from)
    const toLine = state.doc.lineAt(node.to)
    const isWholeDocumentRoot = !node.parent && node.from === 0 && node.to === state.doc.length

    if (!isWholeDocumentRoot && toLine.number > fromLine.number && !seenHeaderLines.has(fromLine.number)) {
      seenHeaderLines.add(fromLine.number)
      items.push({
        from: node.from,
        to: node.to,
        headerLineNumber: fromLine.number,
      })
    }

    node = node.parent
  }

  return items.reverse()
}
