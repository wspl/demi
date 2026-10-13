const encoder = new TextEncoder()

/** Returns the UTF-8 byte length of a string. */
export function utf8Bytes(text: string): number {
  return encoder.encode(text).byteLength
}

/** `bytes` in base64url without padding (RFC 4648 § 5), as WebAuthn's JSON writes binary values. */
export function base64urlEncode(bytes: ArrayBuffer | ArrayBufferView): string {
  const view = bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let binary = ''
  for (const byte of view) {
    binary += String.fromCharCode(byte)
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** The bytes base64url `text` encodes, with or without padding; throws on text that is not base64url. */
export function base64urlDecode(text: string): Uint8Array<ArrayBuffer> {
  const binary = atob(text.replace(/-/g, '+').replace(/_/g, '/'))
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}
