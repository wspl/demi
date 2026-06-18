const BINARY_MARKER = '__demiUint8Array'
const BIGINT_MARKER = '__demiBigInt'
const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

export function stringifyRpcJson(value: unknown): string {
  return JSON.stringify(value, (_key, nested) => {
    if (nested instanceof Uint8Array) {
      return {
        [BINARY_MARKER]: true,
        base64: bytesToBase64(nested),
      }
    }
    if (typeof nested === 'bigint') {
      return {
        [BIGINT_MARKER]: true,
        value: nested.toString(),
      }
    }
    return nested
  })
}

export function parseRpcJson<T>(text: string): T {
  return JSON.parse(text, (_key, nested) => {
    if (isEncodedUint8Array(nested)) {
      return base64ToBytes(nested.base64)
    }
    if (isEncodedBigInt(nested)) {
      return BigInt(nested.value)
    }
    return nested
  }) as T
}

function isEncodedUint8Array(value: unknown): value is { base64: string } {
  return (
    value !== null &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    (value as Record<string, unknown>)[BINARY_MARKER] === true &&
    typeof (value as Record<string, unknown>).base64 === 'string'
  )
}

function isEncodedBigInt(value: unknown): value is { value: string } {
  return (
    value !== null &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    (value as Record<string, unknown>)[BIGINT_MARKER] === true &&
    typeof (value as Record<string, unknown>).value === 'string'
  )
}

function bytesToBase64(bytes: Uint8Array): string {
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

function base64ToBytes(base64: string): Uint8Array {
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
