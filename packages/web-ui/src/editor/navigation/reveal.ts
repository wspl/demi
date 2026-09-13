import { Text } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import type { EditorLinePosition, EditorLineRange } from './types'

function clampLine(doc: Text, line: number) {
  return Math.max(0, Math.min(line, Math.max(0, doc.lines - 1)))
}

export function toDocOffset(doc: Text, position: EditorLinePosition) {
  const lineNumber = clampLine(doc, position.line) + 1
  const line = doc.line(lineNumber)
  return Math.min(line.from + Math.max(0, position.character), line.to)
}

export function getCoveredLineNumbers(doc: Text, range: EditorLineRange) {
  const start = doc.lineAt(toDocOffset(doc, range.start)).number
  const endPosition = range.end ?? range.start
  const endOffset = toDocOffset(doc, endPosition)
  const end = doc.lineAt(Math.max(0, endOffset)).number
  const from = Math.min(start, end)
  const to = Math.max(start, end)
  return Array.from({ length: to - from + 1 }, (_, index) => from + index)
}

function centerOffset(view: EditorView, offset: number) {
  const domAtPos = view.domAtPos(offset)
  const target = domAtPos.node.nodeType === Node.TEXT_NODE
    ? domAtPos.node.parentElement
    : domAtPos.node instanceof Element
      ? domAtPos.node
      : null
  const lineEl = target?.closest<HTMLElement>('.cm-line')
  if (lineEl) {
    lineEl.scrollIntoView({ block: 'center', inline: 'nearest' })
    view.scrollDOM.dispatchEvent(new Event('scroll'))
  }
}

export function centerRange(view: EditorView, range: EditorLineRange) {
  const offset = toDocOffset(view.state.doc, range.start)
  view.dispatch({
    effects: EditorView.scrollIntoView(offset, { y: 'center', x: 'nearest' }),
  })
  requestAnimationFrame(() => centerOffset(view, offset))
}

export function centerLine(view: EditorView, lineNumber: number) {
  const line = view.state.doc.line(Math.max(1, Math.min(lineNumber, view.state.doc.lines)))
  view.dispatch({
    effects: EditorView.scrollIntoView(line.from, { y: 'center', x: 'nearest' }),
  })
  requestAnimationFrame(() => centerOffset(view, line.from))
}

export function resetScrollTop(view: EditorView) {
  view.scrollDOM.scrollTop = 0
  view.scrollDOM.dispatchEvent(new Event('scroll'))
}
