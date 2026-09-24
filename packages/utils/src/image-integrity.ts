/** Checks supported image framing before bytes cross an inference boundary. */
export function hasImageIntegrity(bytes: Uint8Array, mediaType: string): boolean {
  try {
    switch (mediaType.toLowerCase()) {
      case 'image/png': return png(bytes)
      case 'image/jpeg': case 'image/jpg': return jpeg(bytes)
      case 'image/gif': return gif(bytes)
      case 'image/webp': return webp(bytes)
      default: return false
    }
  } catch {
    return false
  }
}

function ascii(b: Uint8Array, p: number, n: number): string {
  return String.fromCharCode(...b.subarray(p, p + n))
}
function u32(b: Uint8Array, p: number, little = false): number {
  return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(p, little)
}
function u16(b: Uint8Array, p: number, little = false): number {
  return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint16(p, little)
}
const crcTable = Uint32Array.from({ length: 256 }, (_, n) => {
  for (let i = 0; i < 8; i++) n = (n & 1) ? 0xedb88320 ^ (n >>> 1) : n >>> 1
  return n >>> 0
})
function crc32(b: Uint8Array, start: number, end: number): number {
  let crc = 0xffffffff
  for (let i = start; i < end; i++) crc = crcTable[(crc ^ b[i]) & 255] ^ (crc >>> 8)
  return (crc ^ 0xffffffff) >>> 0
}
function png(b: Uint8Array): boolean {
  if (b.length < 45 || u32(b, 0) !== 0x89504e47 || u32(b, 4) !== 0x0d0a1a0a) return false
  let data = false
  let header = false
  for (let p = 8; p + 12 <= b.length;) {
    const size = u32(b, p)
    const end = p + 8 + size
    if (end + 4 > b.length || crc32(b, p + 4, end) !== u32(b, end)) return false
    const type = ascii(b, p + 4, 4)
    if (!header && type !== 'IHDR') return false
    if (type === 'IHDR') {
      if (header || size !== 13 || u32(b, p + 8) === 0 || u32(b, p + 12) === 0) return false
      const depth = b[p + 16], color = b[p + 17]
      const depths: Record<number, number[]> = { 0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16] }
      if (!depths[color]?.includes(depth) || b[p + 18] !== 0 || b[p + 19] !== 0 || b[p + 20] > 1) return false
      header = true
    }
    if (type === 'IDAT' && size > 0) data = true
    if (type === 'IEND') return size === 0 && header && data && end + 4 === b.length
    p = end + 4
  }
  return false
}
function jpeg(b: Uint8Array): boolean {
  if (b.length < 4 || u16(b, 0) !== 0xffd8) return false
  let frame = false, scan = false, entropy = false
  let p = 2
  while (p < b.length) {
    if (b[p++] !== 0xff) return false
    while (b[p] === 0xff) p++
    const marker = b[p++]
    if (marker === 0xd9) return frame && scan && entropy
    if (marker === undefined || marker === 0 || marker === 0xd8) return false
    if (marker === 1 || (marker >= 0xd0 && marker <= 0xd7)) continue
    const size = u16(b, p)
    if (size < 2 || p + size > b.length) return false
    if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker)) {
      if (size < 8 || !u16(b, p + 3) || !u16(b, p + 5)) return false
      frame = true
    }
    p += size
    if (marker === 0xda) {
      if (!frame || size < 6) return false
      scan = true
      while (p < b.length) {
        if (b[p] !== 0xff) { entropy = true; p++; continue }
        if (b[p + 1] === 0 || (b[p + 1] >= 0xd0 && b[p + 1] <= 0xd7)) { p += 2; continue }
        break
      }
    }
  }
  return false
}
function gif(b: Uint8Array): boolean {
  if (!['GIF87a', 'GIF89a'].includes(ascii(b, 0, 6)) || !u16(b, 6, true) || !u16(b, 8, true)) return false
  let p = 13 + ((b[10] & 128) ? 3 * (1 << ((b[10] & 7) + 1)) : 0)
  let frames = 0
  const subblocks = (): boolean => {
    while (p < b.length) {
      const size = b[p++]
      if (size === 0) return true
      p += size
      if (p > b.length) return false
    }
    return false
  }
  while (p < b.length) {
    const tag = b[p++]
    if (tag === 0x3b) return frames > 0 && p === b.length
    if (tag === 0x21) { p++; if (!subblocks()) return false; continue }
    if (tag !== 0x2c || p + 9 > b.length || !u16(b, p + 4, true) || !u16(b, p + 6, true)) return false
    const packed = b[p + 8]
    p += 9 + ((packed & 128) ? 3 * (1 << ((packed & 7) + 1)) : 0)
    const codeSize = b[p++]
    if (codeSize < 2 || codeSize > 8 || !b[p] || !subblocks()) return false
    frames++
  }
  return false
}
function webp(b: Uint8Array): boolean {
  if (ascii(b, 0, 4) !== 'RIFF' || ascii(b, 8, 4) !== 'WEBP' || u32(b, 4, true) + 8 !== b.length) return false
  const chunks = (start: number, end: number): number => {
    let images = 0, p = start
    while (p + 8 <= end) {
      const type = ascii(b, p, 4), size = u32(b, p + 4, true), data = p + 8
      const next = data + size + (size & 1)
      if (next > end) return -1
      if (type === 'VP8 ') {
        if (size < 10 || (b[data] & 1) || ascii(b, data + 3, 3) !== '\x9d\x01\x2a' || !(u16(b, data + 6, true) & 16383) || !(u16(b, data + 8, true) & 16383)) return -1
        images++
      } else if (type === 'VP8L') {
        if (size < 5 || b[data] !== 0x2f || b[data + 4] >> 5) return -1
        images++
      } else if (type === 'ANMF') {
        if (size < 16 || chunks(data + 16, data + size) <= 0) return -1
        images++
      }
      p = next
    }
    return p === end ? images : -1
  }
  return chunks(12, b.length) > 0
}
