import { fromEditorUri } from '../editorUri'

export const extToLanguageId: Record<string, string> = {
  '.ts': 'typescript',
  '.tsx': 'typescriptreact',
  '.js': 'javascript',
  '.jsx': 'javascriptreact',
  '.vue': 'vue',
  '.html': 'html',
  '.css': 'css',
  '.scss': 'scss',
  '.less': 'less',
  '.json': 'json',
  '.jsonc': 'jsonc',
  '.md': 'markdown',
  '.yaml': 'yaml',
  '.yml': 'yaml',
  '.py': 'python',
  '.go': 'go',
  '.rs': 'rust',
}

export function inferLanguageIdFromResourceUri(resourceUri: string): string {
  const filePath = fromEditorUri(resourceUri)?.filePath ?? resourceUri
  const dot = filePath.lastIndexOf('.')
  if (dot <= 0) return 'plaintext'
  const ext = filePath.slice(dot).toLowerCase()
  return extToLanguageId[ext] ?? ext.slice(1)
}
