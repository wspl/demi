import { base64ToBytes, bytesToBase64 } from '@demicodes/utils'
import type { Block, ToolResultContentBlock, UserContentBlock } from '@demicodes/core'

/**
 * Content-addressed byte storage for transcript media. Databases hold block
 * rows only; media bytes always live behind one of these (a directory at
 * N=1, S3 at N>1). `put` is idempotent by content.
 */
export interface BlobStore {
  /** Stores bytes and returns their sha256 hex key. */
  put(data: Uint8Array): Promise<string>
  get(sha256: string): Promise<Uint8Array | null>
}

// ── media externalization ───────────────────────────────────────────
//
// The persisted representation of a media source replaces its bytes with a
// `ref` (the blob's sha256). This form exists only at rest — in-memory blocks
// and providers always carry inline bytes; a missing blob degrades to a text
// placeholder at load instead of failing the session.

interface RefSource {
  type: 'ref'
  ref: string
  mediaType: string
  fileName?: string
}

interface RefBase64Source {
  ref: string
  mediaType: string
}

function isRefSource(value: unknown): value is RefSource {
  return typeof value === 'object' && value !== null && (value as { type?: unknown }).type === 'ref'
}

function isRefBase64Source(value: unknown): value is RefBase64Source {
  return typeof value === 'object' && value !== null && typeof (value as { ref?: unknown }).ref === 'string'
}

/** Returns a copy of the block with every inline media source moved into `blobs`. */
export async function externalizeBlockMedia(block: Block, blobs: BlobStore): Promise<Block> {
  if (block.type === 'user' || block.type === 'steer') {
    return { ...block, content: await Promise.all(block.content.map((item) => externalizeUserContent(item, blobs))) }
  }
  if (block.type === 'tool_call') {
    return {
      ...block,
      output: await Promise.all(block.output.map((item) => externalizeToolResult(item, blobs))),
      streamingOutput: await Promise.all(block.streamingOutput.map((item) => externalizeToolResult(item, blobs))),
    }
  }
  return block
}

/** Inverse of `externalizeBlockMedia`; missing blobs degrade to text placeholders. */
export async function rehydrateBlockMedia(block: Block, blobs: BlobStore): Promise<Block> {
  if (block.type === 'user' || block.type === 'steer') {
    return { ...block, content: await Promise.all(block.content.map((item) => rehydrateUserContent(item, blobs))) }
  }
  if (block.type === 'tool_call') {
    return {
      ...block,
      output: await Promise.all(block.output.map((item) => rehydrateToolResult(item, blobs))),
      streamingOutput: await Promise.all(block.streamingOutput.map((item) => rehydrateToolResult(item, blobs))),
    }
  }
  return block
}

async function externalizeUserContent(item: UserContentBlock, blobs: BlobStore): Promise<UserContentBlock> {
  if ((item.type === 'image' || item.type === 'video') && item.source.type === 'binary') {
    const ref = await blobs.put(item.source.data)
    const source: RefSource = { type: 'ref', ref, mediaType: item.source.mediaType }
    return { ...item, source: source as never }
  }
  if (item.type === 'document' && item.source.data instanceof Uint8Array) {
    const ref = await blobs.put(item.source.data)
    const source: RefSource = { type: 'ref', ref, mediaType: item.source.mediaType, fileName: item.source.fileName }
    return { ...item, source: source as never }
  }
  return item
}

async function rehydrateUserContent(item: UserContentBlock, blobs: BlobStore): Promise<UserContentBlock> {
  if (item.type !== 'image' && item.type !== 'video' && item.type !== 'document') return item
  const source: unknown = item.source
  if (!isRefSource(source)) return item
  const data = await blobs.get(source.ref)
  if (data === null) return missingMediaPlaceholder(item.type, source.ref)
  if (item.type === 'document') {
    return { ...item, source: { data, mediaType: source.mediaType, fileName: source.fileName ?? 'document' } }
  }
  return { ...item, source: { type: 'binary', data, mediaType: source.mediaType } as never }
}

async function externalizeToolResult(item: ToolResultContentBlock, blobs: BlobStore): Promise<ToolResultContentBlock> {
  if (item.type !== 'image' && item.type !== 'video') return item
  if (typeof item.source.data !== 'string') return item
  const ref = await blobs.put(base64ToBytes(item.source.data))
  const source: RefBase64Source = { ref, mediaType: item.source.mediaType }
  return { ...item, source: source as never }
}

async function rehydrateToolResult(item: ToolResultContentBlock, blobs: BlobStore): Promise<ToolResultContentBlock> {
  if (item.type !== 'image' && item.type !== 'video') return item
  const source: unknown = item.source
  if (typeof (source as { data?: unknown }).data === 'string') return item
  if (!isRefBase64Source(source)) return item
  const data = await blobs.get(source.ref)
  if (data === null) return { type: 'text', text: `[missing ${item.type} blob ${source.ref}]` }
  return { ...item, source: { mediaType: source.mediaType, data: bytesToBase64(data) } }
}

function missingMediaPlaceholder(kind: 'image' | 'video' | 'document', ref: string): UserContentBlock {
  return { type: 'text', text: `[missing ${kind} blob ${ref}]` }
}
