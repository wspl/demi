const encoder = new TextEncoder()
const decoder = new TextDecoder()

export function encodeUtf8(text: string): Uint8Array {
  return encoder.encode(text)
}

export function decodeUtf8(data: Uint8Array): string {
  return decoder.decode(data)
}

export function concatBytes(chunks: Uint8Array[]): Uint8Array {
  const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0)
  const combined = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    combined.set(chunk, offset)
    offset += chunk.byteLength
  }
  return combined
}

export function dirnamePath(path: string): string {
  const normalized = normalizePath(path)
  if (normalized === '/' || /^[A-Za-z]:\/?$/.test(normalized)) return normalized
  const index = normalized.lastIndexOf('/')
  if (index === -1) return '.'
  if (index === 0) return '/'
  if (index === 2 && /^[A-Za-z]:/.test(normalized)) return normalized.slice(0, 3)
  return normalized.slice(0, index)
}

function normalizePath(path: string): string {
  const slashPath = path.replace(/\\/g, '/')
  const drive = /^[A-Za-z]:/.exec(slashPath)?.[0].toUpperCase() ?? ''
  const absolute = slashPath.startsWith('/') || drive.length > 0
  const body = drive ? slashPath.slice(2) : slashPath
  const parts: string[] = []
  for (const segment of body.split('/')) {
    if (segment === '' || segment === '.') continue
    if (segment === '..') {
      if (parts.length > 0 && parts[parts.length - 1] !== '..') parts.pop()
      else if (!absolute) parts.push(segment)
      continue
    }
    parts.push(segment)
  }
  if (drive) return parts.length > 0 ? `${drive}/${parts.join('/')}` : `${drive}/`
  if (absolute) return `/${parts.join('/')}`
  return parts.join('/') || '.'
}
