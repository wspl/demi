const EDITOR_URI_SCHEME = 'editor:'

function encodePath(filePath: string): string {
  return filePath
    .split('/')
    .map((segment) => encodeURIComponent(segment))
    .join('/')
}

function decodePath(encodedPath: string): string {
  return encodedPath
    .split('/')
    .map((segment) => decodeURIComponent(segment))
    .join('/')
}

export function toEditorUri(hostId: string, filePath: string): string {
  const normalizedHostId = hostId.trim()
  if (!normalizedHostId || /[/?#]/.test(normalizedHostId)) {
    throw new Error(`Invalid editor host id: ${hostId}`)
  }

  const normalizedPath = filePath.startsWith('/') ? filePath : `/${filePath}`
  return `editor://${normalizedHostId}${encodePath(normalizedPath)}`
}

export function fromEditorUri(uri: string): { hostId: string; filePath: string } | null {
  if (!uri.startsWith(EDITOR_URI_SCHEME)) return null

  try {
    const parsed = new URL(uri)
    if (parsed.protocol !== EDITOR_URI_SCHEME) return null
    return {
      hostId: parsed.host,
      filePath: decodePath(parsed.pathname || '/'),
    }
  } catch {
    return null
  }
}
