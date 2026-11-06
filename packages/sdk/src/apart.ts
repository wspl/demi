/**
 * A message whose bytes travel beside its JSON, as a plugin's lane carries
 * them (`crates/shared-types/src/bytes.rs`): each `Uint8Array` in the value
 * is the index of its buffer in the JSON, and the buffers follow it in one
 * binary message. A message with no bytes is the JSON alone, as text.
 *
 * ```text
 * u32 JSON length | JSON: { "bytes": { "$$ref$$": 0 } } | u32 length | bytes
 * ```
 */

/** The key of a buffer's index in the JSON. */
const REF = '$$ref$$'

/** `value` as its lane message: text without bytes, binary with them. */
export function encodeApart(value: unknown): string | Uint8Array<ArrayBuffer> {
  const buffers: Uint8Array[] = []
  const json = JSON.stringify(value, (_key, field: unknown) => {
    if (field instanceof Uint8Array) {
      buffers.push(field)
      return { [REF]: buffers.length - 1 }
    }
    return field
  })
  if (buffers.length === 0) {
    return json
  }
  const parts = [new TextEncoder().encode(json), ...buffers]
  const bytes = new Uint8Array(parts.reduce((total, part) => total + 4 + part.length, 0))
  const view = new DataView(bytes.buffer)
  let at = 0
  for (const part of parts) {
    view.setUint32(at, part.length)
    bytes.set(part, at + 4)
    at += 4 + part.length
  }
  return bytes
}

/** The value of a lane message, its buffers in their places; throws when it does not read. */
export function decodeApart(data: string | ArrayBuffer): unknown {
  if (typeof data === 'string') {
    return JSON.parse(data)
  }
  const bytes = new Uint8Array(data)
  const view = new DataView(data)
  const parts: Uint8Array[] = []
  for (let at = 0; at < bytes.length;) {
    if (at + 4 > bytes.length) {
      throw new Error('A part\'s length is cut short')
    }
    const length = view.getUint32(at)
    if (at + 4 + length > bytes.length) {
      throw new Error('A part is cut short')
    }
    parts.push(bytes.subarray(at + 4, at + 4 + length))
    at += 4 + length
  }
  const [json, ...buffers] = parts
  if (json === undefined) {
    throw new Error('A binary message holds no JSON')
  }
  return JSON.parse(new TextDecoder().decode(json), (_key, field: unknown) => {
    if (typeof field === 'object' && field !== null && Object.keys(field).length === 1 && REF in field) {
      const buffer = buffers[(field as Record<string, number>)[REF]!]
      if (buffer === undefined) {
        throw new Error('The JSON names a buffer that did not come with it')
      }
      return buffer
    }
    return field
  })
}
