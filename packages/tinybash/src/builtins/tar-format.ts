// The tar format as `tar` reads and writes it (`tinybash.md` § Builtins):
// 512-byte headers in ustar layout with GNU's magic on write; on read,
// ustar, GNU (long names and links) and pax (extended headers) archives,
// which is what GNU tar, bsdtar and Go produce. Bytes are handled as
// blocks: a reader yields them from any byte stream, a writer pads a
// member's data to the block and the archive to GNU's record.
import { decodeUtf8, encodeUtf8 } from '@demicodes/utils'

export const BLOCK = 512
/** GNU's default blocking factor: an archive ends padded to 20 blocks. */
export const RECORD = BLOCK * 20

export type TarEntryKind = 'file' | 'directory' | 'symlink' | 'hardlink' | 'other'

/** One member as the archive describes it. */
export interface TarHeader {
  name: string
  kind: TarEntryKind
  mode: number
  uid: number
  gid: number
  size: number
  mtime: Date
  linkName: string
  uname: string
  gname: string
  /** The raw typeflag, for messages about kinds tinybash does not create. */
  typeflag: string
}

const ZERO_BLOCK = new Uint8Array(BLOCK)

function isZeroBlock(block: Uint8Array): boolean {
  return block.every((byte) => byte === 0)
}

function field(block: Uint8Array, offset: number, length: number): string {
  let end = offset
  while (end < offset + length && block[end] !== 0) end += 1
  return decodeUtf8(block.subarray(offset, end))
}

/** An octal numeric field, or GNU's base-256 form for values that do not fit. */
function numeric(block: Uint8Array, offset: number, length: number): number {
  const first = block[offset]!
  if (first & 0x80) {
    let value = 0
    for (let i = 1; i < length; i += 1) value = value * 256 + block[offset + i]!
    return value
  }
  const text = field(block, offset, length).trim()
  return text === '' ? 0 : Number.parseInt(text, 8)
}

function putText(block: Uint8Array, offset: number, length: number, text: string): void {
  const bytes = encodeUtf8(text)
  block.set(bytes.subarray(0, length), offset)
}

function putOctal(block: Uint8Array, offset: number, length: number, value: number): void {
  putText(block, offset, length, `${Math.max(0, Math.floor(value)).toString(8).padStart(length - 1, '0')}`)
}

function checksumOf(block: Uint8Array): number {
  let sum = 0
  for (let i = 0; i < BLOCK; i += 1) sum += i >= 148 && i < 156 ? 32 : block[i]!
  return sum
}

/** GNU's checksum field: six octal digits, a NUL, a space. */
function putChecksum(block: Uint8Array): void {
  putText(block, 148, 8, `${checksumOf(block).toString(8).padStart(6, '0')}\0 `)
}

const TYPEFLAGS: Record<string, TarEntryKind> = { '0': 'file', '\0': 'file', '': 'file', '7': 'file', '5': 'directory', '2': 'symlink', '1': 'hardlink' }

/** Decodes a header block; `null` for a zero block. Throws on a block that is not a header. */
export function decodeHeader(block: Uint8Array): TarHeader | null {
  if (isZeroBlock(block)) return null
  const recorded = numeric(block, 148, 8)
  if (recorded !== checksumOf(block)) throw new Error('This does not look like a tar archive')
  const magic = field(block, 257, 6)
  const ustar = magic === 'ustar' || magic === 'ustar '
  const typeflag = String.fromCharCode(block[156]!)
  const prefix = ustar && magic === 'ustar' ? field(block, 345, 155) : ''
  const base = field(block, 0, 100)
  return {
    name: prefix ? `${prefix}/${base}` : base,
    kind: TYPEFLAGS[typeflag] ?? 'other',
    mode: numeric(block, 100, 8) & 0o7777,
    uid: numeric(block, 108, 8),
    gid: numeric(block, 116, 8),
    size: numeric(block, 124, 12),
    mtime: new Date(numeric(block, 136, 12) * 1000),
    linkName: field(block, 157, 100),
    uname: ustar ? field(block, 265, 32) : '',
    gname: ustar ? field(block, 297, 32) : '',
    typeflag,
  }
}

/** The data blocks a header's member occupies. */
export function dataBlocks(size: number): number {
  return Math.ceil(size / BLOCK)
}

/**
 * Encodes a member's header the way GNU tar does by default: the GNU
 * magic, names longer than the field as a preceding `L` (or `K`) member.
 */
export function encodeHeader(header: Omit<TarHeader, 'typeflag'>): Uint8Array[] {
  const blocks: Uint8Array[] = []
  const nameBytes = encodeUtf8(header.name)
  const linkBytes = encodeUtf8(header.linkName)
  if (nameBytes.byteLength > 100) blocks.push(...longName('L', nameBytes, header))
  if (linkBytes.byteLength > 100) blocks.push(...longName('K', linkBytes, header))
  const block = new Uint8Array(BLOCK)
  block.set(nameBytes.subarray(0, 100), 0)
  putOctal(block, 100, 8, header.mode)
  putOctal(block, 108, 8, header.uid)
  putOctal(block, 116, 8, header.gid)
  putOctal(block, 124, 12, header.kind === 'file' ? header.size : 0)
  putOctal(block, 136, 12, Math.floor(header.mtime.getTime() / 1000))
  block[156] = { file: 0x30, directory: 0x35, symlink: 0x32, hardlink: 0x31, other: 0x30 }[header.kind]
  block.set(linkBytes.subarray(0, 100), 157)
  putText(block, 257, 8, 'ustar  ')
  putText(block, 265, 32, header.uname)
  putText(block, 297, 32, header.gname)
  putChecksum(block)
  blocks.push(block)
  return blocks
}

/** GNU's long-name member: a header of type `L`/`K` named `././@LongLink` whose data is the name. */
function longName(type: 'L' | 'K', bytes: Uint8Array, owner: Pick<TarHeader, 'uid' | 'gid' | 'uname' | 'gname'>): Uint8Array[] {
  const block = new Uint8Array(BLOCK)
  putText(block, 0, 100, '././@LongLink')
  putOctal(block, 100, 8, 0o644)
  putOctal(block, 108, 8, owner.uid)
  putOctal(block, 116, 8, owner.gid)
  putOctal(block, 124, 12, bytes.byteLength + 1)
  putOctal(block, 136, 12, 0)
  block[156] = type.charCodeAt(0)
  putText(block, 257, 8, 'ustar  ')
  putText(block, 265, 32, owner.uname)
  putText(block, 297, 32, owner.gname)
  putChecksum(block)
  const data = new Uint8Array(dataBlocks(bytes.byteLength + 1) * BLOCK)
  data.set(bytes)
  return [block, data]
}

/** Pads a member's bytes to the block. */
export function padToBlock(length: number): Uint8Array {
  const rest = length % BLOCK
  return rest === 0 ? new Uint8Array(0) : ZERO_BLOCK.subarray(0, BLOCK - rest)
}

/** The end of an archive: two zero blocks, then padding to the record. */
export function archiveEnd(written: number): Uint8Array {
  const total = written + 2 * BLOCK
  const padded = Math.ceil(total / RECORD) * RECORD
  return new Uint8Array(padded - written)
}

/**
 * Reads a byte stream as 512-byte blocks. A stream too short for a first
 * block is not an archive; one that ends mid-block later is truncated —
 * GNU's two messages.
 */
export class BlockReader {
  private readonly iterator: AsyncIterator<Uint8Array>
  private buffer = new Uint8Array(0)
  private ended = false
  private blocks = 0

  constructor(source: AsyncIterable<Uint8Array>) {
    this.iterator = source[Symbol.asyncIterator]()
  }

  /** The next block, or `null` at a clean end. */
  async next(): Promise<Uint8Array | null> {
    while (this.buffer.byteLength < BLOCK) {
      if (this.ended) {
        if (this.buffer.byteLength === 0) return null
        throw new Error(this.blocks === 0 ? 'This does not look like a tar archive' : 'Unexpected EOF in archive')
      }
      const chunk = await this.iterator.next()
      if (chunk.done) {
        this.ended = true
        continue
      }
      if (chunk.value.byteLength === 0) continue
      const joined = new Uint8Array(this.buffer.byteLength + chunk.value.byteLength)
      joined.set(this.buffer)
      joined.set(chunk.value, this.buffer.byteLength)
      this.buffer = joined
    }
    const block = this.buffer.subarray(0, BLOCK)
    this.buffer = this.buffer.subarray(BLOCK)
    this.blocks += 1
    return block
  }

  /** Exactly `size` bytes of member data, its padding consumed. */
  async data(size: number): Promise<Uint8Array> {
    const out = new Uint8Array(size)
    let at = 0
    for (let remaining = dataBlocks(size); remaining > 0; remaining -= 1) {
      const block = await this.next()
      if (block === null) throw new Error('Unexpected EOF in archive')
      const take = Math.min(BLOCK, size - at)
      out.set(block.subarray(0, take), at)
      at += take
    }
    return out
  }

  async close(): Promise<void> {
    await this.iterator.return?.()
  }
}

/**
 * A pax extended header's records (`len key=value\n`); `path`, `linkpath`,
 * `size` and `mtime` override the following member's header fields.
 */
export function decodePax(data: Uint8Array): Partial<Pick<TarHeader, 'name' | 'linkName' | 'size' | 'mtime'>> {
  const text = decodeUtf8(data)
  const out: Partial<Pick<TarHeader, 'name' | 'linkName' | 'size' | 'mtime'>> = {}
  let at = 0
  while (at < text.length) {
    const space = text.indexOf(' ', at)
    if (space === -1) break
    const length = Number.parseInt(text.slice(at, space), 10)
    if (!Number.isFinite(length) || length <= 0) break
    const record = text.slice(space + 1, at + length - 1)
    const equals = record.indexOf('=')
    if (equals !== -1) {
      const key = record.slice(0, equals)
      const value = record.slice(equals + 1)
      if (key === 'path') out.name = value
      else if (key === 'linkpath') out.linkName = value
      else if (key === 'size') out.size = Number(value)
      else if (key === 'mtime') out.mtime = new Date(Number(value) * 1000)
    }
    at += length
  }
  return out
}

/** The header string GNU prints for `-v` listings and `ls -l`: type letter and the nine permission bits. */
export function modeString(kind: TarEntryKind, mode: number): string {
  const type = kind === 'directory' ? 'd' : kind === 'symlink' ? 'l' : kind === 'hardlink' ? 'h' : '-'
  const bits = ['r', 'w', 'x']
  let out = type
  for (let shift = 6; shift >= 0; shift -= 3) {
    for (let i = 0; i < 3; i += 1) out += mode & (1 << (shift + 2 - i)) ? bits[i]! : '-'
  }
  return out
}
