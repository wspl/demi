import type { ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import type { InferenceItem } from '@demicodes/provider'
import { base64ToBytes, bytesToBase64, hasImageIntegrity } from '@demicodes/utils'

export const CORRUPT_IMAGE_TEXT = 'Image data is corrupted.'

// Exact content keys avoid stale verdicts after mutable source objects change.
// Both retained key size and entry count are bounded across long-lived sessions.
const cache = new Map<string, boolean>()
const CACHE_CHARS = 8 * 1024 * 1024
let cacheChars = 0

function validBase64(data: string, mediaType: string): boolean {
  const key = `${mediaType}:${data}`
  const cached = cache.get(key)
  if (cached !== undefined) return cached
  let valid = false
  try {
    valid = data.length % 4 === 0 && /^[A-Za-z0-9+/]*={0,2}$/.test(data)
      && hasImageIntegrity(base64ToBytes(data), mediaType)
  } catch { /* Invalid media is represented as text at the inference boundary. */ }
  if (key.length <= CACHE_CHARS) {
    while (cache.size >= 64 || cacheChars + key.length > CACHE_CHARS) {
      const oldest = cache.keys().next().value
      if (oldest === undefined) break
      cacheChars -= oldest.length
      cache.delete(oldest)
    }
    cache.set(key, valid)
    cacheChars += key.length
  }
  return valid
}

function validUrl(url: string): boolean {
  // Remote URLs are resolved by the provider; do not introduce network fetches
  // or change their authorization semantics during local request projection.
  if (!/^data:/i.test(url)) return true
  const match = /^data:([^;,]+);base64,([\s\S]*)$/i.exec(url)
  return !!match && validBase64(match[2], match[1])
}

function validImage(block: Extract<UserContentBlock | ToolResultContentBlock, { type: 'image' }>): boolean {
  const source = block.source
  if ('type' in source && source.type === 'url') return validUrl(source.url)
  return validBase64(typeof source.data === 'string' ? source.data : bytesToBase64(source.data), source.mediaType)
}

export function filterCorruptImages(blocks: UserContentBlock[]): UserContentBlock[]
export function filterCorruptImages(blocks: ToolResultContentBlock[]): ToolResultContentBlock[]
export function filterCorruptImages(blocks: (UserContentBlock | ToolResultContentBlock)[]): (UserContentBlock | ToolResultContentBlock)[] {
  return blocks.map(block => block.type === 'image' && !validImage(block)
    ? { type: 'text', text: CORRUPT_IMAGE_TEXT }
    : block)
}

/** Project requests without changing the persisted transcript or rerunning tools. */
export function filterInferenceImages(items: InferenceItem[]): InferenceItem[] {
  return items.map(item => {
    if (item.type === 'tool_result') return { ...item, output: filterCorruptImages(item.output) }
    if (item.type === 'user_message' || item.type === 'user_steer') return { ...item, content: filterCorruptImages(item.content) }
    return item
  })
}
