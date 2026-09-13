export interface EditorLinePosition {
  line: number
  character: number
}

export interface EditorLineRange {
  start: EditorLinePosition
  end?: EditorLinePosition
}

export interface RevealOptions {
  verticalAlign?: 'center'
  preserveSelection?: boolean
}

export interface FlashOptions {
  durationMs?: number
}

export type FlashKind = 'range' | 'diff-changed' | 'diff-deleted'

export interface DiffChunkHandle {
  id: string
  index: number
  fromA: number
  endA: number
  fromB: number
  endB: number
  anchorLineNumber: number
  changedLineNumbers: number[]
  deletedBlockIndex?: number
}
