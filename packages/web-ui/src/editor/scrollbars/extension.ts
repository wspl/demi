import type { EditorState, Text } from '@codemirror/state'
import { ViewPlugin, type EditorView, type PluginValue, type ViewUpdate } from '@codemirror/view'
import { getChunks, getOriginalDoc } from '@codemirror/merge'
import { searchMatches } from '../searchMatches'
import type { OverlayScrollbars } from 'overlayscrollbars'
import { attachScrollbars } from '../../ui/scrollbars'
import { mountScrollbarRuler, type ScrollbarRuler } from './ruler'
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

/**
 * The editor's scrollbars: OverlayScrollbars' on its scroller, as every
 * scroller of the app has (`ui/scrollbars.ts`), placed in the editor's box,
 * over the ruler of marks for the selection, the find bar's matches and a
 * diff's changes.
 */
class CustomScrollbarPlugin implements PluginValue {
  private readonly ruler: ScrollbarRuler
  private readonly scrollbars: OverlayScrollbars
  private frame = 0

  constructor(view: EditorView) {
    this.ruler = mountScrollbarRuler(view, {
      splitDiffLanes: getChunks(view.state) !== null,
    })
    // After the ruler, so the bars lie over its marks.
    this.scrollbars = attachScrollbars(view.scrollDOM, 'both', view.dom)
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
      this.ruler.update(view, buildScrollbarMarkers({
        totalLines: layout?.totalLines ?? view.state.doc.lines,
        searches: collectSearchMatches(view.state, mapLine),
        selections: collectSelections(view.state, mapLine),
        diffs: layout?.diffs ?? [],
      }))
    })
  }

  destroy() {
    if (this.frame) cancelAnimationFrame(this.frame)
    this.scrollbars.destroy()
    this.ruler.destroy()
  }
}

export function customScrollbarExtension() {
  return ViewPlugin.fromClass(CustomScrollbarPlugin)
}
