import { Text } from '@codemirror/state'
import { fromEditorUri } from '../editorUri'
import type { EditorHost, EditorLspTarget } from '../host/types'

export interface LspPosition { line: number; character: number }
export interface LspRange { start: LspPosition; end: LspPosition }
export interface LspLocation { uri: string; range: LspRange }
export interface LspLocationLink { targetUri: string; targetRange: LspRange; targetSelectionRange: LspRange }
export interface LspTextEdit { range: LspRange; newText: string }

export function toLspPosition(doc: Text, pos: number): LspPosition {
  const line = doc.lineAt(pos)
  return { line: line.number - 1, character: pos - line.from }
}

export function fromLspPosition(doc: Text, pos: LspPosition): number {
  if (pos.line >= doc.lines) return doc.length
  const line = doc.line(pos.line + 1)
  return Math.min(line.from + pos.character, line.to)
}

export function fromLspRange(doc: Text, range: LspRange): { from: number; to: number } {
  return { from: fromLspPosition(doc, range.start), to: fromLspPosition(doc, range.end) }
}

export function findPluginForResource(host: EditorHost, resourceUri: string): EditorLspTarget | null {
  return host.lsp.getTargetsForResource(resourceUri)[0] ?? null
}

export function getPluginTargetKey(target: Pick<EditorLspTarget, 'hostId' | 'cwd' | 'pluginId'>): string {
  return `${target.hostId}:${target.cwd}:${target.pluginId}`
}

export function getResourcePath(resourceUri: string): string {
  return fromEditorUri(resourceUri)?.filePath ?? resourceUri
}

export function getResourceBasename(resourceUri: string): string {
  const filePath = getResourcePath(resourceUri)
  return filePath.split('/').pop() ?? filePath
}
