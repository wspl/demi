import { Facet, Text } from '@codemirror/state'
import { ViewPlugin, type PluginValue } from '@codemirror/view'
import { getSearchQuery, setSearchQuery } from '@codemirror/search'
import { getChunks } from '@codemirror/merge'
import { mountScrollbarDom } from './dom'
import { getResolvedDiagnostics } from '../lsp/diagnostics'
import { buildScrollbarMarkers } from './markers'

export const scrollbarDiffOriginalDoc = Facet.define<Text | null, Text | null>({
  combine(values) {
    return values[0] ?? null
  },
})

function collectSearchMatches(view: import('@codemirror/view').EditorView, mapLine?: (line: number) => number) {
  const query = getSearchQuery(view.state)
  if (!query.valid || !query.search) return []

  const matches: Array<{ fromLine: number; toLine: number }> = []
  const cursor = query.getCursor(view.state.doc)
  let result = cursor.next()

  while (!result.done) {
    matches.push({
      fromLine: mapLine?.(view.state.doc.lineAt(result.value.from).number) ?? view.state.doc.lineAt(result.value.from).number,
      toLine: mapLine?.(view.state.doc.lineAt(result.value.to).number) ?? view.state.doc.lineAt(result.value.to).number,
    })
    result = cursor.next()
  }

  return matches
}

function countLinesInRange(doc: Text, from: number, to: number) {
  if (to <= from) return 0
  const safeFrom = Math.max(0, Math.min(doc.length, from))
  const safeTo = Math.max(safeFrom, Math.min(doc.length, to))
  const startLine = doc.lineAt(safeFrom).number
  const endLine = doc.lineAt(Math.max(safeFrom, safeTo - 1)).number
  return endLine - startLine + 1
}

function createUnifiedLineMapper(view: import('@codemirror/view').EditorView) {
  const chunkState = getChunks(view.state)
  const originalDoc = view.state.facet(scrollbarDiffOriginalDoc)
  if (!chunkState || !originalDoc) return null

  const modifiedDoc = view.state.doc
  let deletedLinesBefore = 0
  const offsets: Array<{ startLine: number; deletedLines: number }> = []
  const diffs: Array<{ fromLine: number; toLine: number; kind: 'added' | 'deleted' }> = []

  for (const chunk of chunkState.chunks) {
    const startLine = modifiedDoc.lineAt(Math.min(modifiedDoc.length, Math.max(0, chunk.fromB))).number
    const addedLines = countLinesInRange(modifiedDoc, chunk.fromB, chunk.endB)
    const deletedLines = countLinesInRange(originalDoc, chunk.fromA, chunk.endA)
    const visualBase = startLine + deletedLinesBefore

    if (deletedLines > 0) {
      diffs.push({
        fromLine: visualBase,
        toLine: visualBase + deletedLines - 1,
        kind: 'deleted',
      })
      offsets.push({ startLine, deletedLines })
      deletedLinesBefore += deletedLines
    }

    if (addedLines > 0) {
      diffs.push({
        fromLine: visualBase + deletedLines,
        toLine: visualBase + deletedLines + addedLines - 1,
        kind: 'added',
      })
    }
  }

  const mapLine = (line: number) => {
    let offset = 0
    for (const entry of offsets) {
      if (entry.startLine <= line) offset += entry.deletedLines
    }
    return line + offset
  }

  return {
    totalLines: modifiedDoc.lines + deletedLinesBefore,
    diffs,
    mapLine,
  }
}

function collectDiffChunks(view: import('@codemirror/view').EditorView) {
  const layout = createUnifiedLineMapper(view)
  if (!layout) return []
  return layout.diffs
}

function collectSelections(view: import('@codemirror/view').EditorView, mapLine?: (line: number) => number) {
  return view.state.selection.ranges
    .filter((range) => !range.empty)
    .map((range) => {
      const from = Math.min(range.from, range.to)
      const to = Math.max(range.from, range.to)
      const safeTo = Math.max(from, to - 1)
      return {
        fromLine: mapLine?.(view.state.doc.lineAt(from).number) ?? view.state.doc.lineAt(from).number,
        toLine: mapLine?.(view.state.doc.lineAt(safeTo).number) ?? view.state.doc.lineAt(safeTo).number,
      }
    })
}

class CustomScrollbarPlugin implements PluginValue {
  private mounted
  private frame = 0
  private readonly view: import('@codemirror/view').EditorView
  private readonly onScroll: () => void
  private lastScrollTop: number
  private lastScrollLeft: number

  constructor(view: import('@codemirror/view').EditorView) {
    this.view = view
    this.mounted = mountScrollbarDom(view, {
      splitDiffLanes: !!view.state.facet(scrollbarDiffOriginalDoc),
    })
    this.lastScrollTop = view.scrollDOM.scrollTop
    this.lastScrollLeft = view.scrollDOM.scrollLeft
    this.onScroll = () => {
      const nextTop = view.scrollDOM.scrollTop
      const nextLeft = view.scrollDOM.scrollLeft
      this.mounted.setScrollActivity({
        vertical: nextTop !== this.lastScrollTop,
        horizontal: nextLeft !== this.lastScrollLeft,
      })
      this.lastScrollTop = nextTop
      this.lastScrollLeft = nextLeft
      this.schedule(view)
    }
    view.scrollDOM.addEventListener('scroll', this.onScroll, { passive: true })
    this.schedule(view)
  }

  update(update: import('@codemirror/view').ViewUpdate) {
    const searchChanged = update.transactions.some((transaction) =>
      transaction.effects.some((effect) => effect.is(setSearchQuery)))
    if (
      update.docChanged
      || update.geometryChanged
      || update.viewportChanged
      || update.selectionSet
      || searchChanged
      || update.transactions.length > 0
    ) {
      this.schedule(update.view)
    }
  }

  private schedule(view: import('@codemirror/view').EditorView) {
    if (this.frame) return
    this.frame = requestAnimationFrame(() => {
      this.frame = 0
      const diffLayout = createUnifiedLineMapper(view)
      const diagnostics = getResolvedDiagnostics(view).map((item) => ({
        fromLine: diffLayout?.mapLine(view.state.doc.lineAt(item.from).number) ?? view.state.doc.lineAt(item.from).number,
        toLine: diffLayout?.mapLine(view.state.doc.lineAt(item.to).number) ?? view.state.doc.lineAt(item.to).number,
        severity: item.severity,
      }))
      const searches = collectSearchMatches(view, diffLayout?.mapLine)
      const diffs = collectDiffChunks(view)
      const selections = collectSelections(view, diffLayout?.mapLine)
      this.mounted.update(view, buildScrollbarMarkers({
        totalLines: diffLayout?.totalLines ?? view.state.doc.lines,
        diagnostics,
        searches,
        selections,
        diffs,
      }))
      this.lastScrollTop = view.scrollDOM.scrollTop
      this.lastScrollLeft = view.scrollDOM.scrollLeft
    })
  }

  destroy() {
    if (this.frame) cancelAnimationFrame(this.frame)
    this.view.scrollDOM.removeEventListener('scroll', this.onScroll)
    this.mounted.destroy()
  }
}

export function customScrollbarExtension() {
  return ViewPlugin.fromClass(CustomScrollbarPlugin)
}
