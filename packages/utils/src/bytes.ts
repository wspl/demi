const encoder = new TextEncoder()

/** Returns the UTF-8 byte length of a string. */
export function utf8Bytes(text: string): number {
  return encoder.encode(text).byteLength
}
