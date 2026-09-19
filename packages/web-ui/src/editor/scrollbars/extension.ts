import type { EditorState, Text } from '@codemirror/state'
import { ViewPlugin, type EditorView, type PluginValue, type ViewUpdate } from '@codemirror/view'
import { getChunks, getOriginalDoc } from '@codemirror/merge'
import { searchMatches } from '../searchMatches'
import { mountScrollbarDom } from './dom'
import { buildScrollbarMarkers, type DiffScrollbarInput, type LineRange } from './markers'

function countLinesInRange(doc: Text, from: number, to: number) {
  if (to <= from) return 0
  const safeFrom = Math.max(0, Math.min(doc.length, from))
  const safeTo = Math.max(safeFrom, Math.min(doc.length, to))
  const startLine = doc.lineAt(safeFrom).number
  const endLine = doc.lineAt(Math.max(safeFrom, safeTo - 1)).number
  return endLine - startLine + 1
}

/**
 * A unified diff's rows: the current text with each removed stretch shown
 * above its replacement, so a marker sits at its row, not at its line of the
 * current text. Null when the view is not a diff.
 */
function unifiedLayout(state: EditorState) {
  const chunkState = getChunks(state)
  if (!chunkState) return null

  const originalDoc = getOriginalDoc(state)
  const modifiedDoc = state.doc
  let deletedLinesBefore = 0
  const offsets: Array<{ startLine: number; deletedLines: number }> = []
  const diffs: DiffScrollbarInput[] = []

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

function collectSearchMatches(state: EditorState, mapLine: (line: number) => number): LineRange[] {
  return searchMatches(state).ranges.map((match) => ({
    fromLine: mapLine(state.doc.lineAt(match.from).number),
    toLine: mapLine(state.doc.lineAt(match.to).number),
  }))
}

function collectSelections(state: EditorState, mapLine: (line: number) => number): LineRange[] {
  return state.selection.ranges
    .filter((range) => !range.empty)
    .map((range) => ({
      fromLine: mapLine(state.doc.lineAt(range.from).number),
      toLine: mapLine(state.doc.lineAt(Math.max(range.from, range.to - 1)).number),
    }))
}

class CustomScrollbarPlugin implements PluginValue {
  private mounted
  private frame = 0
  private readonly view: EditorView
  private readonly onScroll: () => void
  private lastScrollTop: number
  private lastScrollLeft: number

  constructor(view: EditorView) {
    this.view = view
    this.mounted = mountScrollbarDom(view, {
      splitDiffLanes: getChunks(view.state) !== null,
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

  update(update: ViewUpdate) {
    // Any transaction may move a marker: the text, the selection or the search query.
    if (update.transactions.length > 0 || update.geometryChanged || update.viewportChanged)
      this.schedule(update.view)
  }

  private schedule(view: EditorView) {
    if (this.frame) return
    this.frame = requestAnimationFrame(() => {
      this.frame = 0
      const layout = unifiedLayout(view.state)
      const mapLine = layout?.mapLine ?? ((line: number) => line)
      this.mounted.update(view, buildScrollbarMarkers({
        totalLines: layout?.totalLines ?? view.state.doc.lines,
        searches: collectSearchMatches(view.state, mapLine),
        selections: collectSelections(view.state, mapLine),
        diffs: layout?.diffs ?? [],
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
