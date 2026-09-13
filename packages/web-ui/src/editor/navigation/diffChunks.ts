import { Text } from '@codemirror/state'
import type { DiffChunkHandle } from './types'

function countLinesInRange(doc: Text, from: number, to: number) {
  if (to <= from) return 0
  const safeFrom = Math.max(0, Math.min(doc.length, from))
  const safeTo = Math.max(safeFrom, Math.min(doc.length, to))
  const startLine = doc.lineAt(safeFrom).number
  const endLine = doc.lineAt(Math.max(safeFrom, safeTo - 1)).number
  return endLine - startLine + 1
}

function buildLineNumbers(doc: Text, from: number, to: number) {
  const lineCount = countLinesInRange(doc, from, to)
  if (lineCount === 0) return []
  const safeFrom = Math.max(0, Math.min(doc.length, from))
  const startLine = doc.lineAt(safeFrom).number
  return Array.from({ length: lineCount }, (_, index) => startLine + index)
}

export function normalizeDiffChunks(
  chunks: ReadonlyArray<{ fromA: number; endA: number; fromB: number; endB: number }>,
  originalDoc: Text,
  modifiedDoc: Text,
): DiffChunkHandle[] {
  let deletedBlockIndex = 0
  return chunks.map((chunk, index) => {
    const changedLineNumbers = buildLineNumbers(modifiedDoc, chunk.fromB, chunk.endB)
    const deletedLineCount = countLinesInRange(originalDoc, chunk.fromA, chunk.endA)
    const anchorOffset = Math.min(
      modifiedDoc.length,
      Math.max(0, changedLineNumbers.length > 0 ? chunk.fromB : chunk.endB),
    )
    const anchorLineNumber = modifiedDoc.lineAt(anchorOffset).number

    const handle: DiffChunkHandle = {
      id: `chunk-${index}-${chunk.fromA}-${chunk.fromB}`,
      index,
      fromA: chunk.fromA,
      endA: chunk.endA,
      fromB: chunk.fromB,
      endB: chunk.endB,
      anchorLineNumber,
      changedLineNumbers,
    }

    if (deletedLineCount > 0) {
      handle.deletedBlockIndex = deletedBlockIndex
      deletedBlockIndex += 1
    }

    return handle
  })
}
