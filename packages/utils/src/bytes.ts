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
