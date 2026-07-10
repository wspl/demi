const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** Encodes a string to UTF-8 bytes. */
export function encodeUtf8(text: string): Uint8Array {
  return encoder.encode(text)
}

/** Decodes UTF-8 bytes to a string. */
export function decodeUtf8(data: Uint8Array): string {
  return decoder.decode(data)
}

/** Returns the UTF-8 byte length of a string. */
export function utf8Bytes(text: string): number {
  return encoder.encode(text).byteLength
}

/** Packs a latin1 byte-string (each char = one byte, 0–255) into bytes. */
export function encodeLatin1(text: string): Uint8Array {
  const out = new Uint8Array(text.length)
  for (let i = 0; i < text.length; i += 1) out[i] = text.charCodeAt(i) & 0xff
  return out
}

/** Unpacks bytes into a latin1 byte-string (each char = one byte). */
export function decodeLatin1(bytes: Uint8Array): string {
  let out = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) {
    out += String.fromCharCode(...bytes.subarray(i, i + chunk))
  }
  return out
}

/** Strictly decodes UTF-8; returns null when the bytes are not valid UTF-8. */
export function decodeUtf8Strict(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    return null
  }
}

/** Slices a string by UTF-8 byte offsets, returning the decoded substring. */
export function utf8Slice(text: string, start: number, end: number): string {
  if (start <= 0 && end >= utf8Bytes(text)) return text
  return decoder.decode(encoder.encode(text).slice(start, end))
}

/** Concatenates byte chunks into a single `Uint8Array`. */
export function concatBytes(chunks: readonly Uint8Array[]): Uint8Array {
  const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0)
  const combined = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    combined.set(chunk, offset)
    offset += chunk.byteLength
  }
  return combined
}

const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** Encodes bytes as standard base64 (platform-neutral, no Node or DOM globals). */
export function bytesToBase64(bytes: Uint8Array): string {
  let output = ''
  for (let index = 0; index < bytes.byteLength; index += 3) {
    const first = bytes[index]
    const second = bytes[index + 1]
    const third = bytes[index + 2]
    output += BASE64_ALPHABET[first >> 2]
    output += BASE64_ALPHABET[((first & 0x03) << 4) | ((second ?? 0) >> 4)]
    output += second === undefined ? '=' : BASE64_ALPHABET[((second & 0x0f) << 2) | ((third ?? 0) >> 6)]
    output += third === undefined ? '=' : BASE64_ALPHABET[third & 0x3f]
  }
  return output
}

/** Decodes standard base64 to bytes, throwing on malformed payloads. */
export function base64ToBytes(base64: string): Uint8Array {
  const clean = base64.replace(/\s+/g, '')
  if (clean.length === 0) return new Uint8Array()
  if (clean.length % 4 !== 0) throw new Error('Invalid base64 payload length')

  const padding = clean.endsWith('==') ? 2 : clean.endsWith('=') ? 1 : 0
  const bytes = new Uint8Array((clean.length / 4) * 3 - padding)
  let offset = 0

  for (let index = 0; index < clean.length; index += 4) {
    const first = base64Value(clean[index])
    const second = base64Value(clean[index + 1])
    const third = clean[index + 2] === '=' ? 0 : base64Value(clean[index + 2])
    const fourth = clean[index + 3] === '=' ? 0 : base64Value(clean[index + 3])
    const triple = (first << 18) | (second << 12) | (third << 6) | fourth

    if (offset < bytes.byteLength) bytes[offset++] = (triple >> 16) & 0xff
    if (offset < bytes.byteLength) bytes[offset++] = (triple >> 8) & 0xff
    if (offset < bytes.byteLength) bytes[offset++] = triple & 0xff
  }

  return bytes
}

function base64Value(char: string | undefined): number {
  if (!char || char === '=') throw new Error('Invalid base64 payload')
  const value = BASE64_ALPHABET.indexOf(char)
  if (value === -1) throw new Error('Invalid base64 payload')
  return value
}
