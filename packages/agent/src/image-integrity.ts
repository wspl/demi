import type { ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import { base64ToBytes, hasImageIntegrity } from '@demicodes/utils'

export const CORRUPT_IMAGE_TEXT = 'Image data is corrupted.'

function validBase64(data: string, mediaType: string): boolean {
  try {
    return data.length % 4 === 0 && /^[A-Za-z0-9+/]*={0,2}$/.test(data)
      && hasImageIntegrity(base64ToBytes(data), mediaType)
  } catch { return false }
}

function validUrl(url: string): boolean {
  // Remote URLs are resolved by the provider; do not introduce network fetches
  // or change their authorization semantics during input validation.
  if (!/^data:/i.test(url)) return true
  const match = /^data:([^;,]+);base64,([\s\S]*)$/i.exec(url)
  return !!match && validBase64(match[2], match[1])
}

function validImage(block: Extract<UserContentBlock | ToolResultContentBlock, { type: 'image' }>): boolean {
  const source = block.source
  if ('type' in source && source.type === 'url') return validUrl(source.url)
  return typeof source.data === 'string'
    ? validBase64(source.data, source.mediaType)
    : hasImageIntegrity(source.data, source.mediaType)
}

export function filterCorruptImages(blocks: UserContentBlock[]): UserContentBlock[]
export function filterCorruptImages(blocks: ToolResultContentBlock[]): ToolResultContentBlock[]
export function filterCorruptImages(blocks: (UserContentBlock | ToolResultContentBlock)[]): (UserContentBlock | ToolResultContentBlock)[] {
  return blocks.map(block => block.type === 'image' && !validImage(block)
    ? { type: 'text', text: CORRUPT_IMAGE_TEXT }
    : block)
}
