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

export function utf8ByteLength(text: string): number {
  return encodeUtf8(text).byteLength
}
